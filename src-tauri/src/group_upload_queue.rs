//! The Group upload queue (D27, client-background-work.md): a Snapshot
//! taken in a Group whose upload failed on the way waits here, in memory,
//! oldest first, and is sent again one at a time without the operation slot:
//! first after a random [`FIRST_RETRY_MS`], then with the
//! [backoff](crate::scheduler::backoff_ms), and right after every rejoin. It
//! leaves the queue when its upload succeeds, when the backend refuses it
//! (logged), when it is removed locally, when the Group is left or its grace
//! runs out, on `SUPERSEDED`, and when the app quits. One that leaves while
//! it is being sent still ends that attempt before the next send starts; if
//! it was removed, its pending removal goes to the backend again once that
//! upload ended, as the upload may have landed after it (D5).

use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard};

use ipc::IpcError;
use sqlx::SqlitePool;
use tauri::{AppHandle, Manager, Runtime};
use wire::v1::UploadSnapshotRequest;

use crate::backend::{self, Backend};
use crate::scheduler::{Deadline, Scheduler};
use crate::{snapshot, snapshot_removal};

/// The random wait before the first retry, in ms.
pub const FIRST_RETRY_MS: (i64, i64) = (1_000, 10_000);

/// The queue, managed by the app.
#[derive(Default)]
pub struct UploadQueue(Mutex<Queue>);

#[derive(Default)]
struct Queue {
    /// Oldest first.
    waiting: VecDeque<UploadSnapshotRequest>,
    /// The Snapshot being sent, until its attempt ended: one at a time.
    sending: Option<Sending>,
    /// Failed retries since the last success: n of the backoff.
    failed: u32,
}

/// The Snapshot being sent.
struct Sending {
    id: String,
    state: SendingState,
}

/// What became of the Snapshot being sent while its attempt runs.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SendingState {
    /// Still queued: a failed attempt queues it again.
    Active,
    /// It left the queue: a failed attempt is not queued again.
    Left,
    /// Removed locally, so it left the queue too: the pending removals go
    /// again once the attempt ended.
    Removed,
}

impl UploadQueue {
    fn lock(&self) -> MutexGuard<'_, Queue> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn id(request: &UploadSnapshotRequest) -> &str {
    request.snapshot.as_ref().map_or("", |s| s.id.as_str())
}

/// Whether Snapshot `id` waits in the queue or is being sent: an upload of
/// it may still reach the backend.
pub(crate) fn holds<R: Runtime>(app: &AppHandle<R>, id: &str) -> bool {
    let uploads = app.state::<UploadQueue>();
    let queue = uploads.lock();
    queue.sending.as_ref().is_some_and(|s| s.id == id)
        || queue.waiting.iter().any(|r| self::id(r) == id)
}

/// The upload of `request` failed in a Group with `status`: a transport
/// failure queues it, the first retry after a random [`FIRST_RETRY_MS`] when
/// nothing else waits; anything else is logged (`SUPERSEDED` takes the
/// client offline, [`backend::failure`]).
pub(crate) fn upload_failed<R: Runtime>(
    app: &AppHandle<R>,
    request: UploadSnapshotRequest,
    status: &tonic::Status,
) {
    let error = backend::failure(app, status);
    if error != IpcError::Offline {
        return log::warn!(
            "UploadSnapshot of {} refused in a Group ({error}); not retried",
            id(&request)
        );
    }
    log::info!(
        "UploadSnapshot of {} failed ({error}); queued (D27)",
        id(&request)
    );
    let uploads = app.state::<UploadQueue>();
    let mut queue = uploads.lock();
    let idle = queue.waiting.is_empty() && queue.sending.is_none();
    queue.waiting.push_back(request);
    if idle {
        queue.failed = 0;
        let scheduler = app.state::<Scheduler>();
        let wait = scheduler.random_ms(FIRST_RETRY_MS.0, FIRST_RETRY_MS.1);
        scheduler.set_in(Deadline::GroupUploadRetry, wait);
    }
}

/// Sends the queue, oldest first, until it is empty or a transport failure
/// schedules the next attempt after the backoff. Nothing while another
/// drain sends.
pub async fn drain<R: Runtime>(app: &AppHandle<R>) {
    let uploads = app.state::<UploadQueue>();
    loop {
        if app.state::<Backend>().check_superseded().is_err() {
            return clear(app);
        }
        let token: Option<String> = match sqlx::query_scalar("SELECT session_token FROM login")
            .fetch_optional(&*app.state::<SqlitePool>())
            .await
        {
            Ok(token) => token,
            Err(error) => {
                log::error!("cannot read the Login for the upload queue: {error}");
                None
            }
        };
        let Some(token) = token else {
            return clear(app);
        };
        let request = {
            let mut queue = uploads.lock();
            if queue.sending.is_some() {
                return;
            }
            let Some(request) = queue.waiting.pop_front() else {
                return;
            };
            queue.sending = Some(Sending {
                id: id(&request).to_owned(),
                state: SendingState::Active,
            });
            request
        };
        // The original stays queued if this attempt fails too.
        let error = match snapshot::send(app, &token, request.clone()).await {
            Ok(()) => None,
            Err(status) => Some(backend::failure(app, &status)),
        };
        // Under the lock, which must not be held across the await below.
        let state;
        let more = {
            let mut queue = uploads.lock();
            state = queue.sending.take().map_or(SendingState::Left, |s| s.state);
            match error {
                None => {
                    log::info!("queued Snapshot {} uploaded", id(&request));
                    queue.failed = 0;
                    true
                }
                Some(IpcError::Offline) => {
                    if state == SendingState::Active {
                        queue.waiting.push_front(request);
                    }
                    if !queue.waiting.is_empty() {
                        let scheduler = app.state::<Scheduler>();
                        let wait = scheduler.backoff(queue.failed);
                        queue.failed += 1;
                        log::info!("the upload queue retries in {wait} ms");
                        scheduler.set_in(Deadline::GroupUploadRetry, wait);
                    }
                    false
                }
                Some(error) => {
                    log::warn!(
                        "UploadSnapshot of {} refused ({error}); it leaves the queue",
                        id(&request)
                    );
                    true
                }
            }
        };
        if state == SendingState::Removed {
            snapshot_removal::send_pending(app).await;
        }
        if !more {
            return;
        }
    }
}

/// The Group was rejoined: the queue goes out now.
pub(crate) async fn rejoined<R: Runtime>(app: &AppHandle<R>) {
    app.state::<UploadQueue>().lock().failed = 0;
    app.state::<Scheduler>().clear(Deadline::GroupUploadRetry);
    drain(app).await;
}

/// Snapshots `ids` were removed locally: they leave the queue; one being
/// sent ends its attempt first.
pub(crate) fn removed<R: Runtime>(app: &AppHandle<R>, ids: &[String]) {
    let uploads = app.state::<UploadQueue>();
    let mut queue = uploads.lock();
    queue
        .waiting
        .retain(|r| !ids.iter().any(|removed| removed == id(r)));
    if let Some(sending) = queue.sending.as_mut().filter(|s| ids.contains(&s.id)) {
        sending.state = SendingState::Removed;
    }
    if queue.waiting.is_empty() {
        app.state::<Scheduler>().clear(Deadline::GroupUploadRetry);
    }
}

/// Empties the queue: the Group was left or its grace ran out, superseded,
/// signed out. A Snapshot being sent ends its attempt first.
pub(crate) fn clear<R: Runtime>(app: &AppHandle<R>) {
    let uploads = app.state::<UploadQueue>();
    let mut queue = uploads.lock();
    queue.waiting.clear();
    queue.failed = 0;
    if let Some(sending) = queue
        .sending
        .as_mut()
        .filter(|s| s.state == SendingState::Active)
    {
        sending.state = SendingState::Left;
    }
    app.state::<Scheduler>().clear(Deadline::GroupUploadRetry);
}
