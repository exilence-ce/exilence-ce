//! Taking a Snapshot (F3), retention (F12) and the upload outside a Group
//! (D26), against a `wiremock` GGG and poe.ninja and the fake backend.

use std::time::Duration;

use exilence_ce_ng_lib::prices::PriceTables;
use exilence_ce_ng_lib::slot::{self, OperationSlot};
use exilence_ce_ng_lib::snapshot::{BODIES_KEPT, SNAPSHOTS_KEPT};
use exilence_ce_ng_lib::stash_tabs::StashTabs;
use exilence_ce_ng_lib::state::Store;
use exilence_ce_ng_lib::{profiles, session_start};
use ipc::command::{
    CancelSnapshot, NoArgs, PriceKey, ResetIncome, SetCustomPrice, SetCustomPriceArgs, TakeSnapshot,
};
use ipc::slices::{
    Charts, NetWorth, NotificationKind, Notifications, Point, PriceAvailability, Prices, Session,
    Status,
};
use ipc::{Event, IpcError, MessageKey, TOAST_EVENT};
use prost::Message as _;
use tauri::Manager;
use tauri::async_runtime::block_on;
use tonic::Status as GrpcStatus;
use wire::v1::{SnapshotBody, TabKind};
use wiremock::matchers::{any, header, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::fake_backend::{ACCESS_TOKEN, ACCOUNT_ID, FakeBackend, session, signed_in};
use super::fake_ggg::START;
use super::prices::ninja_with_prices;
use super::{Harness, NO_BACKEND, mock_server, next_toast, poll, received};

pub(super) const PROFILE_ID: &str = "01a1cd36-0000-7000-8000-0000000000aa";
pub(super) const LEAGUE: &str = "Settlers";

/// The Profile: `t2` (a folder's tab with child tabs) and `t1` selected in
/// that order, `Hero`'s inventory without equipment.
pub(super) fn profile() -> wire::v1::Profile {
    wire::v1::Profile {
        id: PROFILE_ID.to_owned(),
        name: "profile 1".to_owned(),
        league_id: LEAGUE.to_owned(),
        price_league_id: LEAGUE.to_owned(),
        stash_tab_ids: vec!["t2".to_owned(), "t1".to_owned()],
        character_name: Some("Hero".to_owned()),
        include_equipment: false,
        include_inventory: true,
    }
}

pub(super) fn currency(type_line: &str, stack_size: u32) -> serde_json::Value {
    serde_json::json!({
        "id": format!("{type_line}-{stack_size}"),
        "typeLine": type_line,
        "baseType": type_line,
        "frameType": 5,
        "icon": format!("https://web.poecdn.com/{type_line}.png"),
        "stackSize": stack_size,
    })
}

pub(super) fn ok(body: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(body)
}

/// GGG for [`ACCESS_TOKEN`]: the characters, `Settlers`' stash tab list, the
/// tabs, `t2`'s child tab and `Hero`. `t1` answers after `t1_delay`.
pub(super) fn ggg_with_stash(t1_delay: Duration) -> MockServer {
    block_on(async {
        let server = mock_server().await;
        let answers = [
            (
                "/character".to_owned(),
                ok(serde_json::json!({ "characters": [
                    { "id": "c1", "name": "Hero", "class": "Witch", "league": LEAGUE, "level": 90 },
                ]})),
            ),
            (
                format!("/stash/{LEAGUE}"),
                ok(serde_json::json!({ "stashes": [
                    { "id": "t1", "name": "Currency", "index": 0, "metadata": { "colour": "7c5436" } },
                    { "id": "f1", "name": "Folder", "index": 1, "type": "Folder", "children": [
                        { "id": "t2", "name": "Maps", "index": 2, "parent": "f1" },
                    ]},
                    { "id": "t3", "name": "Not selected", "index": 3 },
                ]})),
            ),
            (
                format!("/stash/{LEAGUE}/t1"),
                ok(serde_json::json!({ "stash": {
                    "id": "t1", "name": "Currency",
                    "items": [currency("Divine Orb", 2), currency("Chaos Orb", 3), currency("Chaos Orb", 4)],
                }}))
                .set_delay(t1_delay),
            ),
            (
                format!("/stash/{LEAGUE}/t2"),
                ok(serde_json::json!({ "stash": {
                    "id": "t2", "name": "Maps",
                    "children": [{ "id": "c1", "name": "Tier 16", "parent": "t2" }],
                }})),
            ),
            (
                format!("/stash/{LEAGUE}/t2/c1"),
                ok(serde_json::json!({ "stash": {
                    "id": "c1", "parent": "t2", "items": [currency("Chaos Orb", 10)],
                }})),
            ),
            (
                "/character/Hero".to_owned(),
                ok(serde_json::json!({ "character": {
                    "name": "Hero",
                    "inventory": [currency("Exalted Orb", 1)],
                    "equipment": [currency("Chaos Orb", 50)],
                }})),
            ),
        ];
        for (url, answer) in answers {
            Mock::given(path(url))
                .and(header(
                    "authorization",
                    format!("Bearer {ACCESS_TOKEN}").as_str(),
                ))
                .respond_with(answer)
                .with_priority(2)
                .mount(&server)
                .await;
        }
        Mock::given(any())
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        server
    })
}

pub(super) struct Setup {
    pub(super) harness: Harness,
    pub(super) ggg: MockServer,
    pub(super) ninja: MockServer,
}

impl Setup {
    /// Signed in with the backend at `backend_url`, the session started and
    /// ready for a Snapshot of [`profile`].
    #[track_caller]
    pub(super) fn ready(backend: Option<&FakeBackend>, ggg: MockServer) -> Self {
        Self::ready_with(backend, ggg, |_| {})
    }

    /// As [`Setup::ready`], with `before_start` run on the signed-in app
    /// just before the session starts.
    #[track_caller]
    pub(super) fn ready_with(
        backend: Option<&FakeBackend>,
        ggg: MockServer,
        before_start: impl FnOnce(&Harness),
    ) -> Self {
        let ninja = ninja_with_prices();
        let harness = Harness::with_login()
            .backend(backend.map_or(NO_BACKEND, |b| b.url.as_str()))
            .ggg(&ggg.uri())
            .ninja(&ninja.uri())
            .profiles(&[profile()], PROFILE_ID)
            .build();
        if let Some(backend) = backend {
            backend.script_start_sessions([session(vec![profile()], PROFILE_ID)]);
        }
        before_start(&harness);
        block_on(session_start::run(harness.app.handle()));
        let setup = Self {
            harness,
            ggg,
            ninja,
        };
        poll(|| {
            let session = setup.read(Session::clone);
            let prices = setup.read(Prices::clone);
            (session.ready
                && session.league_has_tabs
                && prices.availability == PriceAvailability::Available)
                .then_some(())
        });
        setup
    }

    /// Quits cleanly and starts again on the same database, without
    /// starting the session.
    pub(super) fn restart(self) -> Self {
        let Self {
            harness,
            ggg,
            ninja,
        } = self;
        Self {
            harness: harness.restart(),
            ggg,
            ninja,
        }
    }
    pub(super) fn read<S: ipc::Slice, T>(&self, read: impl FnOnce(&S) -> T) -> T {
        self.harness.app.state::<Store>().read(read)
    }

    pub(super) fn take(&self) -> Result<(), IpcError> {
        self.harness.invoke::<TakeSnapshot>(&NoArgs {})
    }

    /// Takes a Snapshot and waits until it has ended.
    #[track_caller]
    pub(super) fn take_and_wait(&self) {
        self.take().unwrap();
        self.wait_until_ended();
    }

    /// Waits until the Snapshot has ended: not running, and the operation
    /// slot given back, which happens only after `snapshot_running` went
    /// out false.
    #[track_caller]
    pub(super) fn wait_until_ended(&self) {
        let slot = self.harness.app.state::<OperationSlot>();
        poll(|| {
            (!self.read(|s: &Status| s.snapshot_running) && slot.try_hold().is_ok()).then_some(())
        });
    }

    pub(super) fn snapshots(&self) -> Vec<(String, f64, bool)> {
        block_on(
            sqlx::query_as(
                "SELECT id, total_value, body IS NOT NULL FROM snapshots \
                 WHERE profile_id = ?1 ORDER BY created_at DESC, id DESC",
            )
            .bind(PROFILE_ID)
            .fetch_all(&self.harness.pool()),
        )
        .unwrap()
    }
}

#[test]
fn f3_d26_a_snapshot_counts_children_and_the_character_and_shows_its_total_without_the_backend() {
    let setup = Setup::ready(None, ggg_with_stash(Duration::ZERO));
    // A custom price at the Snapshot's start applies (F6).
    let (table, _) = setup
        .harness
        .app
        .state::<PriceTables>()
        .get(LEAGUE)
        .unwrap();
    let divine = table.iter().find(|p| p.key.name == "Divine Orb").unwrap();
    setup
        .harness
        .invoke::<SetCustomPrice>(&SetCustomPriceArgs {
            price_league: LEAGUE.to_owned(),
            key: PriceKey {
                name: divine.key.name.clone(),
                icon: divine.key.icon.clone(),
                frame_type: divine.key.frame_type,
                variant: divine.key.variant.clone(),
                links: divine.key.links,
                quality: divine.key.quality,
                level: divine.key.level,
                ilvl: divine.key.ilvl,
                tier: divine.key.tier,
                corrupted: divine.key.corrupted,
                elder: divine.key.elder,
                shaper: divine.key.shaper,
            },
            value: 200.0,
        })
        .unwrap();
    let statuses = setup.harness.events("status");
    let toasts = setup.harness.events(TOAST_EVENT);

    setup.take_and_wait();

    // The tab list was refreshed and each selected tab fetched once, in the
    // list's order; the child tab under its parent; the character.
    assert_eq!(
        received(&setup.ggg, "/stash/Settlers"),
        2,
        "session start + Snapshot"
    );
    for url in [
        "/stash/Settlers/t1",
        "/stash/Settlers/t2",
        "/stash/Settlers/t2/c1",
        "/character/Hero",
    ] {
        assert_eq!(received(&setup.ggg, url), 1, "{url}");
    }
    assert_eq!(
        received(&setup.ggg, "/stash/Settlers/t3"),
        0,
        "not selected"
    );
    let progress: Vec<(u32, u32)> = statuses
        .try_iter()
        .filter_map(|e| match e {
            Event::Status(s) => s.snapshot_progress.map(|p| (p.tab, p.count)),
            _ => None,
        })
        .collect();
    assert!(
        progress.contains(&(1, 2)) && progress.contains(&(2, 2)),
        "{progress:?}"
    );

    // Divine 2 × 200 + Chaos 7 × 1 in t1, Chaos 10 from t2's child tab,
    // Exalted 1 × 12 from the inventory; the equipment is not included.
    let tabs: Vec<(i64, String, String, f64)> = block_on(
        sqlx::query_as("SELECT position, tab_id, name, value FROM snapshot_tabs ORDER BY position")
            .fetch_all(&setup.harness.pool()),
    )
    .unwrap();
    assert_eq!(
        tabs,
        [
            (0, "t1".to_owned(), "Currency".to_owned(), 407.0),
            (1, "t2".to_owned(), "Maps".to_owned(), 10.0),
            (2, String::new(), "Character".to_owned(), 12.0),
        ]
    );
    let saved = setup.snapshots();
    assert_eq!(saved.len(), 1);
    assert_eq!((saved[0].1, saved[0].2), (429.0, true));

    let net_worth = setup.read(NetWorth::clone);
    assert_eq!(net_worth.total, 429.0);
    assert_eq!(net_worth.change, 0.0, "fewer than 2 Snapshots");
    assert_eq!(net_worth.snapshot_count, 1);
    assert_eq!(net_worth.last_snapshot_at, Some(START));
    // F10: the charts follow the new Snapshot; the Character is not charted.
    let charts = setup.read(Charts::clone);
    let point = Point {
        at: START,
        value: 429.0,
    };
    assert_eq!(charts.net_worth[0].points, [point]);
    let tabs: Vec<(&str, f64)> = charts
        .tabs
        .iter()
        .map(|s| (s.name.as_str(), s.points[0].value))
        .collect();
    assert_eq!(tabs, [("Maps", 10.0), ("Currency", 407.0)]);
    assert_eq!(
        setup.read(Status::clone),
        Status::default(),
        "status cleared"
    );

    // The body: merged rows per tab, one price per definition sorted by id.
    let body: Vec<u8> =
        block_on(sqlx::query_scalar("SELECT body FROM snapshots").fetch_one(&setup.harness.pool()))
            .unwrap();
    let body = SnapshotBody::decode(&*zstd::decode_all(&*body).unwrap()).unwrap();
    assert_eq!(body.price_league_id, LEAGUE);
    let mut sorted = body.priced_definition_ids.clone();
    sorted.sort_unstable();
    assert_eq!(body.priced_definition_ids, sorted);
    assert_eq!(body.prices.len(), 3, "Divine, Chaos, Exalted");
    assert_eq!(body.tabs[0].stack_sizes, [2, 7], "identical items merged");
    assert_eq!(body.tabs[0].colour, "7c5436");
    assert_eq!(body.tabs[2].kind, i32::from(TabKind::Character));
    let stored: i64 = block_on(
        sqlx::query_scalar("SELECT count(*) FROM item_definitions WHERE uploaded = 0")
            .fetch_one(&setup.harness.pool()),
    )
    .unwrap();
    assert_eq!(stored, 3);

    // D26: the unreachable backend shows nowhere; the success is list-only.
    // The upload ended before the Snapshot gave the operation slot back.
    assert!(toasts.try_recv().is_err(), "no toast");
    let notifications = setup.read(Notifications::clone);
    assert_eq!(
        notifications.newest[0].message.key,
        MessageKey::NotificationSnapshotSucceeded
    );
    assert_eq!(notifications.newest[0].kind, NotificationKind::Success);
    assert!(!setup.read(|s: &Session| s.offline));
}

#[test]
fn f3_start_conditions() {
    // Before the session is ready.
    let ninja = ninja_with_prices();
    let harness = Harness::for_prices(&ninja);
    assert_eq!(
        harness.invoke::<TakeSnapshot>(&NoArgs {}),
        Err(IpcError::NotReady)
    );

    // While the operation slot is taken.
    let setup = Setup::ready(None, ggg_with_stash(Duration::ZERO));
    let slot = setup.harness.app.state::<OperationSlot>();
    let held = block_on(slot.hold());
    assert_eq!(setup.take(), Err(IpcError::Busy));
    drop(held);

    // A league without tabs.
    let empty = block_on(async {
        let server = mock_server().await;
        Mock::given(path("/character"))
            .respond_with(ok(serde_json::json!({ "characters": [
                { "id": "c1", "name": "Hero", "class": "Witch", "league": LEAGUE },
            ]})))
            .mount(&server)
            .await;
        Mock::given(path("/stash/Settlers"))
            .respond_with(ok(serde_json::json!({ "stashes": [] })))
            .mount(&server)
            .await;
        server
    });
    let ninja = ninja_with_prices();
    let harness = Harness::with_login()
        .ggg(&empty.uri())
        .ninja(&ninja.uri())
        .profiles(&[profile()], PROFILE_ID)
        .build();
    block_on(session_start::run(harness.app.handle()));
    poll(|| {
        harness
            .app
            .state::<Store>()
            .read(|s: &Session| s.ready)
            .then_some(())
    });
    assert!(
        !harness
            .app
            .state::<Store>()
            .read(|s: &Session| s.league_has_tabs)
    );
    assert_eq!(
        harness.invoke::<TakeSnapshot>(&NoArgs {}),
        Err(IpcError::NotReady)
    );
}

#[test]
fn f3_f7_a_ggg_error_fails_the_snapshot_with_a_toast_and_a_cooldown_blocks_the_next() {
    let ggg = ggg_with_stash(Duration::ZERO);
    let setup = Setup::ready(None, ggg);
    block_on(async {
        Mock::given(path("/stash/Settlers/t1"))
            .respond_with(
                ResponseTemplate::new(429)
                    .insert_header("Retry-After", "60")
                    .insert_header("X-Rate-Limit-Policy", "stash-request-limit")
                    .insert_header("X-Rate-Limit-Rules", "Account")
                    .insert_header("X-Rate-Limit-Account", "30:300:60")
                    .insert_header("X-Rate-Limit-Account-State", "31:300:60"),
            )
            .with_priority(1)
            .mount(&setup.ggg)
            .await;
    });
    let toasts = setup.harness.events(TOAST_EVENT);

    setup.take_and_wait();

    let toast = next_toast(&toasts);
    assert_eq!(toast.message.key, MessageKey::NotificationSnapshotFailed);
    assert_eq!(
        toast.description.map(|d| d.key),
        Some(MessageKey::ErrorRateLimited)
    );
    assert!(setup.snapshots().is_empty(), "nothing saved");
    assert_eq!(setup.read(Status::clone), Status::default());
    assert!(matches!(setup.take(), Err(IpcError::RateLimited { .. })));
}

#[test]
fn f3_cancel_stops_the_snapshot_saves_nothing_clears_the_status_and_keeps_the_tab_list() {
    let setup = Setup::ready(None, ggg_with_stash(Duration::from_secs(3)));
    let started = std::time::Instant::now();
    setup.take().unwrap();
    poll(|| setup.read(|s: &Status| s.snapshot_progress));
    // The selected tabs as the refreshed list has them.
    poll(|| (received(&setup.ggg, "/stash/Settlers") == 2).then_some(()));

    setup.harness.invoke::<CancelSnapshot>(&NoArgs {}).unwrap();
    setup.wait_until_ended();

    assert!(
        started.elapsed() < Duration::from_secs(3),
        "the request was dropped"
    );
    assert!(setup.snapshots().is_empty());
    assert_eq!(setup.read(Status::clone), Status::default());
    assert_eq!(setup.read(|n: &NetWorth| n.snapshot_count), 0);
    assert!(setup.harness.app.state::<StashTabs>().get(LEAGUE).is_some());
    assert_eq!(received(&setup.ggg, "/character/Hero"), 0);

    // Reset data, Quit and an update install cancel it the same way.
    setup.take().unwrap();
    poll(|| setup.read(|s: &Status| s.snapshot_progress));
    let held = block_on(slot::cancel_and_hold(setup.harness.app.handle()));
    assert!(!setup.read(|s: &Status| s.snapshot_running));
    drop(held);
    assert!(setup.snapshots().is_empty());
}

#[test]
fn f12_retention_keeps_bodies_of_the_newest_10_and_at_most_1000_snapshots() {
    let setup = Setup::ready(None, ggg_with_stash(Duration::ZERO));
    let pool = setup.harness.pool();
    block_on(async {
        let mut tx = pool.begin().await.unwrap();
        for i in 0..SNAPSHOTS_KEPT {
            sqlx::query(
                "INSERT INTO snapshots (account_id, id, profile_id, created_at, total_value, body) \
                 VALUES (?1, ?2, ?3, ?4, 1.0, x'01')",
            )
            .bind(ACCOUNT_ID)
            .bind(format!("old-{i:04}"))
            .bind(PROFILE_ID)
            .bind(START - 1_000_000 + i)
            .execute(&mut *tx)
            .await
            .unwrap();
        }
        tx.commit().await.unwrap();
    });

    setup.take_and_wait();

    let saved = setup.snapshots();
    assert_eq!(saved.len() as i64, SNAPSHOTS_KEPT);
    assert_eq!(saved[0].1, 329.0, "the new one is the newest");
    assert!(
        !saved.iter().any(|(id, ..)| id == "old-0000"),
        "the oldest went"
    );
    let with_body: Vec<bool> = saved.iter().map(|(_, _, body)| *body).collect();
    assert!(with_body[..BODIES_KEPT as usize].iter().all(|b| *b));
    assert!(with_body[BODIES_KEPT as usize..].iter().all(|b| !*b));
    let net_worth = setup.read(NetWorth::clone);
    assert_eq!(net_worth.snapshot_count as i64, SNAPSHOTS_KEPT);
    assert_eq!(net_worth.change, 328.0, "against the previous newest");
}

#[test]
fn f12_the_startup_sweep_deletes_the_definitions_no_body_references() {
    let setup = Setup::ready(None, ggg_with_stash(Duration::ZERO));
    setup.take_and_wait();
    let pool = setup.harness.pool();
    let definitions = || -> Vec<i64> {
        block_on(sqlx::query_scalar("SELECT id FROM item_definitions ORDER BY id").fetch_all(&pool))
            .unwrap()
    };
    let referenced = definitions();
    assert_eq!(referenced.len(), 3, "the Snapshot's own");
    // Orphans: one of a Snapshot past retention, whose body is gone.
    block_on(async {
        for id in [1_i64, 2] {
            sqlx::query("INSERT INTO item_definitions (id, encoded) VALUES (?1, x'0a00')")
                .bind(id)
                .execute(&pool)
                .await
                .unwrap();
        }
    });
    assert_eq!(definitions().len(), 5);

    let setup = setup.restart();

    let mut expected = referenced;
    expected.sort_unstable();
    let pool = setup.harness.pool();
    let kept: Vec<i64> = block_on(
        sqlx::query_scalar("SELECT id FROM item_definitions ORDER BY id").fetch_all(&pool),
    )
    .unwrap();
    assert_eq!(kept, expected, "only what a body references stays");
    assert_eq!(setup.snapshots().len(), 1);
}

/// Stores a Snapshot of [`PROFILE_ID`] worth `total` taken at `created_at`.
pub(super) fn insert_snapshot(setup: &Setup, id: &str, created_at: i64, total: f64) {
    block_on(
        sqlx::query(
            "INSERT INTO snapshots (account_id, id, profile_id, created_at, total_value) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(ACCOUNT_ID)
        .bind(id)
        .bind(PROFILE_ID)
        .bind(created_at)
        .bind(total)
        .execute(&setup.harness.pool()),
    )
    .unwrap();
}

#[test]
fn f8_income_is_recalculated_after_each_snapshot_and_reset_starts_its_window() {
    const MINUTE: i64 = 60_000;
    let setup = Setup::ready(None, ggg_with_stash(Duration::ZERO));
    let clock = setup.harness.fakes.clock.clone();
    // Before this run: outside every income window.
    insert_snapshot(&setup, "before-start", START - 10 * MINUTE, 1.0);
    insert_snapshot(&setup, "after-start", START + MINUTE / 2, 100.0);

    clock.advance(MINUTE);
    setup.take_and_wait();
    let net_worth = setup.read(NetWorth::clone);
    assert_eq!(net_worth.total, 329.0);
    assert_eq!(net_worth.change, 229.0);
    assert_eq!(net_worth.sparkline, [1.0, 100.0, 329.0]);
    assert_eq!(net_worth.income, 229.0, "the two since the start");

    setup.harness.invoke::<ResetIncome>(&NoArgs {}).unwrap();
    assert_eq!(setup.read(|n: &NetWorth| n.income), 0.0);
    insert_snapshot(&setup, "after-reset", START + MINUTE + MINUTE / 2, 300.0);
    // A push without a Snapshot keeps the income.
    block_on(profiles::push(setup.harness.app.handle()));
    assert_eq!(setup.read(|n: &NetWorth| n.income), 0.0);

    clock.advance(MINUTE);
    setup.take_and_wait();
    let net_worth = setup.read(NetWorth::clone);
    assert_eq!(net_worth.income, 29.0, "only Snapshots after the reset");
    assert_eq!(net_worth.snapshot_count, 5);
}

#[test]
fn d26_a_failed_upload_is_invisible_solo_and_a_later_one_carries_the_new_definitions() {
    let backend = FakeBackend::start(signed_in());
    backend.script_upload_failures([GrpcStatus::internal("boom")]);
    let setup = Setup::ready(Some(&backend), ggg_with_stash(Duration::ZERO));
    let toasts = setup.harness.events(TOAST_EVENT);

    setup.take_and_wait();

    let uploads = backend.uploads();
    assert_eq!(uploads.len(), 1, "one attempt, no retry");
    assert_eq!(setup.snapshots().len(), 1, "saved anyway");
    assert_eq!(
        backend.calls.lock().unwrap().upload_timeouts,
        [Some("30000000u".to_owned())],
        "a 30 s deadline"
    );
    // The upload ended before the Snapshot gave the operation slot back.
    assert!(toasts.try_recv().is_err(), "no toast");
    assert!(!setup.read(|s: &Session| s.offline || s.superseded));

    setup.take_and_wait();
    setup.take_and_wait();

    let uploads = backend.uploads();
    assert_eq!(uploads.len(), 3);
    let second = &uploads[1];
    let snapshot = second.snapshot.as_ref().unwrap();
    let saved = setup.snapshots();
    assert_eq!(snapshot.id, saved[1].0);
    assert_eq!(snapshot.profile_id, PROFILE_ID);
    assert_eq!(snapshot.total_value, 329.0, "Divine Orb at 150c");
    let tab_ids: Vec<&str> = snapshot
        .tab_values
        .iter()
        .map(|t| t.tab_id.as_str())
        .collect();
    assert_eq!(tab_ids, ["t1", "t2", ""]);
    let stored: Vec<u8> = block_on(
        sqlx::query_scalar("SELECT body FROM snapshots WHERE id = ?1")
            .bind(&snapshot.id)
            .fetch_one(&setup.harness.pool()),
    )
    .unwrap();
    assert_eq!(snapshot.body, stored, "the exact bytes saved");
    // Not uploaded after the failure: sent again, each with its hash as id.
    assert_eq!(second.new_definitions.len(), 3);
    for definition in &second.new_definitions {
        assert_eq!(definition.id, definition.content_id());
    }
    assert!(
        uploads[2].new_definitions.is_empty(),
        "the backend has them"
    );
    let pending: i64 = block_on(
        sqlx::query_scalar("SELECT count(*) FROM item_definitions WHERE uploaded = 0")
            .fetch_one(&setup.harness.pool()),
    )
    .unwrap();
    assert_eq!(pending, 0);
}
