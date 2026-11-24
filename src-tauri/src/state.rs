//! The core's state: every slice, each pushed whole to the webviews whenever
//! it changes (ADR 0005).
//!
//! Every event goes out through [`Store`], one at a time. Tauri's event
//! listeners take their lock with `try_lock`: an emit or a listen that finds
//! it taken is parked in a queue that is flushed only after a later emit
//! that ran a Rust handler. Two threads emitting at once could so park a
//! toast behind a slice nobody listens to, for good, or let it overtake a
//! later event. Rust-side listeners register through [`listen`] for the
//! same reason.
//!
//! A push that computes its slice from other state (the database, the
//! Group, other slices) takes its slice's push lock before it reads that
//! state and holds it until the slice went out: its [`Pushing`].
//! Otherwise a push that read before a change could land after the push of
//! that change, and the slice would keep the older value.

use std::marker::PhantomData;
use std::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

use ipc::slices::{Notification, NotificationKind, Notifications, Toast};
use ipc::{Event, Message, Slice, SliceName, Slices};
use tauri::{AppHandle, Emitter, EventId, Listener, Manager, Runtime};

use crate::scheduler::Scheduler;

/// The push lock of slice `S`, managed by the app once per slice that is
/// computed from other state: one push of `S` at a time, so a push never
/// overwrites a newer one.
pub struct Pushing<S>(tokio::sync::Mutex<()>, PhantomData<fn() -> S>);

impl<S> Default for Pushing<S> {
    fn default() -> Self {
        Self(tokio::sync::Mutex::default(), PhantomData)
    }
}

impl<S> Pushing<S> {
    /// Waits until no other push of `S` holds the lock.
    pub async fn lock(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.0.lock().await
    }
}

/// The state store, managed by the app.
#[derive(Default)]
pub struct Store {
    slices: RwLock<Slices>,
    /// Held while an event goes out or a listener registers.
    events: Mutex<()>,
}

impl Store {
    /// A store holding `slices`.
    pub fn new(slices: Slices) -> Self {
        Self {
            slices: RwLock::new(slices),
            events: Mutex::default(),
        }
    }

    /// Reads slice `S` with `read`.
    pub fn read<S: Slice, T>(&self, read: impl FnOnce(&S) -> T) -> T {
        read(S::get(&self.read_lock()))
    }

    /// The current value of each named slice, in order.
    pub fn get(&self, names: &[SliceName]) -> Vec<Event> {
        let slices = self.read_lock();
        names.iter().map(|&name| slices.event(name)).collect()
    }

    /// Changes slice `S` with `change`, then emits the whole new slice,
    /// serialised in place. Updates go out in the order they were made, so
    /// the last event of a slice holds its current value. The event goes out
    /// after the slice lock is released, so listeners may read the store;
    /// they must not emit.
    pub fn update<S: Slice, R: Runtime>(
        &self,
        app: &AppHandle<R>,
        change: impl FnOnce(&mut S),
    ) -> tauri::Result<()> {
        let _events = self.events();
        let (name, payload) = {
            let mut slices = self.write_lock();
            let slice = S::get_mut(&mut slices);
            change(slice);
            let event = slice.as_event();
            (event.name(), serde_json::to_string(&event)?)
        };
        app.emit_str(name, payload)
    }

    fn read_lock(&self) -> RwLockReadGuard<'_, Slices> {
        // A panicking change leaves a valid, if partly changed, slice; keep
        // serving it rather than failing every later command.
        self.slices.read().unwrap_or_else(|e| e.into_inner())
    }

    fn write_lock(&self) -> RwLockWriteGuard<'_, Slices> {
        self.slices.write().unwrap_or_else(|e| e.into_inner())
    }

    fn events(&self) -> MutexGuard<'_, ()> {
        self.events.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Registers a Rust-side `handler` for event `name` on `target` (the app or
/// a window), never while an event goes out (see the module docs). The
/// handler must not emit.
pub fn listen<R: Runtime, L: Listener<R> + Manager<R>>(
    target: &L,
    name: &str,
    handler: impl Fn(tauri::Event) + Send + 'static,
) -> EventId {
    let store = target.state::<Store>();
    let _events = store.events();
    target.listen(name, handler)
}

/// How many notifications the list keeps (F18).
pub const NOTIFICATIONS_KEPT: usize = 10;

/// Adds a notification to the list, newest first, keeping the newest
/// [`NOTIFICATIONS_KEPT`] (F18). A failure is also shown as a toast; other
/// kinds are list-only.
pub fn notify<R: Runtime>(
    app: &AppHandle<R>,
    kind: NotificationKind,
    message: Message,
    description: Option<Message>,
) -> tauri::Result<()> {
    let at = app.state::<Scheduler>().now();
    let toast_too = (kind == NotificationKind::Error).then(|| Toast {
        kind,
        message: message.clone(),
        description: description.clone(),
    });
    // The list first: whoever reacts to the toast finds it already listed.
    app.state::<Store>().update::<Notifications, _>(app, |n| {
        let id = n.newest.first().map_or(1, |newest| newest.id + 1);
        n.newest.insert(
            0,
            Notification {
                id,
                at,
                kind,
                message,
                description,
            },
        );
        n.newest.truncate(NOTIFICATIONS_KEPT);
        n.unread += 1;
    })?;
    toast_too.map_or(Ok(()), |t| toast(app, t))
}

/// [`notify`], logging a failure to push instead of returning it.
pub fn push<R: Runtime>(
    app: &AppHandle<R>,
    kind: NotificationKind,
    message: Message,
    description: Option<Message>,
) {
    if let Err(error) = notify(app, kind, message, description) {
        log::error!("cannot push a notification: {error}");
    }
}

/// Shows a transient toast (F18).
pub fn toast<R: Runtime>(app: &AppHandle<R>, toast: Toast) -> tauri::Result<()> {
    let event = Event::Toast(toast);
    let store = app.state::<Store>();
    let _events = store.events();
    app.emit(event.name(), &event)
}
