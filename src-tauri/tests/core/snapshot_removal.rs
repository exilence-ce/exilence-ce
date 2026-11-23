//! Removing Snapshots (F12, D5, D26) against the fake backend: local first,
//! even offline, under the operation slot; the backend's copies via
//! `pending_removals`, sent again after every successful `StartSession`.

use exilence_ce_ng_lib::slot::OperationSlot;
use exilence_ce_ng_lib::state::Store;
use exilence_ce_ng_lib::{profiles, session_start, snapshot_removal};
use ipc::command::{ListSnapshots, NoArgs, RemoveSnapshots, RemoveSnapshotsArgs, SnapshotEntry};
use ipc::slices::{NetWorth, NotificationKind, Toast};
use ipc::{IpcError, MessageKey, TOAST_EVENT};
use tauri::Manager;
use tauri::async_runtime::block_on;
use tonic::Status;

use super::fake_backend::{ACCOUNT_ID, FakeBackend, ProfileCall, session, signed_in};
use super::{Harness, NO_BACKEND, next_toast, poll};

fn wire_profile(id: &str) -> wire::v1::Profile {
    wire::v1::Profile {
        id: id.to_owned(),
        name: format!("profile {id}"),
        league_id: "Settlers".to_owned(),
        price_league_id: "Settlers".to_owned(),
        stash_tab_ids: vec!["t1".to_owned()],
        character_name: None,
        include_equipment: false,
        include_inventory: false,
    }
}

impl Harness {
    /// Signed in with the backend at `backend_url`; Profiles `p1` (active)
    /// and `p2` with Snapshots: `p1` has `s1` (with item detail), `s2`
    /// (item detail dropped) and the imported `s3`, `p2` has `s4`.
    fn with_snapshots(backend_url: &str) -> Self {
        let harness = Self::with_login()
            .backend(backend_url)
            .profiles(&[wire_profile("p1"), wire_profile("p2")], "p1")
            .build();
        let pool = harness.pool();
        block_on(async {
            for (id, profile, created_at, total, body, imported) in [
                ("s1", "p1", 2_000, 10.0, Some(vec![1u8]), 0),
                ("s2", "p1", 3_000, 12.5, None, 0),
                ("s3", "p1", 1_000, 7.0, None, 1),
                ("s4", "p2", 4_000, 99.0, Some(vec![1u8]), 0),
            ] {
                sqlx::query(
                    "INSERT INTO snapshots \
                     (account_id, id, profile_id, created_at, total_value, body, imported) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                )
                .bind(ACCOUNT_ID)
                .bind(id)
                .bind(profile)
                .bind(created_at)
                .bind(total)
                .bind(body)
                .bind(imported)
                .execute(&pool)
                .await
                .unwrap();
                sqlx::query(
                    "INSERT INTO snapshot_tabs (account_id, snapshot_id, position, tab_id, name, value) \
                     VALUES (?1, ?2, 0, 't1', 'Currency', ?3)",
                )
                .bind(ACCOUNT_ID)
                .bind(id)
                .bind(total)
                .execute(&pool)
                .await
                .unwrap();
            }
        });
        block_on(profiles::push(harness.app.handle()));
        harness
    }

    fn remove_snapshots(&self, ids: &[&str]) -> Result<(), IpcError> {
        self.invoke::<RemoveSnapshots>(&RemoveSnapshotsArgs {
            snapshot_ids: ids.iter().map(|&id| id.to_owned()).collect(),
        })
    }

    /// The ids of a stored table's rows, sorted.
    fn ids(&self, query: &'static str) -> Vec<String> {
        block_on(sqlx::query_scalar(query).fetch_all(&self.pool())).unwrap()
    }

    fn snapshot_ids(&self) -> Vec<String> {
        self.ids("SELECT id FROM snapshots ORDER BY id")
    }

    /// `pending_removals` as Snapshot and Profile ids.
    pub(super) fn pending_removals(&self) -> Vec<(String, String)> {
        block_on(
            sqlx::query_as(
                "SELECT snapshot_id, profile_id FROM pending_removals ORDER BY snapshot_id",
            )
            .fetch_all(&self.pool()),
        )
        .unwrap()
    }

    fn net_worth(&self) -> NetWorth {
        self.app.state::<Store>().read(NetWorth::clone)
    }
}

fn pending(rows: &[(&str, &str)]) -> Vec<(String, String)> {
    rows.iter()
        .map(|&(s, p)| (s.to_owned(), p.to_owned()))
        .collect()
}

fn remove_call(profile_id: &str, ids: &[&str]) -> ProfileCall {
    ProfileCall::RemoveSnapshots(
        profile_id.to_owned(),
        ids.iter().map(|&id| id.to_owned()).collect(),
    )
}

#[test]
fn f12_the_list_holds_the_active_profiles_snapshots_newest_first_marked_without_detail() {
    let harness = Harness::with_snapshots(NO_BACKEND);

    let listed = harness.invoke::<ListSnapshots>(&NoArgs {}).unwrap();

    let entry = |id: &str, created_at, total_value, item_detail| SnapshotEntry {
        id: id.to_owned(),
        created_at,
        total_value,
        item_detail,
    };
    assert_eq!(
        listed,
        [
            entry("s2", 3_000, 12.5, false),
            entry("s1", 2_000, 10.0, true),
            // Imported: never had item detail.
            entry("s3", 1_000, 7.0, false),
        ]
    );
}

#[test]
fn f12_d26_removal_works_offline_and_keeps_the_pending_removal() {
    let harness = Harness::with_snapshots(NO_BACKEND);
    assert_eq!(harness.net_worth().snapshot_count, 3);

    harness.remove_snapshots(&["s2", "s3"]).unwrap();

    assert_eq!(harness.snapshot_ids(), ["s1", "s4"]);
    assert_eq!(
        harness.ids("SELECT snapshot_id FROM snapshot_tabs ORDER BY 1"),
        ["s1", "s4"]
    );
    let net_worth = harness.net_worth();
    assert_eq!(net_worth.snapshot_count, 1);
    assert_eq!(net_worth.total, 10.0);
    assert_eq!(net_worth.last_snapshot_at, Some(2_000));
    // The attempt in the background fails, as one run to its end here
    // does; the imported one was never uploaded, so only s2 waits for the
    // backend.
    block_on(snapshot_removal::send_pending(harness.app.handle()));
    assert_eq!(harness.pending_removals(), pending(&[("s2", "p1")]));
    assert_eq!(
        harness
            .invoke::<ListSnapshots>(&NoArgs {})
            .unwrap()
            .iter()
            .map(|e| e.id.as_str())
            .collect::<Vec<_>>(),
        ["s1"]
    );
}

#[test]
fn d5_removal_reaches_the_backend_at_once_when_it_can() {
    let backend = FakeBackend::start(signed_in());
    let harness = Harness::with_snapshots(&backend.url);

    harness.remove_snapshots(&["s1", "s4"]).unwrap();

    assert_eq!(harness.snapshot_ids(), ["s2", "s3"]);
    poll(|| harness.pending_removals().is_empty().then_some(()));
    assert_eq!(
        backend.profile_calls(),
        [remove_call("p1", &["s1"]), remove_call("p2", &["s4"])]
    );
}

#[test]
fn d5_d26_pending_removals_are_sent_again_after_a_successful_start_session() {
    let backend = FakeBackend::start(signed_in());
    // The attempt right after the removal and the one after it do not get
    // through.
    backend.script_profile_failures([Status::unavailable("down"), Status::unavailable("down")]);
    let harness = Harness::with_snapshots(&backend.url);

    harness.remove_snapshots(&["s1", "s2", "s4"]).unwrap();

    // Each stops at its first failure; the second runs to its end here.
    poll(|| (backend.profile_calls().len() == 1).then_some(()));
    block_on(snapshot_removal::send_pending(harness.app.handle()));
    let p1 = remove_call("p1", &["s1", "s2"]);
    assert_eq!(backend.profile_calls(), [p1.clone(), p1]);
    assert_eq!(
        harness.pending_removals(),
        pending(&[("s1", "p1"), ("s2", "p1"), ("s4", "p2")])
    );

    // A failed StartSession sends nothing.
    backend.script_start_sessions([
        Err(Status::unavailable("down")),
        session(vec![wire_profile("p1"), wire_profile("p2")], "p1"),
    ]);
    block_on(session_start::backend_part(harness.app.handle()));
    assert_eq!(backend.profile_calls().len(), 2);
    assert_eq!(harness.pending_removals().len(), 3);

    block_on(session_start::backend_part(harness.app.handle()));

    assert_eq!(
        backend.profile_calls()[2..],
        [remove_call("p1", &["s1", "s2"]), remove_call("p2", &["s4"])]
    );
    assert!(harness.pending_removals().is_empty());
    assert_eq!(harness.snapshot_ids(), ["s3"]);
}

#[test]
fn f12_removal_is_busy_while_the_operation_slot_is_held() {
    let backend = FakeBackend::start(signed_in());
    let harness = Harness::with_snapshots(&backend.url);
    let toasts = harness.events(TOAST_EVENT);
    // A running Snapshot holds the slot.
    let slot = harness.app.state::<OperationSlot>();
    let snapshot = slot.try_hold().unwrap();

    assert_eq!(harness.remove_snapshots(&["s1"]), Err(IpcError::Busy));

    let Toast {
        kind,
        message,
        description,
    } = next_toast(&toasts);
    assert_eq!(kind, NotificationKind::Error);
    assert_eq!(message.key, MessageKey::NotificationSnapshotsRemoveFailed);
    assert_eq!(description.map(|d| d.key), Some(MessageKey::ErrorBusy));
    assert_eq!(harness.snapshot_ids(), ["s1", "s2", "s3", "s4"]);
    assert!(harness.pending_removals().is_empty());
    assert!(backend.profile_calls().is_empty());

    drop(snapshot);
    harness.remove_snapshots(&["s1"]).unwrap();
    assert_eq!(harness.snapshot_ids(), ["s2", "s3", "s4"]);
}
