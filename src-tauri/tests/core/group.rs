//! Groups (F13, D4): join and create, the Group stream's `GroupSync` and
//! relays, the player selection and the Group views (`net_worth`, `charts`
//! and `items`, D14, D15), relayed Profile changes (D8), leave, against the
//! fake backend scripting the stream's events.

use exilence_ce_ng_lib::slot::OperationSlot;
use exilence_ce_ng_lib::state::Store;
use exilence_ce_ng_lib::{auth, profiles};
use ipc::command::{
    CheckGroupName, CheckGroupNameArgs, CreateGroup, CreateGroupArgs, GroupForm, JoinGroup,
    JoinGroupArgs, LeaveGroup, NoArgs, SetPlayerSelected, SetPlayerSelectedArgs,
};
use ipc::command::{SettingsPatch, UpdateSettings, UpdateSettingsArgs};
use ipc::slices::{
    Charts, Group, GroupMember, Items, NetWorth, NotificationKind, Notifications, TimeSpan,
};
use ipc::{Field, IpcError, MessageKey};
use prost::Message as _;
use tauri::Manager;
use tauri::async_runtime::block_on;
use tonic::Code;
use wire::v1::group_event::Event;
use wire::v1::{
    Account, ActiveProfileChanged, ErrorReason, ItemDefinition, Member, MemberJoined, MemberLeft,
    Profile, SnapshotAdded, SnapshotBody, SnapshotTab, StartSessionResponse, TabValue,
};

use super::fake_backend::{
    ACCOUNT_ID, ACCOUNT_NAME, FakeBackend, GroupCall, account, default_profile, group_sync,
    refused, signed_in,
};
use super::fake_ggg::START;
use super::{Harness, poll};

const MINUTE: i64 = 60_000;
pub(super) const BAR_ID: &str = "9d8c7b6a-5f4e-4d3c-8b2a-1f0e9d8c7b6a";
pub(super) const BAZ_ID: &str = "1a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d";

/// Signed in with the fake backend's account and its Profile, which has one
/// Snapshot of 100c taken 10 minutes ago.
pub(super) fn signed_in_harness(backend: &FakeBackend) -> Harness {
    let profile = default_profile();
    let harness = Harness::with_login()
        .backend(&backend.url)
        .profiles(std::slice::from_ref(&profile), &profile.id)
        .build();
    block_on(
        sqlx::query(
            "INSERT INTO snapshots (account_id, id, profile_id, created_at, total_value) \
             VALUES (?1, 'own-1', ?2, ?3, 100.0)",
        )
        .bind(ACCOUNT_ID)
        .bind(&profile.id)
        .bind(START - 10 * MINUTE)
        .execute(&harness.pool()),
    )
    .unwrap();
    block_on(profiles::push(harness.app.handle()));
    harness
}

fn definition(name: &str) -> ItemDefinition {
    let mut definition = ItemDefinition {
        name: name.to_owned(),
        type_line: name.to_owned(),
        frame_type: 5,
        ..ItemDefinition::default()
    };
    definition.id = definition.content_id();
    definition
}

/// A zstd `SnapshotBody` with one tab holding one of each definition.
fn body(definition_ids: &[u64]) -> Vec<u8> {
    let body = SnapshotBody {
        price_league_id: "Settlers".to_owned(),
        tabs: vec![SnapshotTab {
            tab_id: "t1".to_owned(),
            definition_ids: definition_ids.to_vec(),
            stack_sizes: vec![1; definition_ids.len()],
            ..SnapshotTab::default()
        }],
        priced_definition_ids: Vec::new(),
        prices: Vec::new(),
    };
    zstd::bulk::compress(&body.encode_to_vec(), 3).unwrap()
}

fn profile(id: &str) -> Profile {
    Profile {
        id: id.to_owned(),
        name: "league".to_owned(),
        league_id: "Settlers".to_owned(),
        price_league_id: "Settlers".to_owned(),
        ..Profile::default()
    }
}

/// A member's Snapshot of `profile_id`, `minutes_ago` before [`START`].
fn snapshot(
    id: &str,
    profile_id: &str,
    minutes_ago: i64,
    total: f64,
    body: Vec<u8>,
) -> wire::v1::Snapshot {
    let at = START - minutes_ago * MINUTE;
    wire::v1::Snapshot {
        id: id.to_owned(),
        profile_id: profile_id.to_owned(),
        created_at: Some(prost_types::Timestamp {
            seconds: at / 1000,
            nanos: 0,
        }),
        total_value: total,
        tab_values: vec![TabValue {
            tab_id: "t1".to_owned(),
            value: total,
        }],
        body,
    }
}

pub(super) fn member(id: &str, name: &str, snapshots: Vec<wire::v1::Snapshot>) -> Member {
    Member {
        account: Some(Account {
            id: id.to_owned(),
            name: name.to_owned(),
        }),
        active_profile: Some(profile(&format!("{id}-profile"))),
        snapshots,
    }
}

/// Bar, whose latest Snapshot is 1000c, taken 20 minutes ago.
pub(super) fn bar(body: Vec<u8>) -> Member {
    let profile_id = format!("{BAR_ID}-profile");
    member(
        BAR_ID,
        "Bar#5678",
        vec![snapshot("bar-1", &profile_id, 20, 1000.0, body)],
    )
}

fn form(name: &str, password: &str) -> GroupForm {
    GroupForm {
        name: name.to_owned(),
        password: password.to_owned(),
    }
}

pub(super) fn join(harness: &Harness, name: &str, password: &str) -> Result<(), IpcError> {
    harness.invoke::<JoinGroup>(&JoinGroupArgs {
        group: form(name, password),
    })
}

pub(super) fn group(harness: &Harness) -> Group {
    harness.app.state::<Store>().read(Group::clone)
}

pub(super) fn net_worth(harness: &Harness) -> NetWorth {
    harness.app.state::<Store>().read(NetWorth::clone)
}

pub(super) fn notifications(harness: &Harness) -> Notifications {
    harness.app.state::<Store>().read(Notifications::clone)
}

pub(super) fn players(harness: &Harness) -> Vec<(String, bool)> {
    group(harness)
        .members
        .into_iter()
        .map(|m| (m.account_name, m.selected))
        .collect()
}

/// The stored definitions and whether each is marked as the backend's.
fn stored_definitions(harness: &Harness) -> Vec<(i64, bool)> {
    block_on(
        sqlx::query_as("SELECT id, uploaded = 1 FROM item_definitions ORDER BY id")
            .fetch_all(&harness.pool()),
    )
    .unwrap()
}

pub(super) fn select(harness: &Harness, account_id: &str, selected: bool) {
    harness
        .invoke::<SetPlayerSelected>(&SetPlayerSelectedArgs {
            account_id: account_id.to_owned(),
            selected,
        })
        .unwrap();
}

#[test]
fn f13_d4_join_shows_every_member_selected_with_their_latest_snapshot() {
    let backend = FakeBackend::start(signed_in());
    let (chaos, divine) = (definition("Chaos Orb"), definition("Divine Orb"));
    backend.know_definitions([chaos.clone(), divine.clone()]);
    backend.script_group_syncs([Ok(group_sync(
        "Rich Exiles123",
        vec![bar(body(&[divine.id, chaos.id]))],
    ))]);
    let harness = signed_in_harness(&backend);
    assert_eq!(net_worth(&harness).total, 100.0);

    join(&harness, " Rich Exiles123 ", "secret1").unwrap();

    let mut ids = vec![chaos.id, divine.id];
    ids.sort_unstable();
    assert_eq!(
        backend.group_calls(),
        [
            GroupCall::Join {
                name: "Rich Exiles123".to_owned(),
                password: "secret1".to_owned()
            },
            GroupCall::Open(0),
            GroupCall::GetDefinitions(ids.clone()),
        ]
    );
    assert_eq!(
        group(&harness),
        Group {
            name: Some("Rich Exiles123".to_owned()),
            members: vec![
                GroupMember {
                    account_id: ACCOUNT_ID.to_owned(),
                    account_name: ACCOUNT_NAME.to_owned(),
                    selected: true,
                },
                GroupMember {
                    account_id: BAR_ID.to_owned(),
                    account_name: "Bar#5678".to_owned(),
                    selected: true,
                },
            ],
        }
    );
    let combined = net_worth(&harness);
    assert!(combined.group_combined);
    assert_eq!(combined.total, 1100.0, "D4: Bar's latest counts at once");
    assert_eq!(combined.snapshot_count, 2);
    assert_eq!(combined.last_snapshot_at, Some(START - 10 * MINUTE));
    assert_eq!(
        stored_definitions(&harness),
        ids.iter().map(|&id| (id as i64, true)).collect::<Vec<_>>(),
        "fetched definitions are stored as the backend's"
    );
    let newest = &notifications(&harness).newest[0];
    assert_eq!(newest.kind, NotificationKind::Success);
    assert_eq!(newest.message.key, MessageKey::NotificationGroupJoined);
    assert!(backend.group_stream_open());
}

#[test]
fn f13_f18_name_and_password_refusals_show_on_their_fields_and_a_refused_name_also_toasts() {
    let backend = FakeBackend::start(signed_in());
    let harness = signed_in_harness(&backend);
    let field = |result: Result<(), IpcError>| match result {
        Err(IpcError::Validation { field, key }) => (field, key),
        other => panic!("a field error, got {other:?}"),
    };

    // Checked before the backend: a name and a password of at least 6.
    assert_eq!(
        field(join(&harness, "  ", "")),
        (Field::GroupName, MessageKey::ErrorGroupNameRequired)
    );
    assert_eq!(
        field(harness.invoke::<CreateGroup>(&CreateGroupArgs {
            group: form("Rich Exiles123", "12345"),
        })),
        (Field::GroupPassword, MessageKey::ErrorGroupPasswordTooShort)
    );
    assert!(backend.group_calls().is_empty());

    backend.script_group_failures([
        refused(Code::PermissionDenied, ErrorReason::WrongPassword),
        refused(Code::NotFound, ErrorReason::GroupNotFound),
        refused(Code::AlreadyExists, ErrorReason::GroupNameTaken),
    ]);
    assert_eq!(
        field(join(&harness, "Rich Exiles123", "wrong1")),
        (Field::GroupPassword, MessageKey::ErrorWrongPassword)
    );
    assert_eq!(
        field(join(&harness, "Nobody", "")),
        (Field::GroupName, MessageKey::ErrorGroupNotFound)
    );
    assert_eq!(
        field(harness.invoke::<CreateGroup>(&CreateGroupArgs {
            group: form("Rich Exiles123", ""),
        })),
        (Field::GroupName, MessageKey::ErrorGroupNameTaken)
    );
    // F18: a refused name also toasts, a wrong password only shows on its
    // field; newest first.
    let failures: Vec<_> = notifications(&harness)
        .newest
        .iter()
        .map(|n| (n.kind, n.message.key))
        .collect();
    assert_eq!(
        failures,
        [(
            NotificationKind::Error,
            MessageKey::NotificationGroupJoinFailed
        ); 2],
        "the not-found and name-taken refusals, not the wrong password"
    );
    assert!(!backend.group_calls().contains(&GroupCall::Open(0)));
    assert_eq!(group(&harness), Group::default());

    // The name check on blur.
    backend.script_group_names([true, false]);
    let check = |name: &str| {
        harness
            .invoke::<CheckGroupName>(&CheckGroupNameArgs {
                name: name.to_owned(),
            })
            .unwrap()
    };
    assert!(check(" rich exiles123 "));
    assert!(!check("Fine Farmers100"));
    assert!(
        backend
            .group_calls()
            .contains(&GroupCall::CheckName("rich exiles123".to_owned()))
    );
}

#[test]
fn f13_group_actions_are_busy_while_the_operation_slot_is_taken() {
    let backend = FakeBackend::start(signed_in());
    let harness = signed_in_harness(&backend);
    let slot = harness.app.state::<OperationSlot>();
    let held = slot.try_hold().unwrap();
    assert_eq!(join(&harness, "Rich Exiles123", ""), Err(IpcError::Busy));
    assert_eq!(
        harness.invoke::<LeaveGroup>(&NoArgs {}),
        Err(IpcError::Busy)
    );
    drop(held);
    assert!(backend.group_calls().is_empty());
}

#[test]
fn f13_relayed_members_and_snapshots_update_the_combined_net_worth_by_selection() {
    let backend = FakeBackend::start(signed_in());
    let harness = signed_in_harness(&backend);
    backend.script_group_syncs([Ok(group_sync("Rich Exiles123", vec![bar(Vec::new())]))]);
    harness
        .invoke::<CreateGroup>(&CreateGroupArgs {
            group: form("Rich Exiles123", ""),
        })
        .unwrap();
    assert_eq!(net_worth(&harness).total, 1100.0);

    // A later player appears unselected.
    let baz_profile = format!("{BAZ_ID}-profile");
    assert!(backend.send_group_event(Event::MemberJoined(MemberJoined {
        member: Some(member(
            BAZ_ID,
            "Baz#9012",
            vec![snapshot("baz-1", &baz_profile, 30, 50.0, Vec::new())],
        )),
    })));
    poll(|| (group(&harness).members.len() == 3).then_some(()));
    assert_eq!(
        players(&harness),
        [
            (ACCOUNT_NAME.to_owned(), true),
            ("Bar#5678".to_owned(), true),
            ("Baz#9012".to_owned(), false),
        ]
    );
    assert_eq!(net_worth(&harness).total, 1100.0);

    // Bar's new Snapshot with the definitions it uploaded; the one it
    // references that this client lacks is fetched.
    let (provided, missing) = (definition("Exalted Orb"), definition("Mirror"));
    backend.know_definitions([missing.clone()]);
    let bar_profile = format!("{BAR_ID}-profile");
    assert!(
        backend.send_group_event(Event::SnapshotAdded(SnapshotAdded {
            account_id: BAR_ID.to_owned(),
            snapshot: Some(snapshot(
                "bar-2",
                &bar_profile,
                1,
                1200.0,
                body(&[provided.id, missing.id])
            )),
            new_definitions: vec![provided.clone()],
        }))
    );
    poll(|| (net_worth(&harness).total == 1300.0).then_some(()));
    let combined = net_worth(&harness);
    assert_eq!(combined.change, 200.0, "Bar's newest Snapshot changed it");
    assert_eq!(combined.sparkline, [1100.0, 1300.0]);
    assert_eq!(combined.snapshot_count, 3);
    assert!(
        backend
            .group_calls()
            .contains(&GroupCall::GetDefinitions(vec![missing.id]))
    );
    let mut expected = vec![(provided.id as i64, true), (missing.id as i64, true)];
    expected.sort_unstable();
    assert_eq!(stored_definitions(&harness), expected);

    // A Snapshot of another Profile than the member's active one is not
    // theirs to show.
    assert!(
        backend.send_group_event(Event::SnapshotAdded(SnapshotAdded {
            account_id: BAR_ID.to_owned(),
            snapshot: Some(snapshot("bar-x", "another-profile", 0, 9999.0, Vec::new())),
            new_definitions: Vec::new(),
        }))
    );

    // Each checkbox controls whether that player counts.
    select(&harness, BAZ_ID, true);
    assert_eq!(net_worth(&harness).total, 1350.0);
    select(&harness, ACCOUNT_ID, false);
    assert_eq!(net_worth(&harness).total, 1250.0);
    assert_eq!(
        players(&harness),
        [
            (ACCOUNT_NAME.to_owned(), false),
            ("Bar#5678".to_owned(), true),
            ("Baz#9012".to_owned(), true),
        ]
    );

    // A player who leaves is gone at once.
    assert!(backend.send_group_event(Event::MemberLeft(MemberLeft {
        account_id: BAZ_ID.to_owned(),
    })));
    poll(|| (group(&harness).members.len() == 2).then_some(()));
    // The `group` slice goes out before the views it changes.
    poll(|| (net_worth(&harness).total == 1200.0).then_some(()));
    assert!(net_worth(&harness).group_combined);
}

#[test]
fn f13_definitions_are_fetched_at_most_10000_per_call() {
    let backend = FakeBackend::start(signed_in());
    let harness = signed_in_harness(&backend);
    join(&harness, "Rich Exiles123", "").unwrap();
    let ids: Vec<u64> = (1..=10_001).collect();
    let bar_profile = format!("{BAR_ID}-profile");
    assert!(backend.send_group_event(Event::MemberJoined(MemberJoined {
        member: Some(member(
            BAR_ID,
            "Bar#5678",
            vec![snapshot("bar-1", &bar_profile, 5, 10.0, body(&ids))],
        )),
    })));
    poll(|| {
        let fetched: Vec<usize> = backend
            .group_calls()
            .into_iter()
            .filter_map(|call| match call {
                GroupCall::GetDefinitions(ids) => Some(ids.len()),
                _ => None,
            })
            .collect();
        (fetched == [10_000, 1]).then_some(())
    });
}

#[test]
fn f13_leave_group_closes_the_stream_and_returns_to_solo() {
    let backend = FakeBackend::start(signed_in());
    backend.script_group_syncs([Ok(group_sync("Rich Exiles123", vec![bar(Vec::new())]))]);
    let harness = signed_in_harness(&backend);
    join(&harness, "Rich Exiles123", "").unwrap();

    // A failed leave keeps the Group, with a toast.
    backend.script_group_failures([tonic::Status::unavailable("down")]);
    assert_eq!(
        harness.invoke::<LeaveGroup>(&NoArgs {}),
        Err(IpcError::Offline)
    );
    assert_eq!(group(&harness).name.as_deref(), Some("Rich Exiles123"));
    let failed = &notifications(&harness).newest[0];
    assert_eq!(failed.kind, NotificationKind::Error);
    assert_eq!(failed.message.key, MessageKey::NotificationGroupLeaveFailed);

    harness.invoke::<LeaveGroup>(&NoArgs {}).unwrap();

    assert_eq!(backend.group_calls().last(), Some(&GroupCall::Leave));
    assert_eq!(group(&harness), Group::default());
    let solo = net_worth(&harness);
    assert!(!solo.group_combined);
    assert_eq!(solo.total, 100.0);
    assert_eq!(
        notifications(&harness).newest[0].message.key,
        MessageKey::NotificationGroupLeft
    );
}

#[test]
fn f13_sign_out_closes_the_group_stream() {
    let backend = FakeBackend::start(signed_in());
    let harness = signed_in_harness(&backend);
    join(&harness, "Rich Exiles123", "").unwrap();
    assert!(backend.group_stream_open());

    block_on(auth::sign_out(harness.app.handle())).unwrap();

    poll(|| (!backend.group_stream_open()).then_some(()));
    assert_eq!(group(&harness), Group::default());
    assert!(
        !backend.group_calls().contains(&GroupCall::Leave),
        "the membership stays (D22)"
    );
}

#[test]
fn f13_a_session_start_in_a_group_rejoins_as_on_a_first_join() {
    let backend = FakeBackend::start(signed_in());
    backend.script_start_sessions([Ok(StartSessionResponse {
        account: Some(account()),
        profiles: vec![default_profile()],
        active_profile_id: default_profile().id,
        group_name: Some("Rich Exiles123".to_owned()),
    })]);
    backend.script_group_syncs([Ok(group_sync("Rich Exiles123", vec![bar(Vec::new())]))]);
    let harness = signed_in_harness(&backend);

    block_on(exilence_ce_ng_lib::session_start::run(harness.app.handle()));

    poll(|| group(&harness).name.is_some().then_some(()));
    assert_eq!(backend.group_calls(), [GroupCall::Open(0)]);
    assert!(
        group(&harness).members.iter().all(|m| m.selected),
        "all members selected"
    );
    assert_eq!(net_worth(&harness).total, 1100.0);
}

/// A zstd `SnapshotBody` with one tab holding each definition with its
/// stack size and unit price.
fn priced_body(items: &[(&ItemDefinition, u32, f64)]) -> Vec<u8> {
    let mut priced: Vec<(u64, f64)> = items.iter().map(|(d, _, price)| (d.id, *price)).collect();
    priced.sort_by_key(|&(id, _)| id);
    let body = SnapshotBody {
        price_league_id: "Settlers".to_owned(),
        tabs: vec![SnapshotTab {
            tab_id: "t1".to_owned(),
            name: "Currency".to_owned(),
            definition_ids: items.iter().map(|(d, _, _)| d.id).collect(),
            stack_sizes: items.iter().map(|(_, stack, _)| *stack).collect(),
            ..SnapshotTab::default()
        }],
        priced_definition_ids: priced.iter().map(|&(id, _)| id).collect(),
        prices: priced.iter().map(|&(_, price)| price).collect(),
    };
    zstd::bulk::compress(&body.encode_to_vec(), 3).unwrap()
}

fn charts(harness: &Harness) -> Charts {
    harness.app.state::<Store>().read(Charts::clone)
}

fn items(harness: &Harness) -> Items {
    harness.app.state::<Store>().read(Items::clone)
}

/// The net worth chart as (series name, values).
fn net_worth_lines(harness: &Harness) -> Vec<(String, Vec<f64>)> {
    charts(harness)
        .net_worth
        .into_iter()
        .map(|s| (s.name, s.points.iter().map(|p| p.value).collect()))
        .collect()
}

fn line(name: &str, values: &[f64]) -> (String, Vec<f64>) {
    (name.to_owned(), values.to_vec())
}

/// Rows as (name, quantity, total, number of tabs).
fn item_rows(rows: &[ipc::slices::ItemRow]) -> Vec<(String, i64, f64, usize)> {
    rows.iter()
        .map(|r| (r.name.clone(), r.quantity, r.total, r.tabs.len()))
        .collect()
}

fn item_row(name: &str, quantity: i64, total: f64) -> (String, i64, f64, usize) {
    (name.to_owned(), quantity, total, 0)
}

#[test]
fn d8_d4_a_relayed_profile_switch_shows_the_new_profiles_latest_snapshot() {
    let backend = FakeBackend::start(signed_in());
    let divine = definition("Divine Orb");
    backend.know_definitions([divine.clone()]);
    backend.script_group_syncs([Ok(group_sync("Rich Exiles123", vec![bar(Vec::new())]))]);
    let harness = signed_in_harness(&backend);
    join(&harness, "Rich Exiles123", "").unwrap();
    assert_eq!(net_worth(&harness).total, 1100.0);
    assert_eq!(
        net_worth_lines(&harness),
        [line(ACCOUNT_NAME, &[100.0]), line("Bar#5678", &[1000.0])]
    );

    // Bar switches Profile: its latest Snapshot replaces what Bar showed.
    let switched = Profile {
        name: "bar ssf".to_owned(),
        ..profile("bar-ssf-profile")
    };
    assert!(
        backend.send_group_event(Event::ActiveProfileChanged(ActiveProfileChanged {
            account_id: BAR_ID.to_owned(),
            profile: Some(switched.clone()),
            latest_snapshot: Some(snapshot(
                "bar-ssf-1",
                "bar-ssf-profile",
                5,
                300.0,
                priced_body(&[(&divine, 2, 150.0)]),
            )),
        }))
    );
    poll(|| (net_worth(&harness).total == 400.0).then_some(()));
    poll(|| (items(&harness).rows.len() == 1).then_some(()));
    assert_eq!(
        net_worth_lines(&harness),
        [line(ACCOUNT_NAME, &[100.0]), line("Bar#5678", &[300.0])]
    );
    let shown = items(&harness);
    assert!(shown.group_combined);
    assert_eq!(
        item_rows(&shown.rows),
        [item_row("Divine Orb", 2, 300.0)],
        "its definitions fetched"
    );

    // An edit of the active Profile keeps its Snapshots (D8).
    assert!(
        backend.send_group_event(Event::ActiveProfileChanged(ActiveProfileChanged {
            account_id: BAR_ID.to_owned(),
            profile: Some(Profile {
                name: "bar ssf renamed".to_owned(),
                ..switched
            }),
            latest_snapshot: None,
        }))
    );
    // Events apply in order: once Baz is in, the edit was applied.
    assert!(backend.send_group_event(Event::MemberJoined(MemberJoined {
        member: Some(member(BAZ_ID, "Baz#9012", Vec::new())),
    })));
    poll(|| (group(&harness).members.len() == 3).then_some(()));
    assert_eq!(net_worth(&harness).total, 400.0);
    assert_eq!(
        net_worth_lines(&harness),
        [line(ACCOUNT_NAME, &[100.0]), line("Bar#5678", &[300.0])]
    );
    assert_eq!(item_rows(&items(&harness).rows).len(), 1);
}

#[test]
fn d14_a_members_item_rows_show_definitions_that_arrive_later() {
    let backend = FakeBackend::start(signed_in());
    let (chaos, mirror) = (definition("Chaos Orb"), definition("Mirror"));
    // The backend cannot hand over the Mirror's definition at the join.
    backend.know_definitions([chaos.clone()]);
    let body = || priced_body(&[(&chaos, 1, 1.0), (&mirror, 1, 50.0)]);
    backend.script_group_syncs([Ok(group_sync("Rich Exiles123", vec![bar(body())]))]);
    let harness = signed_in_harness(&backend);
    join(&harness, "Rich Exiles123", "").unwrap();
    assert_eq!(
        item_rows(&items(&harness).rows),
        [item_row("Chaos Orb", 1, 1.0)]
    );

    // Bar's next Snapshot, the same items, relays it.
    assert!(
        backend.send_group_event(Event::SnapshotAdded(SnapshotAdded {
            account_id: BAR_ID.to_owned(),
            snapshot: Some(snapshot(
                "bar-2",
                &format!("{BAR_ID}-profile"),
                1,
                1051.0,
                body()
            )),
            new_definitions: vec![mirror.clone()],
        }))
    );
    poll(|| (items(&harness).rows.len() == 2).then_some(()));
    assert_eq!(
        item_rows(&items(&harness).comparison_rows),
        [],
        "the older Snapshot shows the Mirror too"
    );
}

#[test]
fn d14_d15_group_charts_and_items_follow_the_selection_and_time_span() {
    let backend = FakeBackend::start(signed_in());
    let (chaos, divine) = (definition("Chaos Orb"), definition("Divine Orb"));
    backend.know_definitions([chaos.clone(), divine.clone()]);
    backend.script_group_syncs([Ok(group_sync(
        "Rich Exiles123",
        vec![bar(priced_body(&[(&chaos, 20, 1.0), (&divine, 1, 150.0)]))],
    ))]);
    let harness = signed_in_harness(&backend);
    // The account's own Snapshots: 90c two days ago, its 100c one with 10
    // chaos; the older one held 5.
    let pool = harness.pool();
    block_on(
        sqlx::query("INSERT INTO item_definitions (id, encoded, uploaded) VALUES (?1, ?2, 1)")
            .bind(chaos.id as i64)
            .bind(chaos.content_bytes())
            .execute(&pool),
    )
    .unwrap();
    block_on(
        sqlx::query("UPDATE snapshots SET body = ?1 WHERE id = 'own-1'")
            .bind(priced_body(&[(&chaos, 10, 1.0)]))
            .execute(&pool),
    )
    .unwrap();
    block_on(
        sqlx::query(
            "INSERT INTO snapshots (account_id, id, profile_id, created_at, total_value, body) \
             VALUES (?1, 'own-0', ?2, ?3, 90.0, ?4)",
        )
        .bind(ACCOUNT_ID)
        .bind(default_profile().id)
        .bind(START - 2 * 24 * 60 * MINUTE)
        .bind(priced_body(&[(&chaos, 5, 1.0)]))
        .execute(&pool),
    )
    .unwrap();

    join(&harness, "Rich Exiles123", "").unwrap();

    // D15: a series per selected player, named by account.
    assert_eq!(
        net_worth_lines(&harness),
        [
            line(ACCOUNT_NAME, &[90.0, 100.0]),
            line("Bar#5678", &[1000.0])
        ]
    );
    assert_eq!(
        charts(&harness).tabs.len(),
        0,
        "the tab chart stays the account's own"
    );
    // F11: the selected players' newest Snapshots merged, without tabs.
    let shown = items(&harness);
    assert!(shown.group_combined);
    assert!(shown.tabs.is_empty());
    assert_eq!(
        item_rows(&shown.rows),
        [
            item_row("Chaos Orb", 30, 30.0),
            item_row("Divine Orb", 1, 150.0)
        ]
    );
    // D14: Bar has one Snapshot, so only the account's own diff counts.
    assert_eq!(
        item_rows(&shown.comparison_rows),
        [item_row("Chaos Orb", 5, 5.0)]
    );

    // Bar's next Snapshot: its own diff merges with the account's.
    let bar_profile = format!("{BAR_ID}-profile");
    assert!(
        backend.send_group_event(Event::SnapshotAdded(SnapshotAdded {
            account_id: BAR_ID.to_owned(),
            snapshot: Some(snapshot(
                "bar-2",
                &bar_profile,
                1,
                1005.0,
                priced_body(&[(&chaos, 25, 1.0), (&divine, 1, 150.0)]),
            )),
            new_definitions: Vec::new(),
        }))
    );
    poll(|| {
        (items(&harness).comparison_rows.first().map(|r| r.quantity) == Some(10)).then_some(())
    });
    assert_eq!(
        item_rows(&items(&harness).comparison_rows),
        [item_row("Chaos Orb", 10, 10.0)],
        "5 more chaos each; Bar's divine did not change"
    );
    assert_eq!(
        net_worth_lines(&harness),
        [
            line(ACCOUNT_NAME, &[90.0, 100.0]),
            line("Bar#5678", &[1000.0, 1005.0])
        ]
    );

    // The time span cuts every series.
    harness
        .invoke::<UpdateSettings>(&UpdateSettingsArgs {
            patch: SettingsPatch {
                chart_time_span: Some(TimeSpan::Day),
                ..SettingsPatch::default()
            },
        })
        .unwrap();
    poll(|| (charts(&harness).time_span == TimeSpan::Day).then_some(()));
    assert_eq!(
        net_worth_lines(&harness),
        [
            line(ACCOUNT_NAME, &[100.0]),
            line("Bar#5678", &[1000.0, 1005.0])
        ]
    );

    // Deselected players leave every view.
    select(&harness, ACCOUNT_ID, false);
    assert_eq!(
        net_worth_lines(&harness),
        [line("Bar#5678", &[1000.0, 1005.0])]
    );
    let shown = items(&harness);
    assert_eq!(
        item_rows(&shown.rows),
        [
            item_row("Chaos Orb", 25, 25.0),
            item_row("Divine Orb", 1, 150.0)
        ]
    );
    assert_eq!(
        item_rows(&shown.comparison_rows),
        [item_row("Chaos Orb", 5, 5.0)]
    );
    select(&harness, BAR_ID, false);
    assert!(charts(&harness).net_worth.is_empty());
    let shown = items(&harness);
    assert!(shown.group_combined);
    assert!(shown.rows.is_empty() && shown.comparison_rows.is_empty());

    // Leaving returns to the account's own views.
    harness.invoke::<LeaveGroup>(&NoArgs {}).unwrap();
    assert_eq!(net_worth_lines(&harness), [line("profile 1", &[100.0])]);
    let shown = items(&harness);
    assert!(!shown.group_combined);
    assert_eq!(shown.rows.len(), 1);
    assert_eq!(shown.rows[0].tabs.len(), 1, "solo rows have their tabs");
}
