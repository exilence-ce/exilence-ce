//! Auto-snapshot (F4, client-background-work.md, auto-snapshot): the due
//! time is the scheduler's [`Deadline::AutoSnapshot`], memory only.
//!
//! | Event | Due time |
//! |---|---|
//! | Session ready after launch or sign-in, auto on | now |
//! | Group reconnect succeeded, auto on | now |
//! | Snapshot finished, failed or cancelled, auto on | now + interval |
//! | Auto turned on, interval changed while on | now + interval |
//! | Auto turned off, sign-out | cleared |
//!
//! When it comes and a Snapshot cannot start, it retries every
//! [`RETRY_MS`]; the backend is never asked (D26).

use ipc::slices::Settings;
use tauri::{AppHandle, Manager, Runtime};

use crate::scheduler::{Deadline, Scheduler};
use crate::state::Store;
use crate::{session_start, snapshot};

/// The wait before trying again when a due Snapshot cannot start (F4).
pub const RETRY_MS: i64 = 10_000;

fn auto<R: Runtime>(app: &AppHandle<R>) -> Option<i64> {
    app.state::<Store>().read(|s: &Settings| {
        s.auto_snapshot
            .then_some(i64::from(s.auto_snapshot_minutes) * 60_000)
    })
}

/// With auto on and signed in, sets the due time `delay(interval)` ms from
/// now.
fn schedule<R: Runtime>(app: &AppHandle<R>, delay: impl FnOnce(i64) -> i64) {
    if let Some(interval) = auto(app)
        && session_start::signed_in(app)
    {
        app.state::<Scheduler>()
            .set_in(Deadline::AutoSnapshot, delay(interval));
    }
}

/// The session is ready after a launch or sign-in: due now.
pub fn session_ready<R: Runtime>(app: &AppHandle<R>) {
    schedule(app, |_| 0);
}

/// The Group stream reconnected: due now.
pub fn group_reconnected<R: Runtime>(app: &AppHandle<R>) {
    schedule(app, |_| 0);
}

/// A Snapshot, manual or auto, finished, failed or was cancelled: due one
/// interval from now.
pub fn snapshot_ended<R: Runtime>(app: &AppHandle<R>) {
    schedule(app, |interval| interval);
}

/// The settings changed from `old`: auto off clears the due time; auto
/// turned on or its interval changed sets it one interval from now.
pub fn settings_changed<R: Runtime>(app: &AppHandle<R>, old: &Settings) {
    let new = app.state::<Store>().read(Settings::clone);
    if !new.auto_snapshot {
        clear(app);
    } else if !old.auto_snapshot || old.auto_snapshot_minutes != new.auto_snapshot_minutes {
        schedule(app, |interval| interval);
    }
}

/// Sign-out: no due time.
pub fn clear<R: Runtime>(app: &AppHandle<R>) {
    app.state::<Scheduler>().clear(Deadline::AutoSnapshot);
}

/// The due time came: starts a Snapshot as `take_snapshot` does; when it
/// cannot start (start conditions, slot), due again in [`RETRY_MS`]. Once
/// started, its end sets the next due time.
pub fn fire<R: Runtime>(app: &AppHandle<R>) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = snapshot::start(&app).await {
            log::debug!("auto-snapshot cannot start ({error}); retrying in 10 s");
            if auto(&app).is_some() && session_start::signed_in(&app) {
                app.state::<Scheduler>()
                    .set_in(Deadline::AutoSnapshot, RETRY_MS);
            }
        }
    });
}
