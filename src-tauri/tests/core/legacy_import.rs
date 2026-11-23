//! Legacy import (F21, D4, D28): the golden test over the committed legacy folders
//! (`fixtures/legacy-import/`), scanned and imported into a temp database through the command
//! handlers against the fake backend; failures; the offer at session start (F2).

use std::path::{Path, PathBuf};

use domain::pricing::{CustomPrice, PriceKey};
use exilence_ce_ng_lib::legacy_import::folders::{AppDataEnv, Os, legacy_folders};
use exilence_ce_ng_lib::session_start;
use exilence_ce_ng_lib::state::Store;
use ipc::command::{
    NoArgs, RunLegacyImport, RunLegacyImportArgs, ScanLegacyImport, SkipLegacyImport,
};
use ipc::slices::{
    Currency, ExitAction, LegacyAccount, LegacyImport, LegacyImportOffer, LegacyProfile, NetWorth,
    NotificationKind, Notifications, Profiles, Settings, Status, TimeSpan, Toast,
};
use ipc::{IpcError, Message, MessageKey, TOAST_EVENT};
use tauri::Manager;
use tauri::async_runtime::block_on;
use tonic::{Code, Status as GrpcStatus};
use wire::v1::{ErrorReason, Profile};

use super::fake_backend::{FakeBackend, ProfileCall, refused, session, signed_in};
use super::profiles::{ggg, ninja};
use super::{Harness, NO_BACKEND, next_toast, poll};

/// The Profile both legacy folders hold (`Foo`).
const BOTH_FOLDERS: &str = "e18a3763-0001-4000-8000-aaaaed000000";
/// `Foo#1234`'s Profile with an empty name.
const UNNAMED: &str = "de61fc39-0001-4000-8000-ceaaaf000000";
/// `Foo#1234`'s active Profile, holding the newest Snapshots.
const SETTLERS: &str = "de61fc3a-0002-4000-8000-ceaaaf000000";

fn fixture_set(set: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../fixtures/legacy-import")
        .join(set)
}

/// A fixture set laid out like an `appData` directory, resolved as on Linux.
fn folders_in(app_data: &Path) -> Vec<exilence_ce_ng_lib::legacy_import::folders::LegacyFolder> {
    legacy_folders(
        Os::Linux,
        &AppDataEnv {
            xdg_config_home: Some(app_data.into()),
            ..AppDataEnv::default()
        },
    )
}

impl Harness {
    /// Signed in as `Foo#1234` with the backend at `backend_url`, scanning the legacy folders
    /// under `app_data`.
    fn for_legacy_import(backend_url: &str, app_data: &Path) -> Self {
        Self::with_login()
            .backend(backend_url)
            .legacy_folders(folders_in(app_data))
            .build()
    }

    fn legacy_import(&self) -> LegacyImport {
        self.app.state::<Store>().read(LegacyImport::clone)
    }

    fn scan_legacy(&self) -> Result<(), IpcError> {
        self.invoke::<ScanLegacyImport>(&NoArgs {})
    }

    fn import_legacy(&self, names: &[&str]) -> Result<(), IpcError> {
        self.invoke::<RunLegacyImport>(&RunLegacyImportArgs {
            account_names: names.iter().map(|n| (*n).to_owned()).collect(),
        })
    }

    fn count(&self, sql: &str) -> i64 {
        block_on(sqlx::query_scalar(sqlx::AssertSqlSafe(sql)).fetch_one(&self.pool())).unwrap()
    }

    /// Nothing of the import is stored: no Profile, Snapshot, custom price, imported setting or
    /// marker.
    fn assert_nothing_imported(&self) {
        for table in [
            "profiles",
            "snapshots",
            "snapshot_tabs",
            "custom_prices",
            "legacy_import",
        ] {
            assert_eq!(
                self.count(&format!("SELECT count(*) FROM {table}")),
                0,
                "{table}"
            );
        }
        assert_eq!(
            self.count("SELECT count(*) FROM settings WHERE key IN ('currency', 'ui_scale')"),
            0
        );
    }
}

fn import_failed(description: MessageKey) -> Toast {
    Toast {
        kind: NotificationKind::Error,
        message: MessageKey::NotificationLegacyImportFailed.into(),
        description: Some(description.into()),
    }
}

fn wire_profile(
    id: &str,
    name: &str,
    league: &str,
    tabs: &[&str],
    character: Option<&str>,
    (equipment, inventory): (bool, bool),
) -> Profile {
    Profile {
        id: id.to_owned(),
        name: name.to_owned(),
        league_id: league.to_owned(),
        price_league_id: league.to_owned(),
        stash_tab_ids: tabs.iter().map(|t| (*t).to_owned()).collect(),
        character_name: character.map(str::to_owned),
        include_equipment: equipment,
        include_inventory: inventory,
    }
}

/// The Profiles `both/` brings into an account without one, in legacy order: `Foo`'s merged
/// Profile with the fields of `exilence-ce-app`, whose Snapshots are newer, then `Foo#1234`'s.
fn both_profile_calls() -> Vec<ProfileCall> {
    vec![
        ProfileCall::Create(wire_profile(
            BOTH_FOLDERS,
            "Standard main (CE)",
            "Standard",
            &["a1b2c3d4e5"],
            None,
            (false, false),
        )),
        ProfileCall::Create(wire_profile(
            UNNAMED,
            "profile 2",
            "Settlers",
            &[],
            None,
            (false, false),
        )),
        ProfileCall::Create(wire_profile(
            SETTLERS,
            "Settlers",
            "Settlers",
            &["0a1b2c3d4e", "5f6a7b8c9d"],
            Some("FooChar"),
            (false, true),
        )),
        ProfileCall::SetActive(SETTLERS.to_owned()),
    ]
}

#[test]
fn f21_golden_import_of_every_fixture_maps_the_rows_and_a_second_run_adds_nothing() {
    let backend = FakeBackend::start(signed_in());
    let harness = Harness::for_legacy_import(&backend.url, &fixture_set("both"));
    let pool = harness.pool();
    // Values already set in NG win (per device).
    block_on(async {
        sqlx::query("INSERT INTO settings (key, value) VALUES ('ui_scale', '120')")
            .execute(&pool)
            .await?;
        sqlx::query(
            "INSERT INTO custom_prices (price_league_id, name, icon, frame_type, value) \
             VALUES ('Settlers', 'Divine Orb', 'https://web.poecdn.com/divine.png', 5, 99)",
        )
        .execute(&pool)
        .await
    })
    .unwrap();

    harness.scan_legacy().unwrap();

    assert_eq!(
        harness.legacy_import(),
        LegacyImport {
            offer: Some(LegacyImportOffer {
                accounts: vec![
                    LegacyAccount {
                        name: "Foo".to_owned(),
                        preselected: true,
                        profiles: vec![LegacyProfile {
                            name: "Standard main (CE)".to_owned(),
                            snapshot_count: 22,
                            first_snapshot_at: Some(1_693_526_400_000),
                            last_snapshot_at: Some(1_696_122_000_000),
                        }],
                    },
                    LegacyAccount {
                        name: "Foo#1234".to_owned(),
                        preselected: true,
                        profiles: vec![
                            LegacyProfile {
                                name: String::new(),
                                snapshot_count: 0,
                                first_snapshot_at: None,
                                last_snapshot_at: None,
                            },
                            LegacyProfile {
                                name: "Settlers".to_owned(),
                                snapshot_count: 18,
                                first_snapshot_at: Some(1_722_470_400_000),
                                last_snapshot_at: Some(1_722_531_600_000),
                            },
                        ],
                    },
                ],
            }),
            running: false,
        }
    );

    harness.import_legacy(&["Foo", "Foo#1234"]).unwrap();

    assert_eq!(backend.profile_calls(), both_profile_calls());
    assert!(backend.uploads().is_empty(), "D4: never uploaded");
    type ProfileRow = (
        String,
        i64,
        String,
        String,
        String,
        String,
        Option<String>,
        bool,
        bool,
    );
    let profiles: Vec<ProfileRow> = block_on(
        sqlx::query_as(
            "SELECT id, position, name, league_id, price_league_id, stash_tab_ids, \
             character_name, include_equipment, include_inventory FROM profiles ORDER BY position",
        )
        .fetch_all(&pool),
    )
    .unwrap();
    let row = |id: &str,
               position,
               name: &str,
               league: &str,
               tabs: &str,
               character: Option<&str>,
               inventory| {
        (
            id.to_owned(),
            position,
            name.to_owned(),
            league.to_owned(),
            league.to_owned(),
            tabs.to_owned(),
            character.map(str::to_owned),
            false,
            inventory,
        )
    };
    assert_eq!(
        profiles,
        [
            row(
                BOTH_FOLDERS,
                0,
                "Standard main (CE)",
                "Standard",
                r#"["a1b2c3d4e5"]"#,
                None,
                false
            ),
            row(UNNAMED, 1, "profile 2", "Settlers", "[]", None, false),
            row(
                SETTLERS,
                2,
                "Settlers",
                "Settlers",
                r#"["0a1b2c3d4e","5f6a7b8c9d"]"#,
                Some("FooChar"),
                true
            ),
        ]
    );
    assert_eq!(
        harness.count("SELECT count(*) FROM accounts WHERE active_profile_id = 'de61fc3a-0002-4000-8000-ceaaaf000000'"),
        1
    );
    // Snapshots: both folders unioned (20 + 2) and Foo#1234's 18, totals only.
    let per_profile: Vec<(String, i64)> = block_on(
        sqlx::query_as(
            "SELECT profile_id, count(*) FROM snapshots GROUP BY profile_id ORDER BY profile_id",
        )
        .fetch_all(&pool),
    )
    .unwrap();
    assert_eq!(
        per_profile,
        [(SETTLERS.to_owned(), 18), (BOTH_FOLDERS.to_owned(), 22)]
    );
    assert_eq!(
        harness.count("SELECT count(*) FROM snapshots WHERE body IS NULL AND imported = 1"),
        40
    );
    assert_eq!(
        harness.count(
            "SELECT count(*) FROM snapshots s WHERE abs(total_value - \
             (SELECT sum(value) FROM snapshot_tabs t WHERE t.snapshot_id = s.id)) > 1e-9"
        ),
        0,
        "the total is the sum of the tabs, Character included"
    );
    type TabRow = (i64, String, String, f64);
    let tabs = |id: &str| -> (i64, f64, Vec<TabRow>) {
        block_on(async {
            let (created_at, total): (i64, f64) =
                sqlx::query_as("SELECT created_at, total_value FROM snapshots WHERE id = ?1")
                    .bind(id)
                    .fetch_one(&pool)
                    .await?;
            let tabs = sqlx::query_as(
                "SELECT position, tab_id, name, value FROM snapshot_tabs \
                 WHERE snapshot_id = ?1 ORDER BY position",
            )
            .bind(id)
            .fetch_all(&pool)
            .await?;
            Ok::<_, sqlx::Error>((created_at, total, tabs))
        })
        .unwrap()
    };
    let tab =
        |position, id: &str, name: &str, value| (position, id.to_owned(), name.to_owned(), value);
    // The newest Snapshot of exilence-ce-app, with items.
    assert_eq!(
        tabs("cd01bbdb-0000-4000-8000-ceaaaaeda000"),
        (
            1_696_122_000_000,
            2028.0,
            vec![
                tab(0, "a1b2c3d4e5", "Currency", 1007.25),
                tab(1, "", "Character", 1020.75)
            ]
        )
    );
    // The oldest of exilence-next-app, without items: names from newer Snapshots.
    assert_eq!(
        tabs("62d30909-0013-4000-8000-aeaaa0000000"),
        (
            1_693_526_400_000,
            3040.5,
            vec![
                tab(0, "a1b2c3d4e5", "Currency", 1000.0),
                tab(1, "f6a7b8c9d0", "Maps", 1013.5),
                tab(2, "", "Character", 1027.0),
            ]
        )
    );
    // A UTF-16 tab name.
    assert_eq!(
        tabs("20ffd0b6-0011-4000-8000-cea000000000").2,
        vec![
            tab(0, "0a1b2c3d4e", "Währung ✓ 通貨", 1000.0),
            tab(1, "5f6a7b8c9d", "Dump", 1013.5),
            tab(2, "", "Character", 1027.0),
        ]
    );
    // Custom prices, as a Snapshot prices with them: the existing one wins; 0 and missing are
    // skipped; absent fields stay absent.
    let mut prices =
        block_on(exilence_ce_ng_lib::prices::custom_prices(&pool, "Settlers")).unwrap();
    prices.sort_by(|a, b| a.key.name.cmp(&b.key.name));
    assert_eq!(
        prices,
        [
            CustomPrice {
                key: PriceKey {
                    name: "Divine Orb".to_owned(),
                    icon: "https://web.poecdn.com/divine.png".to_owned(),
                    frame_type: Some(5),
                    ..PriceKey::default()
                },
                value: 99.0,
            },
            CustomPrice {
                key: PriceKey {
                    name: "Headhunter".to_owned(),
                    icon: "https://web.poecdn.com/hh.png".to_owned(),
                    frame_type: Some(3),
                    variant: Some(String::new()),
                    links: Some(0),
                    quality: Some(20),
                    level: Some(0),
                    ilvl: Some(84),
                    tier: Some(0),
                    corrupted: Some(true),
                    elder: Some(false),
                    shaper: Some(false),
                },
                value: 1500.0,
            },
        ]
    );
    // Settings: exilence-ce-app's, its local-settings.json for the exit action; NG's UI scale.
    let settings = harness.app.state::<Store>().read(Settings::clone);
    assert_eq!(
        settings,
        Settings {
            auto_snapshot: true,
            auto_snapshot_minutes: 5,
            low_confidence_pricing: false,
            price_threshold: 0.5,
            total_price_threshold: 0.0,
            currency: Currency::Exalted,
            chart_time_span: TimeSpan::Week,
            net_worth_chart_open: true,
            tab_chart_open: false,
            items_open: true,
            exit_action: ExitAction::Exit,
            nav_drawer_open: true,
            ui_scale: 120,
            ..Settings::default()
        }
    );
    assert_eq!(
        harness.count("SELECT count(*) FROM legacy_import WHERE state = 'done'"),
        1
    );
    // The slices show it at once.
    let slice = harness.app.state::<Store>().read(Profiles::clone);
    assert_eq!(slice.profiles.len(), 3);
    assert_eq!(slice.active_profile_id.as_deref(), Some(SETTLERS));
    let net_worth = harness.app.state::<Store>().read(NetWorth::clone);
    assert_eq!(net_worth.snapshot_count, 18);
    assert_eq!(net_worth.total, 1123.25 + 1136.75 + 1150.25);
    assert_eq!(harness.legacy_import(), LegacyImport::default());
    assert_eq!(
        harness
            .app
            .state::<Store>()
            .read(|s: &Status| s.message.clone()),
        None
    );
    let notifications = harness.app.state::<Store>().read(Notifications::clone);
    assert_eq!(notifications.newest[0].kind, NotificationKind::Success);
    assert_eq!(
        notifications.newest[0].message,
        Message::new(MessageKey::NotificationLegacyImportSucceeded)
            .arg("profiles", 3u32)
            .arg("snapshots", 40u32)
    );

    // Settings → Advanced runs it again, whatever the marker says: nothing twice.
    harness.scan_legacy().unwrap();
    assert!(harness.legacy_import().offer.is_some());
    harness.import_legacy(&["Foo", "Foo#1234"]).unwrap();
    assert_eq!(backend.profile_calls(), both_profile_calls(), "no call");
    assert_eq!(harness.count("SELECT count(*) FROM profiles"), 3);
    assert_eq!(harness.count("SELECT count(*) FROM snapshots"), 40);
    // 2 CE Snapshots × 2 tabs, 20 Next × 3, 18 Foo#1234 × 3.
    assert_eq!(
        harness.count("SELECT count(*) FROM snapshot_tabs"),
        2 * 2 + 20 * 3 + 18 * 3
    );
    assert_eq!(harness.count("SELECT count(*) FROM custom_prices"), 2);
    let notifications = harness.app.state::<Store>().read(Notifications::clone);
    assert_eq!(
        notifications.newest[0].message,
        Message::new(MessageKey::NotificationLegacyImportSucceeded)
            .arg("profiles", 0u32)
            .arg("snapshots", 0u32)
    );
    assert!(backend.uploads().is_empty(), "D4: never uploaded");

    // Data older than `migration.current` 3 is not offered.
    let old = Harness::for_legacy_import(&backend.url, &fixture_set("old-migration"));
    let toasts = old.events(TOAST_EVENT);
    old.scan_legacy().unwrap();
    assert_eq!(
        next_toast(&toasts),
        Toast {
            kind: NotificationKind::Info,
            message: MessageKey::LegacyImportNoData.into(),
            description: None,
        }
    );
    assert_eq!(old.legacy_import(), LegacyImport::default());
    old.assert_nothing_imported();
}

#[test]
fn f21_a_failure_at_any_step_writes_nothing_and_shows_a_toast() {
    // Unreadable legacy data (decode error): the offer stays open.
    let backend = FakeBackend::start(signed_in());
    let app_data = tempfile::tempdir().unwrap();
    copy_dir(&fixture_set("both"), app_data.path());
    let harness = Harness::for_legacy_import(&backend.url, app_data.path());
    let toasts = harness.events(TOAST_EVENT);
    harness.scan_legacy().unwrap();
    std::fs::write(
        app_data.path().join("exilence-ce-app/local-settings.json"),
        "{",
    )
    .unwrap();
    assert!(harness.import_legacy(&["Foo"]).is_err());
    assert_eq!(
        next_toast(&toasts),
        import_failed(MessageKey::ErrorLegacyUnreadable)
    );
    assert!(backend.profile_calls().is_empty());
    harness.assert_nothing_imported();
    assert!(
        harness.legacy_import().offer.is_some(),
        "the offer stays open"
    );
    assert!(!harness.legacy_import().running);

    // The backend unreachable.
    let harness = Harness::for_legacy_import(NO_BACKEND, &fixture_set("both"));
    let toasts = harness.events(TOAST_EVENT);
    harness.scan_legacy().unwrap();
    assert_eq!(harness.import_legacy(&["Foo"]), Err(IpcError::Offline));
    assert_eq!(next_toast(&toasts), import_failed(MessageKey::ErrorOffline));
    harness.assert_nothing_imported();
    assert!(harness.legacy_import().offer.is_some());

    // SUPERSEDED after a Profile was created: the next StartSession brings it, a retry adds
    // its Snapshots.
    let backend = FakeBackend::start(signed_in());
    backend.script_profile_failures([
        GrpcStatus::new(Code::Ok, ""),
        refused(Code::Unauthenticated, ErrorReason::Superseded),
    ]);
    let harness = Harness::for_legacy_import(&backend.url, &fixture_set("both"));
    let toasts = harness.events(TOAST_EVENT);
    harness.scan_legacy().unwrap();
    assert_eq!(
        harness.import_legacy(&["Foo", "Foo#1234"]),
        Err(IpcError::Superseded)
    );
    assert_eq!(
        next_toast(&toasts),
        import_failed(MessageKey::ErrorSuperseded)
    );
    assert_eq!(backend.profile_calls(), both_profile_calls()[..2]);
    harness.assert_nothing_imported();
    assert!(harness.legacy_import().offer.is_some());

    // SESSION_EXPIRED when setting the active Profile: back to login with its toast.
    let backend = FakeBackend::start(signed_in());
    backend.script_profile_failures([
        GrpcStatus::new(Code::Ok, ""),
        GrpcStatus::new(Code::Ok, ""),
        GrpcStatus::new(Code::Ok, ""),
        refused(Code::Unauthenticated, ErrorReason::SessionExpired),
    ]);
    let harness = Harness::for_legacy_import(&backend.url, &fixture_set("both"));
    let toasts = harness.events(TOAST_EVENT);
    harness.scan_legacy().unwrap();
    assert_eq!(
        harness.import_legacy(&["Foo", "Foo#1234"]),
        Err(IpcError::SessionExpired)
    );
    assert_eq!(
        next_toast(&toasts).message,
        MessageKey::ErrorSessionExpired.into()
    );
    assert_eq!(backend.profile_calls(), both_profile_calls());
    harness.assert_nothing_imported();
    assert!(backend.uploads().is_empty());
}

#[test]
fn f21_a_retry_after_a_failed_run_imports_everything_once() {
    let backend = FakeBackend::start(signed_in());
    backend.script_profile_failures([
        GrpcStatus::new(Code::Ok, ""),
        GrpcStatus::unavailable("backend down"),
    ]);
    let harness = Harness::for_legacy_import(&backend.url, &fixture_set("both"));
    harness.scan_legacy().unwrap();
    assert_eq!(
        harness.import_legacy(&["Foo", "Foo#1234"]),
        Err(IpcError::Offline)
    );
    harness.assert_nothing_imported();

    harness.import_legacy(&["Foo", "Foo#1234"]).unwrap();

    // The first Profile again: CreateProfile is idempotent on its id.
    let calls = backend.profile_calls();
    assert_eq!(calls[..2], both_profile_calls()[..2]);
    assert_eq!(calls[2..], both_profile_calls());
    assert_eq!(harness.count("SELECT count(*) FROM profiles"), 3);
    assert_eq!(harness.count("SELECT count(*) FROM snapshots"), 40);
    assert!(backend.uploads().is_empty(), "D4: never uploaded");
}

#[test]
fn f21_f2_the_first_session_start_offers_the_import_and_skip_creates_profile_1() {
    let backend = FakeBackend::start(signed_in());
    backend.script_start_sessions([session(Vec::new(), "")]);
    let (ggg, ninja) = (ggg(), ninja());
    let harness = Harness::with_login()
        .backend(&backend.url)
        .ggg(&ggg.uri())
        .ninja(&ninja.uri())
        .legacy_folders(folders_in(&fixture_set("both")))
        .build();

    block_on(session_start::run(harness.app.handle()));

    let offer = poll(|| harness.legacy_import().offer);
    let preselected: Vec<(String, bool)> = offer
        .accounts
        .iter()
        .map(|a| (a.name.clone(), a.preselected))
        .collect();
    assert_eq!(
        preselected,
        [("Foo".to_owned(), true), ("Foo#1234".to_owned(), true)]
    );
    // `profile 1` and the session's GGG part wait for the answer.
    harness.wait_for_characters();
    assert!(backend.profile_calls().is_empty());

    harness.invoke::<SkipLegacyImport>(&NoArgs {}).unwrap();
    harness.wait_for_ready();

    let profiles = poll(|| {
        Some(harness.app.state::<Store>().read(Profiles::clone)).filter(|p| !p.profiles.is_empty())
    });
    assert_eq!(profiles.profiles[0].name, "profile 1");
    assert_eq!(harness.legacy_import(), LegacyImport::default());
    assert_eq!(
        harness.count("SELECT count(*) FROM legacy_import WHERE state = 'declined'"),
        1
    );
    assert_eq!(harness.count("SELECT count(*) FROM snapshots"), 0);

    // Skipped: never offered again on this device.
    let harness = harness.restart();
    // The session start offers it in its backend part, which `run` awaits.
    block_on(session_start::run(harness.app.handle()));
    assert_eq!(harness.legacy_import(), LegacyImport::default());
}

#[test]
fn f21_f2_import_brings_the_legacy_profiles_instead_of_profile_1() {
    let backend = FakeBackend::start(signed_in());
    backend.script_start_sessions([session(Vec::new(), "")]);
    let (ggg, ninja) = (ggg(), ninja());
    let harness = Harness::with_login()
        .backend(&backend.url)
        .ggg(&ggg.uri())
        .ninja(&ninja.uri())
        .legacy_folders(folders_in(&fixture_set("both")))
        .build();
    block_on(session_start::run(harness.app.handle()));
    poll(|| harness.legacy_import().offer);

    harness.import_legacy(&["Foo#1234"]).unwrap();
    harness.wait_for_ready();

    let calls = backend.profile_calls();
    assert_eq!(calls.len(), 3, "{calls:?}");
    let ProfileCall::Create(unnamed) = &calls[0] else {
        panic!("{calls:?}");
    };
    assert_eq!(
        (unnamed.id.as_str(), unnamed.name.as_str()),
        (UNNAMED, "profile 1")
    );
    assert_eq!(calls[2], ProfileCall::SetActive(SETTLERS.to_owned()));
    assert_eq!(harness.count("SELECT count(*) FROM snapshots"), 18);
    assert!(backend.uploads().is_empty(), "D4: never uploaded");
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}
