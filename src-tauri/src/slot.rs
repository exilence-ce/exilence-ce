//! The operation slot (client-background-work.md, operation slot): serialises
//! the work that changes Profiles, Snapshots or Group membership. A user
//! command finds it taken and is refused with [`IpcError::Busy`]; internal
//! work waits for it. While it is held, `status.operation_running` tells the
//! UI (F1: Sign out is disabled). Reset data cancels the holder first: a
//! cancellable holder (a Snapshot) watches [`OperationSlot::cancellation`].

use std::sync::atomic::{AtomicUsize, Ordering};

use ipc::IpcError;
use ipc::slices::Status;
use tauri::{AppHandle, Manager, Runtime};
use tokio::sync::{Mutex, MutexGuard, watch};

use crate::state::Store;

/// The slot, managed by the app. Held while the guard lives.
pub struct OperationSlot {
    lock: Mutex<()>,
    /// True while someone waits to cancel the holder.
    cancel: watch::Sender<bool>,
    /// How many wait for the slot ([`Self::waiting`]).
    waiting: AtomicUsize,
}

impl Default for OperationSlot {
    fn default() -> Self {
        Self {
            lock: Mutex::default(),
            cancel: watch::Sender::new(false),
            waiting: AtomicUsize::new(0),
        }
    }
}

impl OperationSlot {
    /// For a user command: the slot, or `Busy` when it is taken.
    pub fn try_hold(&self) -> Result<MutexGuard<'_, ()>, IpcError> {
        self.lock.try_lock().map_err(|_| IpcError::Busy)
    }

    /// For internal work: the slot once it is free.
    pub async fn hold(&self) -> MutexGuard<'_, ()> {
        self.wait().await
    }

    /// For a cancellable holder: becomes true when [`Self::cancel_and_hold`]
    /// wants the slot; the holder then stops and releases it.
    pub fn cancellation(&self) -> watch::Receiver<bool> {
        self.cancel.subscribe()
    }

    /// The slot, after asking a cancellable holder to stop (F17: Reset data
    /// cancels a running Snapshot). Other holders finish first.
    pub async fn cancel_and_hold(&self) -> MutexGuard<'_, ()> {
        self.cancel.send_replace(true);
        let guard = self.wait().await;
        self.cancel.send_replace(false);
        guard
    }

    /// How many [`Self::hold`] and [`Self::cancel_and_hold`] calls wait for
    /// the slot: tests see work queue behind a holder.
    pub fn waiting(&self) -> usize {
        self.waiting.load(Ordering::SeqCst)
    }

    /// The slot once it is free, counted in [`Self::waiting`] until then.
    async fn wait(&self) -> MutexGuard<'_, ()> {
        struct Waiting<'a>(&'a AtomicUsize);
        impl Drop for Waiting<'_> {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::SeqCst);
            }
        }
        self.waiting.fetch_add(1, Ordering::SeqCst);
        let _waiting = Waiting(&self.waiting);
        self.lock.lock().await
    }
}

/// The app's slot, held; `status.operation_running` until dropped.
pub struct Held<'a, R: Runtime> {
    app: &'a AppHandle<R>,
    _guard: MutexGuard<'a, ()>,
}

impl<'a, R: Runtime> Held<'a, R> {
    fn new(app: &'a AppHandle<R>, guard: MutexGuard<'a, ()>) -> Self {
        set_running(app, true);
        Self { app, _guard: guard }
    }
}

impl<R: Runtime> Drop for Held<'_, R> {
    fn drop(&mut self) {
        set_running(self.app, false);
    }
}

/// [`OperationSlot::try_hold`] on the app's slot, pushing `status`.
pub fn try_hold<R: Runtime>(app: &AppHandle<R>) -> Result<Held<'_, R>, IpcError> {
    let guard = app.state::<OperationSlot>().inner().try_hold()?;
    Ok(Held::new(app, guard))
}

/// [`OperationSlot::hold`] on the app's slot, pushing `status`.
pub async fn hold<R: Runtime>(app: &AppHandle<R>) -> Held<'_, R> {
    let guard = app.state::<OperationSlot>().inner().hold().await;
    Held::new(app, guard)
}

/// [`OperationSlot::cancel_and_hold`] on the app's slot, pushing `status`.
pub async fn cancel_and_hold<R: Runtime>(app: &AppHandle<R>) -> Held<'_, R> {
    let guard = app.state::<OperationSlot>().inner().cancel_and_hold().await;
    Held::new(app, guard)
}

fn set_running<R: Runtime>(app: &AppHandle<R>, running: bool) {
    if let Err(error) = app
        .state::<Store>()
        .update::<Status, _>(app, |s| s.operation_running = running)
    {
        log::error!("cannot push the status: {error}");
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn commands_are_refused_and_internal_work_waits_while_the_slot_is_taken() {
        tauri::async_runtime::block_on(async {
            let slot = OperationSlot::default();
            let held = slot.try_hold().unwrap();
            assert_eq!(slot.try_hold().unwrap_err(), IpcError::Busy);
            let waiting = tokio::time::timeout(Duration::from_millis(50), slot.hold()).await;
            assert!(waiting.is_err(), "internal work waits");
            assert_eq!(slot.waiting(), 0, "a wait given up no longer counts");
            drop(held);
            let _held = slot.hold().await;
            assert_eq!(slot.try_hold().unwrap_err(), IpcError::Busy);
        });
    }

    #[test]
    fn f17_cancel_and_hold_stops_a_cancellable_holder() {
        tauri::async_runtime::block_on(async {
            let slot = std::sync::Arc::new(OperationSlot::default());
            let held = slot.hold().await;
            let mut cancellation = slot.cancellation();
            let canceller = {
                let slot = slot.clone();
                tauri::async_runtime::spawn(async move {
                    drop(slot.cancel_and_hold().await);
                })
            };
            // The holder (a Snapshot) sees the request and releases the slot.
            cancellation.wait_for(|c| *c).await.unwrap();
            drop(held);
            canceller.await.unwrap();
            assert!(!*slot.cancellation().borrow(), "cleared once taken");
            drop(slot.try_hold().unwrap());
        });
    }
}
