//! Profiles (F2, D26): the Profile commands against the fake backend,
//! `profile 1` and the `Standard` fallback at session start, with `wiremock`
//! GGG and poe.ninja.

use exilence_ce_ng_lib::scheduler::Deadline;
use exilence_ce_ng_lib::session_start;
use exilence_ce_ng_lib::slot::OperationSlot;
use exilence_ce_ng_lib::state::Store;
use ipc::command::{
    CreateProfile, CreateProfileArgs, GetProfileOptions, GetProfileOptionsArgs, NewProfile,
    ProfileOptions, RemoveProfile, RemoveProfileArgs, SetActiveProfile, SetActiveProfileArgs,
    StashTab, UpdateProfile, UpdateProfileArgs,
};
use ipc::slices::{NotificationKind, Notifications, Profile, Profiles, Session};
use ipc::{Field, IpcError, MessageKey, TOAST_EVENT};
use tauri::Manager;
use tauri::async_runtime::{block_on, spawn};
use tonic::{Code, Status};
use wire::v1::ErrorReason;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::fake_backend::{ACCOUNT_ID, FakeBackend, ProfileCall, refused, session, signed_in};
use super::{Harness, NO_BACKEND, mock_server, next_toast, poll, received};

/// GGG with three characters in two leagues; Settlers lists a folder between
/// two tabs, Standard one tab.
pub(super) fn ggg() -> MockServer {
    block_on(async {
        let server = mock_server().await;
        Mock::given(path("/character"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "characters": [
                    { "id": "c1", "name": "Hero", "class": "Witch", "league": "Settlers", "level": 90 },
                    { "id": "c2", "name": "Old", "class": "Duelist", "league": "Standard", "level": 70 },
                    { "id": "c3", "name": "Alt", "class": "Ranger", "league": "Settlers", "level": 12 },
                ]
            })))
            .mount(&server)
            .await;
        Mock::given(path("/stash/Settlers"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "stashes": [
                    { "id": "f1", "name": "Folder", "type": "Folder", "index": 0,
                      "metadata": { "colour": "ff0000", "folder": true },
                      "children": [
                          { "id": "c1", "name": "In folder 1", "type": "NormalStash", "index": 0,
                            "metadata": { "colour": "00ff00" } },
                          { "id": "c2", "name": "In folder 2", "type": "QuadStash", "index": 1,
                            "metadata": {} },
                      ] },
                    { "id": "t2", "name": "Currency", "type": "CurrencyStash", "index": 1,
                      "metadata": { "colour": "7c5436" } },
                ]
            })))
            .mount(&server)
            .await;
        Mock::given(path("/stash/Standard"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "stashes": [
                    { "id": "s1", "name": "Dump", "type": "PremiumStash", "index": 0,
                      "metadata": { "colour": "ffffff" } },
                ]
            })))
            .mount(&server)
            .await;
        server
    })
}

/// poe.ninja pricing two leagues.
pub(super) fn ninja() -> MockServer {
    block_on(async {
        let server = mock_server().await;
        Mock::given(method("GET"))
            .and(path("/poe1/api/economy/leagues"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                { "id": "settlers", "name": "Settlers" },
                { "id": "standard", "name": "Standard" },
            ])))
            .mount(&server)
            .await;
        server
    })
}

fn wire_profile(id: &str, league: &str) -> wire::v1::Profile {
    wire::v1::Profile {
        id: id.to_owned(),
        name: format!("profile {id}"),
        league_id: league.to_owned(),
        price_league_id: "Settlers".to_owned(),
        stash_tab_ids: vec!["t2".to_owned()],
        character_name: Some("Hero".to_owned()),
        include_equipment: true,
        include_inventory: false,
    }
}

fn ipc_profile(id: &str, league: &str) -> Profile {
    let p = wire_profile(id, league);
    Profile {
        id: p.id,
        name: p.name,
        league_id: p.league_id,
        price_league_id: p.price_league_id,
        stash_tab_ids: p.stash_tab_ids,
        character_name: p.character_name,
        include_equipment: p.include_equipment,
        include_inventory: p.include_inventory,
    }
}

/// Profiles `p1` and `p2` in Settlers.
fn p1_and_p2() -> [wire::v1::Profile; 2] {
    [
        wire_profile("p1", "Settlers"),
        wire_profile("p2", "Settlers"),
    ]
}

/// Signed in with `backend`, [`ggg`] and [`ninja`].
fn with_ggg_and_ninja(backend: &FakeBackend, ggg: &MockServer, ninja: &MockServer) -> Harness {
    Harness::with_login()
        .backend(&backend.url)
        .ggg(&ggg.uri())
        .ninja(&ninja.uri())
        .build()
}

impl Harness {
    fn profiles_slice(&self) -> Profiles {
        self.app.state::<Store>().read(Profiles::clone)
    }

    /// Stored Profiles: id, name, position, league.
    fn stored_profiles(&self) -> Vec<(String, String, i64, String)> {
        block_on(
            sqlx::query_as("SELECT id, name, position, league_id FROM profiles ORDER BY position")
                .fetch_all(&self.pool()),
        )
        .unwrap()
    }

    fn stored_active(&self) -> Option<String> {
        block_on(
            sqlx::query_scalar("SELECT active_profile_id FROM accounts").fetch_one(&self.pool()),
        )
        .unwrap()
    }

    fn profile_notifications(&self) -> Notifications {
        self.app.state::<Store>().read(Notifications::clone)
    }
}

fn new_profile(name: &str) -> NewProfile {
    NewProfile {
        name: name.to_owned(),
        league_id: "Settlers".to_owned(),
        price_league_id: "Settlers".to_owned(),
        stash_tab_ids: Vec::new(),
        character_name: None,
        include_equipment: false,
        include_inventory: false,
    }
}

#[test]
fn f2_first_login_creates_profile_1_with_the_first_two_tabs_of_the_first_league() {
    let backend = FakeBackend::start(signed_in());
    backend.script_start_sessions([session(Vec::new(), "")]);
    let (ggg, ninja) = (ggg(), ninja());
    let harness = with_ggg_and_ninja(&backend, &ggg, &ninja);
    let toasts = harness.events(TOAST_EVENT);

    block_on(session_start::run(harness.app.handle()));

    let slice = poll(|| Some(harness.profiles_slice()).filter(|p| !p.profiles.is_empty()));
    let created = &slice.profiles[0];
    assert_eq!(created.name, "profile 1");
    assert_eq!(created.league_id, "Settlers", "the first league");
    assert_eq!(
        created.price_league_id, "Settlers",
        "the first price league"
    );
    assert_eq!(
        created.stash_tab_ids,
        ["c1", "c2"],
        "the first two tabs, folders replaced by their tabs"
    );
    assert_eq!(created.character_name, None);
    assert!(!created.include_equipment && !created.include_inventory);
    assert_eq!(slice.active_profile_id.as_ref(), Some(&created.id));
    assert_eq!(slice.price_leagues, ["Settlers", "Standard"]);
    let id = created.id.clone();
    assert_eq!(uuid::Uuid::parse_str(&id).unwrap().get_version_num(), 7);
    let calls = backend.profile_calls();
    let ProfileCall::Create(sent) = &calls[0] else {
        panic!("{calls:?}");
    };
    assert_eq!(
        (sent.id.as_str(), sent.name.as_str(), &sent.stash_tab_ids),
        (id.as_str(), "profile 1", &created.stash_tab_ids)
    );
    assert_eq!(calls[1..], [ProfileCall::SetActive(id.clone())]);
    assert_eq!(
        harness.stored_profiles(),
        [(id.clone(), "profile 1".to_owned(), 0, "Settlers".to_owned())]
    );
    assert_eq!(harness.stored_active(), Some(id));
    harness.wait_for_ready();
    assert!(toasts.try_recv().is_err(), "a success is list-only");
    assert_eq!(
        harness.profile_notifications().newest[0].message.key,
        MessageKey::NotificationProfileCreated
    );

    // The next session start finds it on the backend, active: nothing is
    // created.
    let id = harness.profiles_slice().profiles[0].id.clone();
    backend.script_start_sessions([session(
        vec![wire::v1::Profile {
            id: id.clone(),
            ..wire_profile("", "Settlers")
        }],
        &id,
    )]);
    let harness = harness.restart();
    block_on(session_start::run(harness.app.handle()));
    harness.wait_for_ready();
    assert_eq!(backend.profile_calls().len(), 2);
}

#[test]
fn f2_no_profile_1_when_the_backend_has_profiles() {
    let backend = FakeBackend::start(signed_in());
    backend.script_start_sessions([session(vec![wire_profile("p1", "Settlers")], "p1")]);
    let (ggg, ninja) = (ggg(), ninja());
    let harness = with_ggg_and_ninja(&backend, &ggg, &ninja);

    block_on(session_start::run(harness.app.handle()));
    harness.wait_for_ready();

    assert!(backend.profile_calls().is_empty());
    // Only the session start's stash tab list of the active league (F3).
    assert_eq!(received(&ggg, "/stash/Settlers"), 1);
    let ids: Vec<String> = harness
        .profiles_slice()
        .profiles
        .into_iter()
        .map(|p| p.id)
        .collect();
    assert_eq!(ids, ["p1"]);
}

#[test]
fn f2_a_profile_whose_league_no_longer_exists_falls_back_to_standard() {
    let backend = FakeBackend::start(signed_in());
    backend.script_start_sessions([session(
        vec![
            wire_profile("p1", "Necropolis"),
            wire_profile("p2", "Settlers"),
        ],
        "p1",
    )]);
    let (ggg, ninja) = (ggg(), ninja());
    let harness = with_ggg_and_ninja(&backend, &ggg, &ninja);

    block_on(session_start::run(harness.app.handle()));

    let slice = poll(|| {
        Some(harness.profiles_slice())
            .filter(|p| p.profiles.first().map(|p| p.league_id.as_str()) == Some("Standard"))
    });
    assert_eq!(
        slice.profiles,
        [
            Profile {
                league_id: "Standard".to_owned(),
                ..ipc_profile("p1", "Necropolis")
            },
            ipc_profile("p2", "Settlers"),
        ]
    );
    // The stored Profile mirrors the backend's; the fallback is what the
    // app works with.
    assert_eq!(harness.stored_profiles()[0].3, "Necropolis");
}

#[test]
fn f2_create_profile_creates_it_without_tabs_and_switches_to_it() {
    let backend = FakeBackend::start(signed_in());
    let harness = Harness::with_login()
        .backend(&backend.url)
        .profiles(&[wire_profile("p1", "Settlers")], "p1")
        .build();
    let toasts = harness.events(TOAST_EVENT);

    harness
        .invoke::<CreateProfile>(&CreateProfileArgs {
            profile: NewProfile {
                name: "  Divine Exile ".to_owned(),
                ..new_profile("")
            },
        })
        .unwrap();

    let slice = harness.profiles_slice();
    assert_eq!(slice.profiles.len(), 2);
    let created = &slice.profiles[1];
    assert_eq!(created.name, "Divine Exile");
    assert!(created.stash_tab_ids.is_empty());
    assert_eq!(slice.active_profile_id.as_ref(), Some(&created.id));
    let expected = wire::v1::Profile {
        id: created.id.clone(),
        name: "Divine Exile".to_owned(),
        league_id: "Settlers".to_owned(),
        price_league_id: "Settlers".to_owned(),
        stash_tab_ids: Vec::new(),
        character_name: None,
        include_equipment: false,
        include_inventory: false,
    };
    assert_eq!(
        backend.profile_calls(),
        [
            ProfileCall::Create(expected),
            ProfileCall::SetActive(created.id.clone())
        ]
    );
    assert_eq!(harness.stored_profiles()[1].2, 1, "after the others");
    assert_eq!(harness.stored_active().as_ref(), Some(&created.id));
    assert!(toasts.try_recv().is_err());
    let notification = &harness.profile_notifications().newest[0];
    assert_eq!(
        (notification.kind, notification.message.key),
        (
            NotificationKind::Success,
            MessageKey::NotificationProfileCreated
        )
    );
}

#[test]
fn f2_a_blank_profile_name_is_an_inline_error() {
    let backend = FakeBackend::start(signed_in());
    let harness = Harness::with_login().backend(&backend.url).build();
    let toasts = harness.events(TOAST_EVENT);

    let error = harness
        .invoke::<CreateProfile>(&CreateProfileArgs {
            profile: new_profile("   "),
        })
        .unwrap_err();

    assert_eq!(
        error,
        IpcError::Validation {
            field: Field::ProfileName,
            key: MessageKey::ErrorProfileNameRequired
        }
    );
    assert!(backend.profile_calls().is_empty());
    // The command shows its toast, if any, before it answers.
    assert!(toasts.try_recv().is_err());
}

#[test]
fn f2_update_and_switch_go_to_the_backend_first() {
    let backend = FakeBackend::start(signed_in());
    let harness = Harness::with_login()
        .backend(&backend.url)
        .profiles(&p1_and_p2(), "p1")
        .build();
    let edited = Profile {
        name: "Rich Trader".to_owned(),
        stash_tab_ids: vec!["s1".to_owned()],
        league_id: "Standard".to_owned(),
        character_name: None,
        include_equipment: false,
        ..ipc_profile("p2", "Settlers")
    };

    harness
        .invoke::<UpdateProfile>(&UpdateProfileArgs {
            profile: edited.clone(),
        })
        .unwrap();
    harness
        .invoke::<SetActiveProfile>(&SetActiveProfileArgs {
            profile_id: "p2".to_owned(),
        })
        .unwrap();

    let calls = backend.profile_calls();
    assert!(matches!(&calls[0], ProfileCall::Update(p) if p.name == "Rich Trader" && p.id == "p2"));
    assert_eq!(calls[1], ProfileCall::SetActive("p2".to_owned()));
    let slice = harness.profiles_slice();
    assert_eq!(slice.profiles[1], edited);
    assert_eq!(slice.active_profile_id.as_deref(), Some("p2"));
    assert_eq!(harness.stored_active().as_deref(), Some("p2"));
    let harness = harness.restart();
    assert_eq!(harness.profiles_slice().profiles[1], edited, "stored");
}

#[test]
fn f2_remove_switches_to_another_profile_first_and_deletes_its_snapshots() {
    let backend = FakeBackend::start(signed_in());
    let harness = Harness::with_login()
        .backend(&backend.url)
        .profiles(&p1_and_p2(), "p1")
        .build();
    let pool = harness.pool();
    block_on(async {
        for (snapshot, profile) in [("s1", "p1"), ("s2", "p2")] {
            sqlx::query(
                "INSERT INTO snapshots (account_id, id, profile_id, created_at, total_value) \
                 VALUES (?1, ?2, ?3, 0, 1.0)",
            )
            .bind(ACCOUNT_ID)
            .bind(snapshot)
            .bind(profile)
            .execute(&pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO snapshot_tabs (account_id, snapshot_id, position, tab_id, name, value) \
                 VALUES (?1, ?2, 0, 't2', 'Currency', 1.0)",
            )
            .bind(ACCOUNT_ID)
            .bind(snapshot)
            .execute(&pool)
            .await
            .unwrap();
        }
        sqlx::query(
            "INSERT INTO pending_removals (account_id, snapshot_id, profile_id) \
             VALUES (?1, 'gone', 'p1'), (?1, 'gone-too', 'p2')",
        )
        .bind(ACCOUNT_ID)
        .execute(&pool)
        .await
        .unwrap();
    });

    harness
        .invoke::<RemoveProfile>(&RemoveProfileArgs {
            profile_id: "p1".to_owned(),
        })
        .unwrap();

    assert_eq!(
        backend.profile_calls(),
        [
            ProfileCall::SetActive("p2".to_owned()),
            ProfileCall::Remove("p1".to_owned())
        ]
    );
    let slice = harness.profiles_slice();
    assert_eq!(slice.profiles, [ipc_profile("p2", "Settlers")]);
    assert_eq!(slice.active_profile_id.as_deref(), Some("p2"));
    let left = |table: &str, column: &str| -> Vec<String> {
        block_on(
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "SELECT {column} FROM {table} ORDER BY 1"
            )))
            .fetch_all(&pool),
        )
        .unwrap()
    };
    assert_eq!(left("snapshots", "id"), ["s2"]);
    assert_eq!(left("snapshot_tabs", "snapshot_id"), ["s2"]);
    assert_eq!(left("pending_removals", "profile_id"), ["p2"]);
    assert_eq!(
        harness.profile_notifications().newest[0].message.key,
        MessageKey::NotificationProfileRemoved
    );

    // The only Profile stays, without asking the backend.
    let error = harness
        .invoke::<RemoveProfile>(&RemoveProfileArgs {
            profile_id: "p2".to_owned(),
        })
        .unwrap_err();
    assert!(matches!(error, IpcError::Internal { .. }), "{error:?}");
    assert_eq!(backend.profile_calls().len(), 2);
    assert_eq!(harness.stored_profiles().len(), 1);
}

#[test]
fn f2_removing_an_inactive_profile_keeps_the_active_one() {
    let backend = FakeBackend::start(signed_in());
    let harness = Harness::with_login()
        .backend(&backend.url)
        .profiles(&p1_and_p2(), "p1")
        .build();

    harness
        .invoke::<RemoveProfile>(&RemoveProfileArgs {
            profile_id: "p2".to_owned(),
        })
        .unwrap();

    assert_eq!(
        backend.profile_calls(),
        [ProfileCall::Remove("p2".to_owned())]
    );
    assert_eq!(
        harness.profiles_slice().active_profile_id.as_deref(),
        Some("p1")
    );
}

/// Every Profile command against `harness`, with its failure title.
fn every_profile_change(harness: &Harness) -> Vec<(MessageKey, Result<(), IpcError>)> {
    vec![
        (
            MessageKey::NotificationProfileCreateFailed,
            harness.invoke::<CreateProfile>(&CreateProfileArgs {
                profile: new_profile("Swift Slayer"),
            }),
        ),
        (
            MessageKey::NotificationProfileUpdateFailed,
            harness.invoke::<UpdateProfile>(&UpdateProfileArgs {
                profile: ipc_profile("p1", "Standard"),
            }),
        ),
        (
            MessageKey::NotificationProfileChangeFailed,
            harness.invoke::<SetActiveProfile>(&SetActiveProfileArgs {
                profile_id: "p2".to_owned(),
            }),
        ),
        (
            MessageKey::NotificationProfileRemoveFailed,
            harness.invoke::<RemoveProfile>(&RemoveProfileArgs {
                profile_id: "p2".to_owned(),
            }),
        ),
    ]
}

#[test]
fn d26_profile_changes_fail_with_a_toast_when_the_backend_cannot_be_reached() {
    let harness = Harness::with_login()
        .backend(NO_BACKEND)
        .profiles(&p1_and_p2(), "p1")
        .build();
    let before = (harness.stored_profiles(), harness.profiles_slice());
    let toasts = harness.events(TOAST_EVENT);

    for (title, result) in every_profile_change(&harness) {
        assert_eq!(result, Err(IpcError::Offline), "{title:?}");
        let toast = next_toast(&toasts);
        assert_eq!(toast.kind, NotificationKind::Error);
        assert_eq!(toast.message.key, title);
        assert_eq!(
            toast.description.map(|d| d.key),
            Some(MessageKey::ErrorOffline)
        );
    }

    assert_eq!(
        (harness.stored_profiles(), harness.profiles_slice()),
        before
    );
    let session = harness.app.state::<Store>().read(Session::clone);
    assert!(!session.offline, "outside a Group nothing shows offline");
}

#[test]
fn f2_profile_changes_are_busy_while_a_snapshot_runs() {
    let backend = FakeBackend::start(signed_in());
    let harness = Harness::with_login()
        .backend(&backend.url)
        .profiles(&p1_and_p2(), "p1")
        .build();
    // A running Snapshot holds the operation slot.
    let slot = harness.app.state::<OperationSlot>();
    let snapshot = slot.try_hold().unwrap();

    for (title, result) in every_profile_change(&harness) {
        assert_eq!(result, Err(IpcError::Busy), "{title:?}");
    }

    assert!(backend.profile_calls().is_empty());
    drop(snapshot);
    harness
        .invoke::<SetActiveProfile>(&SetActiveProfileArgs {
            profile_id: "p2".to_owned(),
        })
        .unwrap();
}

#[test]
fn d25_a_superseded_profile_change_takes_the_client_offline_until_restarted() {
    let backend = FakeBackend::start(signed_in());
    backend.script_profile_failures([refused(Code::Unauthenticated, ErrorReason::Superseded)]);
    let harness = Harness::with_login()
        .backend(&backend.url)
        .profiles(&p1_and_p2(), "p1")
        .build();
    let toasts = harness.events(TOAST_EVENT);

    let changes = every_profile_change(&harness);
    for (title, result) in &changes {
        assert_eq!(result, &Err(IpcError::Superseded), "{title:?}");
    }
    // One toast per change, each saying it was superseded. Under load they
    // may arrive in another order than the changes ran, between other toasts.
    let titles: Vec<MessageKey> = changes.iter().map(|(title, _)| *title).collect();
    let mut toasted = Vec::new();
    while toasted.len() < titles.len() {
        let toast = next_toast(&toasts);
        if titles.contains(&toast.message.key) {
            assert_eq!(
                toast.description.map(|d| d.key),
                Some(MessageKey::ErrorSuperseded)
            );
            toasted.push(toast.message.key);
        }
    }
    let sorted = |mut keys: Vec<MessageKey>| {
        keys.sort_by_key(|k| k.id());
        keys
    };
    assert_eq!(sorted(toasted), sorted(titles));

    assert_eq!(backend.profile_calls().len(), 1, "later ones fail at once");
    assert!(
        harness
            .app
            .state::<Store>()
            .read(|s: &Session| s.superseded)
    );
    assert!(harness.profile_notifications().newest.iter().any(|n| n.kind
        == NotificationKind::Warning
        && n.message.key == MessageKey::ErrorSuperseded));
}

#[test]
fn f2_profile_options_list_the_leagues_and_the_tabs_and_characters_of_one() {
    let backend = FakeBackend::start(signed_in());
    let (ggg, ninja) = (ggg(), ninja());
    let harness = with_ggg_and_ninja(&backend, &ggg, &ninja);

    let first = harness
        .invoke::<GetProfileOptions>(&GetProfileOptionsArgs { league: None })
        .unwrap();
    let standard = harness
        .invoke::<GetProfileOptions>(&GetProfileOptionsArgs {
            league: Some("Standard".to_owned()),
        })
        .unwrap();

    let tab = |id: &str, name: &str, index: u32, colour: Option<&str>| StashTab {
        id: id.to_owned(),
        name: name.to_owned(),
        index,
        colour: colour.map(str::to_owned),
    };
    assert_eq!(
        first,
        ProfileOptions {
            league: Some("Settlers".to_owned()),
            leagues: vec!["Settlers".to_owned(), "Standard".to_owned()],
            stash_tabs: vec![
                tab("c1", "In folder 1", 0, Some("00ff00")),
                tab("c2", "In folder 2", 1, None),
                tab("t2", "Currency", 1, Some("7c5436")),
            ],
            characters: vec!["Hero".to_owned(), "Alt".to_owned()],
        }
    );
    assert_eq!(standard.stash_tabs, [tab("s1", "Dump", 0, Some("ffffff"))]);
    assert_eq!(standard.characters, ["Old"]);
    assert_eq!(
        harness.profiles_slice().price_leagues,
        ["Settlers", "Standard"]
    );
    assert_eq!(received(&ggg, "/character"), 1, "characters loaded once");
    assert_eq!(received(&ninja, "/poe1/api/economy/leagues"), 1);
}

#[test]
fn f2_f7_profile_options_are_rate_limited_during_a_cooldown() {
    let backend = FakeBackend::start(signed_in());
    let ggg = block_on(async {
        let server = mock_server().await;
        Mock::given(path("/character"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "characters": [{ "name": "Hero", "league": "Settlers" }]
            })))
            .mount(&server)
            .await;
        Mock::given(path("/stash/Settlers"))
            .respond_with(
                ResponseTemplate::new(429)
                    .insert_header("X-Rate-Limit-Policy", "stash-list-request-limit")
                    .insert_header("X-Rate-Limit-Rules", "Account")
                    .insert_header("X-Rate-Limit-Account", "30:300:300")
                    .insert_header("X-Rate-Limit-Account-State", "31:300:300")
                    .insert_header("Retry-After", "300"),
            )
            .mount(&server)
            .await;
        server
    });
    let ninja = ninja();
    let harness = with_ggg_and_ninja(&backend, &ggg, &ninja);
    let toasts = harness.events(TOAST_EVENT);
    let options = || harness.invoke::<GetProfileOptions>(&GetProfileOptionsArgs { league: None });

    assert_eq!(options(), Err(IpcError::RateLimited { seconds_left: 300 }));
    assert_eq!(options(), Err(IpcError::RateLimited { seconds_left: 300 }));

    assert_eq!(
        received(&ggg, "/stash/Settlers"),
        1,
        "the second never left"
    );
    let toast = next_toast(&toasts);
    assert_eq!(
        toast.message.key,
        MessageKey::NotificationProfileOptionsFailed
    );
    assert_eq!(
        toast.description.map(|d| d.key),
        Some(MessageKey::ErrorRateLimited)
    );
}

#[test]
fn d21_profile_options_fail_with_a_toast_without_poe_ninja() {
    let backend = FakeBackend::start(signed_in());
    let ggg = ggg();
    let harness = Harness::with_login()
        .backend(&backend.url)
        .ggg(&ggg.uri())
        .build();
    let toasts = harness.events(TOAST_EVENT);

    let error = harness
        .invoke::<GetProfileOptions>(&GetProfileOptionsArgs { league: None })
        .unwrap_err();

    assert!(matches!(error, IpcError::Internal { .. }), "{error:?}");
    let toast = next_toast(&toasts);
    assert_eq!(
        toast.description.map(|d| d.key),
        Some(MessageKey::ErrorPriceLeagues)
    );
    assert!(harness.profiles_slice().price_leagues.is_empty());
}

#[test]
fn f2_profile_1_waits_for_the_backend_and_retries_it_silently() {
    let backend = FakeBackend::start(signed_in());
    backend.script_start_sessions([session(Vec::new(), ""), session(Vec::new(), "")]);
    backend.script_profile_failures([Status::unavailable("backend down")]);
    let (ggg, ninja) = (ggg(), ninja());
    let harness = with_ggg_and_ninja(&backend, &ggg, &ninja);
    let toasts = harness.events(TOAST_EVENT);

    // The characters are known when the backend part's `StartSession`
    // finds no Profile; the GGG part waits for that answer. Then both parts
    // find `profile 1` due: one's `CreateProfile` is held, the other waits
    // for the slot.
    let app = harness.app.handle();
    let held = backend.hold_next_profile_call();
    let ggg_part = spawn({
        let app = app.clone();
        async move { session_start::retry_ggg(&app).await }
    });
    harness.wait_for_characters();
    let backend_part = spawn({
        let app = app.clone();
        async move { session_start::backend_part(&app).await }
    });
    poll(|| (backend.profile_calls().len() == 1).then_some(()));
    let slot = harness.app.state::<OperationSlot>();
    poll(|| (slot.waiting() == 1).then_some(()));
    held.release();
    block_on(backend_part).unwrap();
    block_on(ggg_part).unwrap();

    assert_eq!(
        backend.profile_calls().len(),
        1,
        "the waiting attempt stands down until the backend part's retry"
    );
    let due = harness
        .due(Deadline::BackendSessionRetry)
        .expect("the backend part retries");
    assert!(harness.profiles_slice().profiles.is_empty());
    assert!(toasts.try_recv().is_err(), "silent");

    harness.clock().advance(due - super::fake_ggg::START);
    assert_eq!(harness.tick(), [Deadline::BackendSessionRetry]);

    let slice = poll(|| Some(harness.profiles_slice()).filter(|p| !p.profiles.is_empty()));
    assert_eq!(slice.profiles[0].name, "profile 1");
    assert_eq!(backend.start_session_count(), 2);
}

#[test]
fn f2_profile_1_is_made_active_by_the_retry_when_its_activation_failed() {
    let backend = FakeBackend::start(signed_in());
    backend.script_start_sessions([session(Vec::new(), "")]);
    // `CreateProfile` succeeds, `SetActiveProfile` fails.
    backend.script_profile_failures([
        Status::new(Code::Ok, ""),
        Status::unavailable("backend down"),
    ]);
    let (ggg, ninja) = (ggg(), ninja());
    let harness = with_ggg_and_ninja(&backend, &ggg, &ninja);
    let toasts = harness.events(TOAST_EVENT);

    // The characters are known when the backend part's `StartSession`
    // finds no Profile: `profile 1` is created, but cannot be activated.
    let app = harness.app.handle();
    let ggg_part = spawn({
        let app = app.clone();
        async move { session_start::retry_ggg(&app).await }
    });
    harness.wait_for_characters();
    block_on(session_start::backend_part(app));
    block_on(ggg_part).unwrap();

    let calls = backend.profile_calls();
    let [ProfileCall::Create(created), ProfileCall::SetActive(_)] = &calls[..] else {
        panic!("{calls:?}");
    };
    let due = harness
        .due(Deadline::BackendSessionRetry)
        .expect("the backend part retries");
    assert!(toasts.try_recv().is_err(), "silent");

    // The retry's `StartSession` finds `profile 1`, but no active Profile.
    backend.script_start_sessions([session(vec![created.clone()], "")]);
    harness.clock().advance(due - super::fake_ggg::START);
    assert_eq!(harness.tick(), [Deadline::BackendSessionRetry]);

    let slice = poll(|| Some(harness.profiles_slice()).filter(|p| p.active_profile_id.is_some()));
    assert_eq!(slice.active_profile_id.as_ref(), Some(&created.id));
    assert_eq!(
        backend.profile_calls()[2..],
        [ProfileCall::SetActive(created.id.clone())],
        "activated, not created again"
    );
    assert_eq!(harness.stored_active(), Some(created.id.clone()));
    assert!(toasts.try_recv().is_err(), "silent");
}
