//! Group resilience (D22, D25, D27; client-background-work.md, in a Group,
//! backoff, Group upload queue) on the fake clock and jitter: a dead stream
//! takes the client offline, the reconnect backs off and rejoins keeping
//! the selection, the grace running out leaves the Group, `Superseded`
//! stays offline, and failed uploads in a Group retry from the queue.

use std::time::Duration;

use exilence_ce_ng_lib::clock::Clock;
use exilence_ce_ng_lib::group_connection::SILENCE_MS;
use exilence_ce_ng_lib::scheduler::{Deadline, Scheduler};
use exilence_ce_ng_lib::{group_upload_queue, session_start};
use ipc::command::{
    LeaveGroup, NoArgs, RemoveSnapshots, RemoveSnapshotsArgs, SettingsPatch, UpdateSettings,
    UpdateSettingsArgs,
};
use ipc::slices::{Group, LoginState, NotificationKind};
use ipc::{IpcError, MessageKey};
use tauri::Manager;
use tauri::async_runtime::block_on;
use tonic::{Code, Status};
use wire::v1::group_event::Event;
use wire::v1::{ErrorReason, Heartbeat, MemberCursor, StartSessionResponse, Superseded};

use super::fake_backend::{
    FakeBackend, GroupCall, ProfileCall, group_sync, refused, session, session_in_group, signed_in,
};
use super::fake_ggg::START;
use super::group::{
    BAR_ID, BAZ_ID, bar, group, join, member, net_worth, notifications, players, select,
    signed_in_harness,
};
use super::snapshot::{PROFILE_ID, Setup, ggg_with_stash, profile};
use super::{Harness, poll};

const NAME: &str = "Rich Exiles123";

/// The first reconnect wait with `r` = 1: `min(60 s, 5 s × 2⁰)`.
const FIRST_WAIT: i64 = 5_000;

#[track_caller]
fn wait_offline(harness: &Harness, offline: bool) {
    poll(|| (harness.session_slice().offline == offline).then_some(()));
}

/// The newest notification's kind and message.
fn newest(harness: &Harness) -> (NotificationKind, MessageKey) {
    let newest = &notifications(harness).newest[0];
    (newest.kind, newest.message.key)
}

#[test]
fn d22_75_s_without_any_event_take_the_client_offline_until_it_reconnects() {
    let backend = FakeBackend::start(signed_in());
    let harness = signed_in_harness(&backend);
    join(&harness, NAME, "").unwrap();
    assert_eq!(
        harness.due(Deadline::GroupStreamSilence),
        Some(START + SILENCE_MS)
    );

    // Any event, the heartbeat included, keeps the stream alive.
    harness.clock().advance(60_000);
    assert!(backend.send_group_event(Event::Heartbeat(Heartbeat {})));
    let silent_at = START + 60_000 + SILENCE_MS;
    poll(|| (harness.due(Deadline::GroupStreamSilence) == Some(silent_at)).then_some(()));
    harness.clock().advance(SILENCE_MS - 1);
    assert_eq!(harness.tick(), []);
    assert!(!harness.session_slice().offline);

    harness.clock().advance(1);
    assert_eq!(harness.tick(), [Deadline::GroupStreamSilence]);
    let session = harness.session_slice();
    assert!(session.offline && !session.superseded, "{session:?}");
    assert_eq!(
        group(&harness).name.as_deref(),
        Some(NAME),
        "still in the Group"
    );
    assert_eq!(
        harness.due(Deadline::GroupReconnect),
        Some(silent_at + FIRST_WAIT)
    );

    backend.script_start_sessions([session_in_group(NAME)]);
    harness.clock().advance(FIRST_WAIT);
    assert_eq!(harness.tick(), [Deadline::GroupReconnect]);
    wait_offline(&harness, false);
    assert_eq!(
        backend.group_calls(),
        [
            GroupCall::Join {
                name: NAME.to_owned(),
                password: String::new()
            },
            GroupCall::Open(0),
            GroupCall::Open(0),
        ]
    );
    assert_eq!(
        harness.due(Deadline::GroupStreamSilence),
        Some(silent_at + FIRST_WAIT + SILENCE_MS),
        "the new stream is watched"
    );
}

#[test]
fn d22_a_stream_error_or_an_end_without_superseded_takes_the_client_offline_a_leave_does_not() {
    let ends: [fn(&FakeBackend); 2] = [FakeBackend::end_group_stream, |backend| {
        backend.fail_group_stream(Status::unavailable("connection reset"))
    }];
    for end in ends {
        let backend = FakeBackend::start(signed_in());
        let harness = signed_in_harness(&backend);
        join(&harness, NAME, "").unwrap();

        end(&backend);

        wait_offline(&harness, true);
        assert_eq!(
            harness.due(Deadline::GroupReconnect),
            Some(START + FIRST_WAIT)
        );
        assert_eq!(harness.due(Deadline::GroupStreamSilence), None);
        assert_eq!(group(&harness).name.as_deref(), Some(NAME));
        assert_eq!(
            newest(&harness),
            (
                NotificationKind::Success,
                MessageKey::NotificationGroupJoined
            ),
            "no notification, no toast"
        );
    }

    // Leaving ends the stream on purpose.
    let backend = FakeBackend::start(signed_in());
    let harness = signed_in_harness(&backend);
    join(&harness, NAME, "").unwrap();
    harness.invoke::<LeaveGroup>(&NoArgs {}).unwrap();
    // The leave forgot the Group before it answered: an end of its stream
    // read since or later takes nothing offline.
    assert!(!harness.session_slice().offline);
    assert_eq!(harness.due(Deadline::GroupReconnect), None);
    assert_eq!(harness.due(Deadline::GroupStreamSilence), None);
}

#[test]
fn d22_the_reconnect_backs_off_to_60_s_and_rejoins_with_cursors_keeping_the_selection() {
    let backend = FakeBackend::start(signed_in());
    backend.script_group_syncs([Ok(group_sync(NAME, vec![bar(Vec::new())]))]);
    let harness = signed_in_harness(&backend);
    harness
        .invoke::<UpdateSettings>(&UpdateSettingsArgs {
            patch: SettingsPatch {
                auto_snapshot: Some(true),
                ..SettingsPatch::default()
            },
        })
        .unwrap();
    join(&harness, NAME, "").unwrap();
    select(&harness, BAR_ID, false);

    backend.end_group_stream();
    wait_offline(&harness, true);
    let down = || Err(Status::unavailable("backend down"));
    backend.script_start_sessions([down(), down(), down(), down(), session_in_group(NAME)]);
    let baz = member(BAZ_ID, "Baz#9012", Vec::new());
    backend.script_group_syncs([Ok(group_sync(NAME, vec![bar(Vec::new()), baz]))]);

    // min(60 s, 5 s × 2ⁿ) × r before attempt n+1; `r` is drawn when an
    // attempt fails: 0.5 after the first, 1 after the others.
    let mut now = START;
    for (attempt, wait) in [5_000, 5_000, 20_000, 40_000, 60_000]
        .into_iter()
        .enumerate()
    {
        assert_eq!(
            poll(|| harness.due(Deadline::GroupReconnect)),
            now + wait,
            "before attempt {}",
            attempt + 1
        );
        harness
            .fakes
            .jitter
            .set(if attempt == 0 { 0.5 } else { 1.0 });
        harness.clock().advance(wait);
        now += wait;
        assert_eq!(harness.tick(), [Deadline::GroupReconnect]);
        poll(|| (backend.start_session_count() == attempt + 1).then_some(()));
        assert!(
            harness.session_slice().offline || attempt == 4,
            "offline while it fails"
        );
    }

    wait_offline(&harness, false);
    assert_eq!(
        backend.open_cursors().last().unwrap(),
        &[MemberCursor {
            account_id: BAR_ID.to_owned(),
            newest_snapshot_id: "bar-1".to_owned(),
        }],
        "the newest Snapshot held per member"
    );
    poll(|| (players(&harness).len() == 3).then_some(()));
    assert_eq!(
        players(&harness),
        [
            (super::fake_backend::ACCOUNT_NAME.to_owned(), true),
            ("Bar#5678".to_owned(), false),
            ("Baz#9012".to_owned(), false),
        ],
        "the selection is kept; who joined meanwhile appears unselected"
    );
    // Auto-snapshot at once (F4), once the reconnect is through.
    poll(|| (harness.due(Deadline::AutoSnapshot) == Some(now)).then_some(()));
    assert_eq!(
        newest(&harness),
        (
            NotificationKind::Success,
            MessageKey::NotificationGroupJoined
        ),
        "the failed attempts stay silent"
    );
}

#[test]
fn d22_without_group_name_or_with_not_in_group_the_grace_ran_out_and_the_client_is_solo() {
    for not_in_group in [false, true] {
        let backend = FakeBackend::start(signed_in());
        let harness = signed_in_harness(&backend);
        join(&harness, NAME, "").unwrap();
        backend.end_group_stream();
        wait_offline(&harness, true);
        if not_in_group {
            backend.script_start_sessions([session_in_group(NAME)]);
            backend.script_group_syncs([Err(refused(
                Code::FailedPrecondition,
                ErrorReason::NotInGroup,
            ))]);
        }
        // Unscripted, `StartSession` has no `group_name`.

        harness.clock().advance(FIRST_WAIT);
        assert_eq!(harness.tick(), [Deadline::GroupReconnect]);

        poll(|| (newest(&harness).1 == MessageKey::ErrorNotInGroup).then_some(()));
        assert_eq!(newest(&harness).0, NotificationKind::Warning);
        assert_eq!(group(&harness), Group::default());
        assert!(!net_worth(&harness).group_combined, "solo again");
        assert!(!harness.session_slice().offline);
        assert_eq!(harness.due(Deadline::GroupReconnect), None);
        assert_eq!(harness.due(Deadline::GroupStreamSilence), None);
        let opens = backend
            .group_calls()
            .iter()
            .filter(|c| matches!(c, GroupCall::Open(_)))
            .count();
        assert_eq!(opens, if not_in_group { 2 } else { 1 });
    }
}

#[test]
fn d22_d25_superseded_closes_the_group_and_stays_offline_until_restarted() {
    for by_event in [true, false] {
        let backend = FakeBackend::start(signed_in());
        let harness = signed_in_harness(&backend);
        join(&harness, NAME, "").unwrap();

        if by_event {
            assert!(backend.send_group_event(Event::Superseded(Superseded {})));
        } else {
            backend.fail_group_stream(refused(Code::Unauthenticated, ErrorReason::Superseded));
        }

        // `superseded` and `offline` go out in two `session` pushes.
        poll(|| harness.session_slice().superseded.then_some(()));
        poll(|| harness.session_slice().offline.then_some(()));
        poll(|| (group(&harness) == Group::default()).then_some(()));
        assert_eq!(
            newest(&harness),
            (NotificationKind::Warning, MessageKey::ErrorSuperseded)
        );
        assert_eq!(harness.due(Deadline::GroupReconnect), None);
        assert_eq!(harness.due(Deadline::GroupStreamSilence), None);
        harness.clock().advance(10 * 60_000);
        assert_eq!(harness.tick(), [], "nothing reconnects");
        assert_eq!(backend.start_session_count(), 0);
        assert_eq!(join(&harness, NAME, ""), Err(IpcError::Superseded));
        assert!(harness.session_slice().offline);
    }
}

#[test]
fn d22_session_expired_on_a_reconnect_sends_the_user_to_login() {
    let backend = FakeBackend::start(signed_in());
    let harness = signed_in_harness(&backend);
    join(&harness, NAME, "").unwrap();
    backend.end_group_stream();
    wait_offline(&harness, true);
    backend.script_start_sessions([Err(refused(
        Code::Unauthenticated,
        ErrorReason::SessionExpired,
    ))]);

    harness.clock().advance(FIRST_WAIT);
    assert_eq!(harness.tick(), [Deadline::GroupReconnect]);

    poll(|| (harness.session_slice().login == LoginState::SignedOut).then_some(()));
    assert!(!harness.session_slice().offline);
    // The closed stream's `group` push runs in the background.
    poll(|| (group(&harness) == Group::default()).then_some(()));
    assert_eq!(harness.due(Deadline::GroupReconnect), None);
}

#[test]
fn d22_a_failed_rejoin_at_the_session_start_reconnects_as_on_a_first_join() {
    let backend = FakeBackend::start(signed_in());
    backend.script_start_sessions([session_in_group(NAME), session_in_group(NAME)]);
    backend.script_group_syncs([
        Err(Status::unavailable("down")),
        Ok(group_sync(NAME, vec![bar(Vec::new())])),
    ]);
    let harness = signed_in_harness(&backend);

    block_on(session_start::run(harness.app.handle()));

    wait_offline(&harness, true);
    assert_eq!(
        harness.due(Deadline::GroupReconnect),
        Some(START + FIRST_WAIT)
    );
    harness.clock().advance(FIRST_WAIT);
    assert!(harness.tick().contains(&Deadline::GroupReconnect));
    wait_offline(&harness, false);
    poll(|| (players(&harness).len() == 2).then_some(()));
    assert!(players(&harness).iter().all(|(_, selected)| *selected));
    assert_eq!(
        backend.open_cursors(),
        [Vec::new(), Vec::new()],
        "nothing held: no cursors"
    );
}

/// The `StartSession` answer of [`Setup`]'s account in the Group.
fn setup_session_in_group() -> Result<StartSessionResponse, Status> {
    let mut answer = session(vec![profile()], PROFILE_ID)?;
    answer.group_name = Some(NAME.to_owned());
    Ok(answer)
}

/// Ready for Snapshots, in the Group.
fn setup_in_group(backend: &FakeBackend) -> Setup {
    let setup = Setup::ready(Some(backend), ggg_with_stash(Duration::ZERO));
    join(&setup.harness, NAME, "").unwrap();
    setup
}

/// Takes a Snapshot and waits for its upload attempt: the `uploads`th. The
/// attempt has ended, failure handling included: the Snapshot uploads
/// before it gives the operation slot back.
#[track_caller]
fn take_and_upload(setup: &Setup, backend: &FakeBackend, uploads: usize) -> String {
    setup.take_and_wait();
    poll(|| (backend.uploads().len() == uploads).then_some(()));
    setup.snapshots()[0].0.clone()
}

/// The Snapshot ids uploaded so far, in order.
fn uploaded(backend: &FakeBackend) -> Vec<String> {
    backend
        .uploads()
        .into_iter()
        .map(|u| u.snapshot.unwrap().id)
        .collect()
}

#[test]
fn d27_a_failed_upload_in_a_group_retries_after_1_to_10_s_then_with_the_backoff() {
    let backend = FakeBackend::start(signed_in());
    let setup = setup_in_group(&backend);
    let harness = &setup.harness;
    backend.script_upload_failures([
        Status::unavailable("down"),
        Status::deadline_exceeded("slow"),
    ]);
    harness.fakes.jitter.set(0.5);

    let id = take_and_upload(&setup, &backend, 1);

    let failed_at = harness.clock().now();
    assert_eq!(
        poll(|| harness.due(Deadline::GroupUploadRetry)),
        failed_at + 1_000,
        "a random 1–10 s, the shortest with r = 0.5"
    );
    harness.fakes.jitter.set(1.0);
    harness.clock().advance(999);
    assert!(!harness.tick().contains(&Deadline::GroupUploadRetry));
    harness.clock().advance(1);
    assert!(harness.tick().contains(&Deadline::GroupUploadRetry));
    poll(|| (backend.uploads().len() == 2).then_some(()));
    assert_eq!(
        poll(|| harness.due(Deadline::GroupUploadRetry)),
        failed_at + 1_000 + 5_000,
        "then the backoff: min(60 s, 5 s × 2⁰) × 1"
    );

    run_upload_retry(harness);
    assert_eq!(uploaded(&backend), [id.clone(), id.clone(), id]);
    assert_eq!(harness.due(Deadline::GroupUploadRetry), None, "uploaded");
    assert!(!harness.session_slice().offline, "uploads never go offline");
}

#[test]
fn d27_the_queue_goes_out_right_after_a_rejoin() {
    let backend = FakeBackend::start(signed_in());
    let setup = setup_in_group(&backend);
    let harness = &setup.harness;
    backend.script_upload_failures([Status::unavailable("down")]);
    let id = take_and_upload(&setup, &backend, 1);
    let failed_at = harness.clock().now();
    assert_eq!(
        poll(|| harness.due(Deadline::GroupUploadRetry)),
        failed_at + 10_000,
        "r = 1: 10 s"
    );

    backend.end_group_stream();
    wait_offline(harness, true);
    backend.script_start_sessions([setup_session_in_group()]);
    harness.clock().advance(FIRST_WAIT);
    let fired = harness.tick();
    assert!(fired.contains(&Deadline::GroupReconnect), "{fired:?}");
    assert!(!fired.contains(&Deadline::GroupUploadRetry), "{fired:?}");

    poll(|| (backend.uploads().len() == 2).then_some(()));
    assert_eq!(uploaded(&backend), [id.clone(), id]);
    wait_offline(harness, false);
    assert_eq!(harness.due(Deadline::GroupUploadRetry), None);
}

/// Moves the clock to the pending Group upload retry.
#[track_caller]
fn upload_retry_due(harness: &Harness) {
    let at = poll(|| harness.due(Deadline::GroupUploadRetry));
    harness.clock().advance(at - harness.clock().now());
}

/// Fires the pending Group upload retry: its drain runs in the background.
#[track_caller]
fn fire_upload_retry(harness: &Harness) {
    upload_retry_due(harness);
    assert!(harness.tick().contains(&Deadline::GroupUploadRetry));
}

/// Runs the pending Group upload retry as `tick` starts it, its drain to
/// the end: each attempt answered and handled, the next retry set.
#[track_caller]
fn run_upload_retry(harness: &Harness) {
    upload_retry_due(harness);
    harness
        .app
        .state::<Scheduler>()
        .clear(Deadline::GroupUploadRetry);
    block_on(group_upload_queue::drain(harness.app.handle()));
}

#[test]
fn d27_a_queued_upload_leaves_on_refusal_removal_leave_and_superseded_and_solo_never_queues() {
    let backend = FakeBackend::start(signed_in());
    let setup = setup_in_group(&backend);
    let harness = &setup.harness;
    let unavailable = || Status::unavailable("down");
    let invalid = || refused(Code::InvalidArgument, ErrorReason::InvalidArgument);

    // A refusal is not retried.
    backend.script_upload_failures([invalid()]);
    take_and_upload(&setup, &backend, 1);
    assert_eq!(harness.due(Deadline::GroupUploadRetry), None);

    // Removed locally.
    backend.script_upload_failures([unavailable()]);
    let id = take_and_upload(&setup, &backend, 2);
    poll(|| harness.due(Deadline::GroupUploadRetry));
    harness
        .invoke::<RemoveSnapshots>(&RemoveSnapshotsArgs {
            snapshot_ids: vec![id],
        })
        .unwrap();
    assert_eq!(harness.due(Deadline::GroupUploadRetry), None);

    // A retry refused.
    backend.script_upload_failures([unavailable(), invalid()]);
    take_and_upload(&setup, &backend, 3);
    run_upload_retry(harness);
    assert_eq!(backend.uploads().len(), 4);
    assert_eq!(harness.due(Deadline::GroupUploadRetry), None);

    // Leaving the Group.
    backend.script_upload_failures([unavailable()]);
    take_and_upload(&setup, &backend, 5);
    poll(|| harness.due(Deadline::GroupUploadRetry));
    harness.invoke::<LeaveGroup>(&NoArgs {}).unwrap();
    assert_eq!(harness.due(Deadline::GroupUploadRetry), None);

    // Solo: never queued, also not on a later join.
    backend.script_upload_failures([unavailable()]);
    take_and_upload(&setup, &backend, 6);
    assert_eq!(harness.due(Deadline::GroupUploadRetry), None);
    join(harness, NAME, "").unwrap();
    assert_eq!(harness.due(Deadline::GroupUploadRetry), None);

    // Superseded.
    backend.script_upload_failures([unavailable()]);
    take_and_upload(&setup, &backend, 7);
    poll(|| harness.due(Deadline::GroupUploadRetry));
    assert!(backend.send_group_event(Event::Superseded(Superseded {})));
    // `superseded` goes out before the queue is cleared: wait for the clear.
    poll(|| {
        harness
            .due(Deadline::GroupUploadRetry)
            .is_none()
            .then_some(())
    });
    assert!(harness.session_slice().superseded);

    harness.clock().advance(60 * 60_000);
    assert!(!harness.tick().contains(&Deadline::GroupUploadRetry));
    block_on(group_upload_queue::drain(harness.app.handle()));
    assert_eq!(backend.uploads().len(), 7, "nothing sent again");
}

#[test]
fn d5_d27_a_removal_while_its_queued_upload_is_in_flight_leaves_no_backend_copy() {
    let backend = FakeBackend::start(signed_in());
    let setup = setup_in_group(&backend);
    let harness = &setup.harness;
    backend.script_upload_failures([Status::unavailable("down")]);
    let id = take_and_upload(&setup, &backend, 1);
    let held = backend.hold_next_upload();
    fire_upload_retry(harness);
    poll(|| (backend.uploads().len() == 2).then_some(()));

    harness
        .invoke::<RemoveSnapshots>(&RemoveSnapshotsArgs {
            snapshot_ids: vec![id.clone()],
        })
        .unwrap();
    // The removal reaches the backend before the upload lands.
    let removal = ProfileCall::RemoveSnapshots(PROFILE_ID.to_owned(), vec![id.clone()]);
    poll(|| backend.profile_calls().contains(&removal).then_some(()));
    held.release();

    poll(|| harness.pending_removals().is_empty().then_some(()));
    assert_eq!(uploaded(&backend), [id.clone(), id]);
    assert!(
        backend.copies().is_empty(),
        "the upload brought the removed Snapshot back: {:?}",
        backend.copies()
    );
}

#[test]
fn d27_a_removal_during_a_send_never_starts_a_second_send() {
    let backend = FakeBackend::start(signed_in());
    let setup = setup_in_group(&backend);
    let harness = &setup.harness;
    backend.script_upload_failures([Status::unavailable("down"), Status::unavailable("down")]);
    let sending = take_and_upload(&setup, &backend, 1);
    let waiting = take_and_upload(&setup, &backend, 2);
    let held = backend.hold_next_upload();
    fire_upload_retry(harness);
    poll(|| (backend.uploads().len() == 3).then_some(()));

    harness
        .invoke::<RemoveSnapshots>(&RemoveSnapshotsArgs {
            snapshot_ids: vec![sending.clone()],
        })
        .unwrap();
    // A retry or a rejoin drains meanwhile.
    block_on(group_upload_queue::drain(harness.app.handle()));

    assert_eq!(backend.uploads().len(), 3, "a second send started");
    held.release();
    poll(|| (backend.uploads().len() == 4).then_some(()));
    assert_eq!(
        uploaded(&backend),
        [sending.clone(), waiting.clone(), sending, waiting]
    );
}

#[test]
fn d27_clearing_the_queue_during_a_send_never_starts_a_second_send() {
    let backend = FakeBackend::start(signed_in());
    let setup = setup_in_group(&backend);
    let harness = &setup.harness;
    backend.script_upload_failures([Status::unavailable("down")]);
    let sending = take_and_upload(&setup, &backend, 1);
    let held = backend.hold_next_upload();
    fire_upload_retry(harness);
    poll(|| (backend.uploads().len() == 2).then_some(()));

    // Leaving clears the queue; joined again, a new failed upload queues.
    harness.invoke::<LeaveGroup>(&NoArgs {}).unwrap();
    join(harness, NAME, "").unwrap();
    backend.script_upload_failures([Status::unavailable("down")]);
    let waiting = take_and_upload(&setup, &backend, 3);
    // A retry or a rejoin drains meanwhile.
    block_on(group_upload_queue::drain(harness.app.handle()));

    assert_eq!(backend.uploads().len(), 3, "a second send started");
    held.release();
    poll(|| (backend.uploads().len() == 4).then_some(()));
    assert_eq!(
        uploaded(&backend),
        [sending.clone(), sending, waiting.clone(), waiting]
    );
}
