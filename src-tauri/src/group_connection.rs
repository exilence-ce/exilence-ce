//! The Group connection (client-background-work.md, in a Group; D22, D25).
//! The Group stream is dead after [`SILENCE_MS`] without any event, the
//! heartbeat included, on a stream error, or when it ends without
//! `Superseded`. The client is then offline (`session.offline`: "Not
//! connected to server" in the toolbar, F18) and reconnects with the
//! [backoff](crate::scheduler::backoff_ms). Each attempt: the token check
//! (D6), `StartSession`, its answer applied, the pending Snapshot removals
//! sent again (D5), `OpenGroupStream` with the newest Snapshot held per
//! member as cursors, keeping the player selection (F13), the Group upload
//! queue drained (D27); then the client is online and auto-snapshot is due
//! at once (F4). GGG data and prices are not reloaded.
//!
//! Without `group_name`, or with `NOT_IN_GROUP` on the stream open, the
//! grace ran out (D22): the client has left the Group, notes it and goes on
//! solo. `Superseded`, as an event or a status, leaves the client offline
//! until restarted with its Group closed and the upload queue cleared;
//! `SESSION_EXPIRED` sends the user to login.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use ipc::slices::{NotificationKind, Session};
use ipc::{BackendReason, IpcError, MessageKey};
use tauri::{AppHandle, Manager, Runtime};

use crate::auth::{self, TokenCheck};
use crate::backend::Backend;
use crate::scheduler::{Deadline, Scheduler};
use crate::state::{self, Store};
use crate::{auto_snapshot, group, group_upload_queue, internal, session_start, snapshot_removal};

/// How long the Group stream may stay silent before it counts as dead: two
/// 30 s heartbeats and some (ADR 0004).
pub const SILENCE_MS: i64 = 75_000;

/// The Group connection's state, managed by the app; memory only.
#[derive(Default)]
pub struct GroupConnection {
    /// The stream was lost and the reconnect loop runs.
    lost: AtomicBool,
    /// Failed reconnect attempts since the loss: n of the backoff.
    failed: AtomicU32,
}

/// Whether the reconnect loop runs: the account is in a Group without its
/// stream.
pub fn reconnecting<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.state::<GroupConnection>().lost.load(Ordering::SeqCst)
}

fn set_offline<R: Runtime>(app: &AppHandle<R>, offline: bool) {
    let store = app.state::<Store>();
    if store.read(|s: &Session| s.offline) == offline {
        return;
    }
    if let Err(error) = store.update::<Session, _>(app, |s| s.offline = offline) {
        log::error!("cannot push the session: {error}");
    }
}

/// A Group stream opened: online, and the silence watch starts.
pub(crate) fn opened<R: Runtime>(app: &AppHandle<R>) {
    let connection = app.state::<GroupConnection>();
    connection.lost.store(false, Ordering::SeqCst);
    connection.failed.store(0, Ordering::SeqCst);
    app.state::<Scheduler>().clear(Deadline::GroupReconnect);
    heard(app);
    set_offline(app, false);
}

/// An event came on the Group stream: silent for [`SILENCE_MS`] from now
/// means dead.
pub(crate) fn heard<R: Runtime>(app: &AppHandle<R>) {
    app.state::<Scheduler>()
        .set_in(Deadline::GroupStreamSilence, SILENCE_MS);
}

/// No Group any more (left, grace out, superseded, signed out): nothing to
/// watch or reconnect, and not offline.
pub(crate) fn closed<R: Runtime>(app: &AppHandle<R>) {
    let connection = app.state::<GroupConnection>();
    connection.lost.store(false, Ordering::SeqCst);
    connection.failed.store(0, Ordering::SeqCst);
    let scheduler = app.state::<Scheduler>();
    scheduler.clear(Deadline::GroupStreamSilence);
    scheduler.clear(Deadline::GroupReconnect);
    set_offline(app, false);
}

/// [`SILENCE_MS`] passed without an event: the stream is dead.
pub fn silent<R: Runtime>(app: &AppHandle<R>) {
    if group::detach(app) {
        lost(app, "75 s without an event");
    }
}

/// The Group stream is dead (`why`): offline, and the first reconnect
/// attempt after the backoff, unless the loop runs already.
pub(crate) fn lost<R: Runtime>(app: &AppHandle<R>, why: &str) {
    let scheduler = app.state::<Scheduler>();
    scheduler.clear(Deadline::GroupStreamSilence);
    let connection = app.state::<GroupConnection>();
    if connection.lost.swap(true, Ordering::SeqCst) {
        return;
    }
    connection.failed.store(0, Ordering::SeqCst);
    let wait = scheduler.backoff(0);
    log::warn!("Group connection lost ({why}); reconnecting in {wait} ms");
    scheduler.set_in(Deadline::GroupReconnect, wait);
    set_offline(app, true);
}

/// One reconnect attempt; a failure schedules the next after the backoff.
pub async fn reconnect<R: Runtime>(app: &AppHandle<R>) {
    if !reconnecting(app) || app.state::<Backend>().check_superseded().is_err() {
        return;
    }
    if auth::check_token(app).await == TokenCheck::SentToLogin {
        return;
    }
    let response = match session_start::start_session(app).await {
        Ok(Some(response)) => response,
        // Signed out meanwhile.
        Ok(None) => return,
        Err(error) => return failed(app, error).await,
    };
    let in_group = response.group_name.is_some();
    if let Err(error) = session_start::apply(app, response).await {
        return failed(app, internal(error)).await;
    }
    snapshot_removal::send_pending(app).await;
    if !in_group {
        return grace_ran_out(app).await;
    }
    match group::open_stream(app).await {
        Ok(()) => {
            log::info!("reconnected to the Group");
            group_upload_queue::rejoined(app).await;
            auto_snapshot::group_reconnected(app);
        }
        Err(IpcError::Backend {
            reason: BackendReason::NotInGroup,
        }) => grace_ran_out(app).await,
        Err(error) => failed(app, error).await,
    }
}

/// A failed reconnect attempt: login for an expired session, nothing once
/// superseded ([`superseded`] handled it) or no longer reconnecting, else
/// the next attempt after the backoff.
async fn failed<R: Runtime>(app: &AppHandle<R>, error: IpcError) {
    match error {
        IpcError::SessionExpired => auth::send_to_login(app).await,
        IpcError::Superseded => {}
        error if reconnecting(app) => {
            let failed = app
                .state::<GroupConnection>()
                .failed
                .fetch_add(1, Ordering::SeqCst)
                + 1;
            let scheduler = app.state::<Scheduler>();
            let wait = scheduler.backoff(failed);
            log::info!("Group reconnect failed ({error}); retrying in {wait} ms");
            scheduler.set_in(Deadline::GroupReconnect, wait);
        }
        error => log::info!("Group reconnect stopped ({error})"),
    }
}

/// The grace ran out while reconnecting (D22): the client has left the
/// Group, notes it, and goes on solo.
async fn grace_ran_out<R: Runtime>(app: &AppHandle<R>) {
    if !reconnecting(app) {
        return;
    }
    log::info!("the Group's grace ran out: left the Group");
    group::close(app).await;
    state::push(
        app,
        NotificationKind::Warning,
        MessageKey::ErrorNotInGroup.into(),
        None,
    );
}

/// Another Login of the account took over (D22, D25): a Group is closed
/// (the drawer follows `session.superseded`), its upload queue cleared, and
/// a client that was in one stays offline until restarted.
pub(crate) fn superseded<R: Runtime>(app: &AppHandle<R>) {
    let in_group = group::in_group(app);
    group::close_now(app);
    if in_group {
        set_offline(app, true);
    }
}
