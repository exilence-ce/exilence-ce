//! The window's copy of the core's state: one signal per slice, filled by
//! `get_state` at start and replaced whole by every slice event.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use ipc::command::{GetState, GetStateArgs, LogLevel};
use ipc::slices::{
    Charts, Group, Items, LegacyImport, NetWorth, Notifications, Prices, Profiles, RateLimit,
    Session, Settings, Status, Toast, Update,
};
use ipc::{Event, SliceName, TOAST_EVENT};
use leptos::prelude::*;

use crate::tauri;

/// Every slice as a signal, plus the latest toast and whether the core's
/// values arrived.
#[derive(Clone, Copy)]
pub struct State {
    pub session: RwSignal<Session>,
    pub status: RwSignal<Status>,
    pub rate_limit: RwSignal<RateLimit>,
    pub profiles: RwSignal<Profiles>,
    pub prices: RwSignal<Prices>,
    pub net_worth: RwSignal<NetWorth>,
    pub items: RwSignal<Items>,
    pub charts: RwSignal<Charts>,
    pub group: RwSignal<Group>,
    pub notifications: RwSignal<Notifications>,
    pub settings: RwSignal<Settings>,
    pub update: RwSignal<Update>,
    pub legacy_import: RwSignal<LegacyImport>,
    pub toast: RwSignal<Option<Toast>>,
    /// `get_state` answered: the slices hold the core's values, not their
    /// defaults.
    pub loaded: RwSignal<bool>,
}

impl State {
    /// Every slice at its default, until the core's values arrive.
    pub fn new() -> Self {
        Self {
            session: RwSignal::default(),
            status: RwSignal::default(),
            rate_limit: RwSignal::default(),
            profiles: RwSignal::default(),
            prices: RwSignal::default(),
            net_worth: RwSignal::default(),
            items: RwSignal::default(),
            charts: RwSignal::default(),
            group: RwSignal::default(),
            notifications: RwSignal::default(),
            settings: RwSignal::default(),
            update: RwSignal::default(),
            legacy_import: RwSignal::default(),
            toast: RwSignal::default(),
            loaded: RwSignal::default(),
        }
    }

    /// Replaces the slice `event` carries, or shows its toast.
    pub fn apply(&self, event: Event) {
        match event {
            Event::Session(v) => self.session.set(v),
            Event::Status(v) => self.status.set(v),
            Event::RateLimit(v) => self.rate_limit.set(v),
            Event::Profiles(v) => self.profiles.set(v),
            Event::Prices(v) => self.prices.set(v),
            Event::NetWorth(v) => self.net_worth.set(v),
            Event::Items(v) => self.items.set(v),
            Event::Charts(v) => self.charts.set(v),
            Event::Group(v) => self.group.set(v),
            Event::Notifications(v) => self.notifications.set(v),
            Event::Settings(v) => self.settings.set(v),
            Event::Update(v) => self.update.set(v),
            Event::LegacyImport(v) => self.legacy_import.set(v),
            Event::Toast(v) => self.toast.set(Some(v)),
        }
    }

    /// Subscribes to `slices` and toasts, then loads the current values. A
    /// slice that already arrived by event keeps that value: `get_state` may
    /// have answered before the change the event carries.
    pub async fn connect(self, slices: &[SliceName]) {
        let arrived = Rc::new(RefCell::new(HashSet::new()));
        let names = slices.iter().map(|s| s.event_name()).chain([TOAST_EVENT]);
        for name in names {
            let arrived = Rc::clone(&arrived);
            let handler = move |event: Event| {
                arrived.borrow_mut().extend(event.slice());
                self.apply(event);
            };
            if let Err(error) = tauri::listen(name, handler).await {
                tauri::log(LogLevel::Error, format!("listen {name}: {error}"));
            }
        }
        let args = GetStateArgs {
            slices: slices.to_vec(),
        };
        match tauri::invoke::<GetState>(&args).await {
            Ok(events) => {
                let arrived = arrived.borrow();
                events
                    .into_iter()
                    .filter(|e| e.slice().is_none_or(|s| !arrived.contains(&s)))
                    .for_each(|event| self.apply(event));
                self.loaded.set(true);
            }
            Err(error) => tauri::log(LogLevel::Error, format!("get_state: {error}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_replaces_the_slice_whole() {
        let state = State::new();
        let notifications = Notifications {
            newest: Vec::new(),
            unread: 4,
        };
        state.apply(Event::Notifications(notifications.clone()));
        assert_eq!(state.notifications.get_untracked(), notifications);
        assert_eq!(state.session.get_untracked(), Session::default());
    }
}
