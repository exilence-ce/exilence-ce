//! Auto-snapshot (F4): every due-time rule of client-background-work.md, the
//! 10 s retry, and that an unreachable backend does not stop it (D26), with
//! the injected clock.

use std::time::Duration;

use exilence_ce_ng_lib::clock::Clock;
use exilence_ce_ng_lib::scheduler::{self, Deadline, Scheduler};
use exilence_ce_ng_lib::{auth, auto_snapshot, session_start};
use ipc::command::{CancelSnapshot, NoArgs, SettingsPatch};
use ipc::slices::Status;
use tauri::Manager;
use tauri::async_runtime::block_on;

use super::snapshot::{Setup, ggg_with_stash};
use super::{Harness, poll, update_settings};

/// The default interval: 20 minutes.
const INTERVAL_MS: i64 = 20 * 60_000;

fn auto(on: bool) -> SettingsPatch {
    SettingsPatch {
        auto_snapshot: Some(on),
        ..SettingsPatch::default()
    }
}

fn minutes(minutes: u32) -> SettingsPatch {
    SettingsPatch {
        auto_snapshot_minutes: Some(minutes),
        ..SettingsPatch::default()
    }
}

fn due(harness: &Harness) -> Option<i64> {
    harness
        .app
        .state::<Scheduler>()
        .due_at(Deadline::AutoSnapshot)
}

fn now(harness: &Harness) -> i64 {
    harness.fakes.clock.now()
}

/// Makes the due time come and fires it.
fn fire(harness: &Harness) {
    let at = due(harness).expect("a due time");
    harness.fakes.clock.advance((at - now(harness)).max(0));
    assert!(scheduler::tick(harness.app.handle()).contains(&Deadline::AutoSnapshot));
}

#[test]
fn f4_d26_session_ready_with_auto_on_is_due_now_and_snapshots_without_the_backend() {
    // No backend listens: the session start's backend part keeps failing.
    let setup = Setup::ready_with(None, ggg_with_stash(Duration::ZERO), |harness| {
        update_settings(harness, auto(true));
    });
    let harness = &setup.harness;
    poll(|| due(harness).filter(|&at| at <= now(harness)));

    fire(harness);
    poll(|| (setup.snapshots().len() == 1).then_some(()));
    setup.wait_until_ended();
    // Finished: due one interval later.
    assert_eq!(due(harness), Some(now(harness) + INTERVAL_MS));

    fire(harness);
    poll(|| (setup.snapshots().len() == 2).then_some(()));
    setup.wait_until_ended();
    assert_eq!(due(harness), Some(now(harness) + INTERVAL_MS));
}

#[test]
fn f4_session_ready_with_auto_off_is_not_due() {
    let setup = Setup::ready(None, ggg_with_stash(Duration::ZERO));
    assert_eq!(due(&setup.harness), None);
}

#[test]
fn f4_the_silent_backend_session_retry_is_not_a_session_start() {
    let setup = Setup::ready(None, ggg_with_stash(Duration::ZERO));
    let harness = &setup.harness;
    update_settings(harness, auto(true));
    let at = due(harness).unwrap();
    block_on(session_start::backend_part(harness.app.handle()));
    assert_eq!(due(harness), Some(at), "unchanged");
}

#[test]
fn f4_group_reconnect_with_auto_on_is_due_now() {
    let setup = Setup::ready(None, ggg_with_stash(Duration::ZERO));
    let harness = &setup.harness;
    auto_snapshot::group_reconnected(harness.app.handle());
    assert_eq!(due(harness), None, "auto off");

    update_settings(harness, auto(true));
    auto_snapshot::group_reconnected(harness.app.handle());
    assert_eq!(due(harness), Some(now(harness)));
}

#[test]
fn f4_auto_on_and_interval_changes_are_due_an_interval_later_and_off_clears() {
    let setup = Setup::ready(None, ggg_with_stash(Duration::ZERO));
    let harness = &setup.harness;
    update_settings(harness, minutes(30));
    assert_eq!(due(harness), None, "interval changed while off");

    update_settings(harness, auto(true));
    assert_eq!(due(harness), Some(now(harness) + 30 * 60_000));

    harness.fakes.clock.advance(60_000);
    update_settings(harness, minutes(5));
    assert_eq!(due(harness), Some(now(harness) + 5 * 60_000));

    harness.fakes.clock.advance(60_000);
    update_settings(harness, auto(true));
    assert_eq!(
        due(harness),
        Some(now(harness) - 60_000 + 5 * 60_000),
        "neither turned on nor changed"
    );

    update_settings(harness, auto(false));
    assert_eq!(due(harness), None);
}

#[test]
fn f4_sign_out_clears_the_due_time() {
    let setup = Setup::ready(None, ggg_with_stash(Duration::ZERO));
    let harness = &setup.harness;
    update_settings(harness, auto(true));
    assert!(due(harness).is_some());
    auth::stop_background_work(harness.app.handle());
    assert_eq!(due(harness), None);
}

#[test]
fn f4_a_manual_snapshot_finished_or_failed_is_due_an_interval_later() {
    let setup = Setup::ready(None, ggg_with_stash(Duration::ZERO));
    let harness = &setup.harness;
    setup.take().unwrap();
    setup.wait_until_ended();
    assert_eq!(due(harness), None, "auto off");

    update_settings(harness, auto(true));
    harness.fakes.clock.advance(60_000);
    setup.take().unwrap();
    setup.wait_until_ended();
    assert_eq!(setup.snapshots().len(), 2);
    assert_eq!(due(harness), Some(now(harness) + INTERVAL_MS), "finished");

    // GGG answers 404 from now on: the Snapshot fails.
    block_on(setup.ggg.reset());
    harness.fakes.clock.advance(60_000);
    setup.take().unwrap();
    setup.wait_until_ended();
    assert_eq!(setup.snapshots().len(), 2);
    assert_eq!(due(harness), Some(now(harness) + INTERVAL_MS), "failed");
}

#[test]
fn f4_a_cancelled_snapshot_is_due_an_interval_later() {
    let setup = Setup::ready(None, ggg_with_stash(Duration::from_secs(30)));
    let harness = &setup.harness;
    update_settings(harness, auto(true));
    harness.fakes.clock.advance(60_000);
    setup.take().unwrap();
    harness.invoke::<CancelSnapshot>(&NoArgs {}).unwrap();
    setup.wait_until_ended();
    assert!(setup.snapshots().is_empty());
    assert_eq!(due(harness), Some(now(harness) + INTERVAL_MS));
}

#[test]
fn f4_a_due_snapshot_that_cannot_start_retries_every_10_s() {
    // Another Snapshot runs: the slot is taken.
    let setup = Setup::ready(None, ggg_with_stash(Duration::from_secs(30)));
    let harness = &setup.harness;
    update_settings(harness, auto(true));
    setup.take().unwrap();
    auto_snapshot::group_reconnected(harness.app.handle());

    fire(harness);
    let retry = poll(|| due(harness).filter(|&at| at > now(harness)));
    assert_eq!(retry, now(harness) + auto_snapshot::RETRY_MS);
    assert!(
        setup.read(|s: &Status| s.snapshot_running),
        "still the first"
    );

    fire(harness);
    let retry = poll(|| due(harness).filter(|&at| at > now(harness)));
    assert_eq!(retry, now(harness) + 10_000);
}

#[test]
fn f4_d26_a_due_snapshot_before_the_session_is_ready_retries_every_10_s() {
    // Signed in at launch, the session not started: not ready. No backend.
    let harness = Harness::with_login().build();
    update_settings(&harness, auto(true));
    assert_eq!(due(&harness), Some(now(&harness) + INTERVAL_MS));

    for _ in 0..3 {
        fire(&harness);
        let retry = poll(|| due(&harness).filter(|&at| at > now(&harness)));
        assert_eq!(retry, now(&harness) + 10_000);
    }
}
