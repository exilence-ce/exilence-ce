//! Sign-out (F1, D19; client-background-work.md, sign-out), against the fake
//! backend.

use std::time::Duration;

use exilence_ce_ng_lib::scheduler::{Deadline, Scheduler};
use exilence_ce_ng_lib::state::Store;
use exilence_ce_ng_lib::{auth, slot};
use ipc::command::{NoArgs, SignOut};
use ipc::slices::{LoginState, NotificationKind, Status};
use ipc::{IpcError, MessageKey};
use tauri::Manager;
use tauri::async_runtime::block_on;
use wiremock::MockServer;

use super::fake_backend::{
    ACCESS_TOKEN, ACCESS_TOKEN_EXPIRES_AT, ACCOUNT_ID, ACCOUNT_NAME, FakeBackend, SESSION_TOKEN,
    session, signed_in,
};
use super::session_start::{ggg_accepting, logins, profiles};
use super::{Harness, poll};

/// Every row of the account's local history, as text.
fn history(harness: &Harness) -> Vec<String> {
    block_on(
        sqlx::query_scalar(
            "SELECT 'account ' || id || ' ' || name FROM accounts \
             UNION ALL SELECT 'profile ' || id FROM profiles WHERE account_id = ?1 \
             UNION ALL SELECT 'snapshot ' || id || ' ' || total_value FROM snapshots \
             WHERE account_id = ?1 ORDER BY 1",
        )
        .bind(ACCOUNT_ID)
        .fetch_all(&harness.pool()),
    )
    .unwrap()
}

/// Signed in with a started session against `ggg`: the account has Profile
/// `p1` and one Snapshot of it.
fn signed_in_with_history(backend: &FakeBackend, ggg: &MockServer) -> Harness {
    backend.script_start_sessions([session(profiles(&["p1"]), "p1")]);
    let harness = Harness::launched(&backend.url, ggg, ACCESS_TOKEN_EXPIRES_AT);
    harness.start_session();
    block_on(
        sqlx::query(
            "INSERT INTO snapshots (account_id, id, profile_id, created_at, total_value) \
             VALUES (?1, 's1', 'p1', 1, 42.5)",
        )
        .bind(ACCOUNT_ID)
        .execute(&harness.pool()),
    )
    .unwrap();
    assert_eq!(history(&harness).len(), 3);
    harness
}

fn status(harness: &Harness) -> Status {
    harness.app.state::<Store>().read(Status::clone)
}

#[test]
fn f1_d19_sign_out_stops_background_work_calls_sign_out_then_forgets_the_login() {
    let backend = FakeBackend::start(signed_in());
    backend.script.lock().unwrap().sign_out_delay = Duration::from_millis(300);
    let ggg = ggg_accepting(ACCESS_TOKEN);
    let harness = signed_in_with_history(&backend, &ggg);
    let before = history(&harness);
    let scheduler = harness.app.state::<Scheduler>();
    scheduler.set_in(Deadline::GggSessionRetry, 45_000);
    scheduler.set_in(Deadline::BackendSessionRetry, 5_000);

    let handle = harness.app.handle().clone();
    let signing_out = tauri::async_runtime::spawn(async move { auth::sign_out(&handle).await });

    // While `SignOut` runs: background work stopped, the Login still stored,
    // still signed in, and the slot taken.
    poll(|| (!backend.sign_outs().is_empty()).then_some(()));
    assert_eq!(harness.due(Deadline::GggSessionRetry), None);
    assert_eq!(harness.due(Deadline::BackendSessionRetry), None);
    assert_eq!(logins(&harness), 1);
    assert_eq!(harness.session_slice().login, LoginState::SignedIn);
    assert!(status(&harness).operation_running);
    assert_eq!(
        harness.invoke::<SignOut>(&NoArgs {}).unwrap_err(),
        IpcError::Busy
    );

    block_on(signing_out).unwrap().unwrap();
    assert_eq!(
        backend.sign_outs(),
        [Some(format!("Bearer {SESSION_TOKEN}"))],
        "this Login's session token"
    );
    assert_eq!(logins(&harness), 0);
    let session = harness.session_slice();
    assert_eq!(session.login, LoginState::SignedOut);
    assert_eq!(session.account_name, None);
    assert!(!status(&harness).operation_running);
    let newest = harness.notifications().newest[0].clone();
    assert_eq!(
        (newest.kind, newest.message.key),
        (NotificationKind::Success, MessageKey::NotificationSignedOut)
    );
    assert_eq!(history(&harness), before, "the local history stays");
    assert!(before[0].contains(ACCOUNT_NAME));

    // Signed out for good: a restart shows the login screen.
    let harness = harness.restart();
    assert_eq!(harness.session_slice().login, LoginState::SignedOut);
}

#[test]
fn d19_sign_out_completes_when_the_backend_call_fails() {
    let backend = FakeBackend::start(signed_in());
    backend
        .script
        .lock()
        .unwrap()
        .sign_outs
        .push_back(Err(tonic::Status::unavailable("down")));
    let ggg = ggg_accepting(ACCESS_TOKEN);
    let harness = signed_in_with_history(&backend, &ggg);
    let before = history(&harness);

    harness.invoke::<SignOut>(&NoArgs {}).unwrap();

    assert_eq!(backend.sign_outs().len(), 1);
    assert_eq!(logins(&harness), 0);
    assert_eq!(harness.session_slice().login, LoginState::SignedOut);
    assert_eq!(
        harness.notifications().newest[0].message.key,
        MessageKey::NotificationSignedOut
    );
    assert_eq!(history(&harness), before);
}

#[test]
fn f1_sign_out_is_refused_while_the_operation_slot_is_taken() {
    let backend = FakeBackend::start(signed_in());
    let ggg = ggg_accepting(ACCESS_TOKEN);
    let harness = signed_in_with_history(&backend, &ggg);
    let held = slot::try_hold(harness.app.handle()).unwrap();
    assert!(
        status(&harness).operation_running,
        "the UI disables Sign out"
    );

    assert_eq!(
        harness.invoke::<SignOut>(&NoArgs {}).unwrap_err(),
        IpcError::Busy
    );
    assert!(backend.sign_outs().is_empty());
    assert_eq!(logins(&harness), 1);
    assert_eq!(harness.session_slice().login, LoginState::SignedIn);

    drop(held);
    assert!(!status(&harness).operation_running);
    harness.invoke::<SignOut>(&NoArgs {}).unwrap();
    assert_eq!(harness.session_slice().login, LoginState::SignedOut);
}
