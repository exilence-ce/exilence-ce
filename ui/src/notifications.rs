//! Toasts and the notification bell (F18). The core decides what becomes a
//! notification or a toast; this only shows them.

use std::time::Duration;

use ipc::command::{MarkNotificationsRead, NoArgs};
use ipc::slices::{Notification, NotificationKind, Notifications};
use ipc::{Message, MessageKey};
use leptos::prelude::*;

use crate::LOCALE;
use crate::shell::{Icon, icon};
use crate::state::State;
use crate::tauri;

/// How long a toast shows unless closed.
const TOAST_SHOWN: Duration = Duration::from_secs(6);

/// The latest toast, until it times out, is closed or a newer one replaces
/// it.
#[component]
pub fn Toasts(state: State) -> impl IntoView {
    let generation = StoredValue::new(0u64);
    Effect::new(move |_| {
        if state.toast.with(Option::is_some) {
            let shown = generation.get_value() + 1;
            generation.set_value(shown);
            set_timeout(
                move || {
                    if generation.get_value() == shown {
                        state.toast.set(None);
                    }
                },
                TOAST_SHOWN,
            );
        }
    });
    move || {
        state.toast.get().map(|toast| {
            view! {
                <div class=format!("toast {}", kind_class(toast.kind)) role="alert">
                    <Icon path=kind_icon(toast.kind) />
                    <div class="toast-text">
                        <div class="toast-title">{LOCALE.text(&toast.message)}</div>
                        {toast
                            .description
                            .map(|d| view! { <div class="toast-description">{LOCALE.text(&d)}</div> })}
                    </div>
                    <button
                        class="control"
                        title=LOCALE.key(MessageKey::TitleBarClose)
                        on:click=move |_| state.toast.set(None)
                    >
                        <Icon path=icon::CLOSE />
                    </button>
                </div>
            }
        })
    }
}

/// The bell: its badge counts unread notifications; opening the list shows
/// the newest 10 and marks all read.
#[component]
pub fn NotificationBell(state: State) -> impl IntoView {
    let open = RwSignal::new(false);
    let toggle = move |_| {
        let opening = !open.get_untracked();
        open.set(opening);
        if opening {
            tauri::send::<MarkNotificationsRead>(NoArgs {});
        }
    };
    let title = move || LOCALE.text(&state.notifications.with(unread_message));
    view! {
        <div class="bell" data-tour="notifications">
            <button class="control" class:active=move || open.get() title=title on:click=toggle>
                <Icon path=icon::BELL />
                {move || {
                    badge(state.notifications.with(|n| n.unread))
                        .map(|badge| view! { <span class="badge">{badge}</span> })
                }}
            </button>
            <Show when=move || open.get()>
                <div class="backdrop" on:click=move |_| open.set(false)></div>
                {move || view! { <NotificationList notifications=state.notifications.get() /> }}
            </Show>
        </div>
    }
}

#[component]
fn NotificationList(notifications: Notifications) -> impl IntoView {
    let now = js_sys::Date::now() as i64;
    let empty = notifications.newest.is_empty().then(|| {
        view! { <p class="empty">{LOCALE.key(MessageKey::NotificationsEmpty)}</p> }
    });
    let items = notifications
        .newest
        .into_iter()
        .map(|n| view! { <NotificationItem notification=n now /> })
        .collect_view();
    view! {
        <section class="notification-list">
            <h2>{LOCALE.key(MessageKey::Notifications)}</h2>
            {empty}
            <ul>{items}</ul>
        </section>
    }
}

#[component]
fn NotificationItem(notification: Notification, now: i64) -> impl IntoView {
    view! {
        <li class=format!("notification {}", kind_class(notification.kind))>
            <Icon path=kind_icon(notification.kind) />
            <div>
                <div class="notification-title">{LOCALE.text(&notification.message)}</div>
                <div class="notification-time">
                    {LOCALE.text(&time_ago(now, notification.at))}
                </div>
                {notification
                    .description
                    .map(|d| view! { <div class="notification-description">{LOCALE.text(&d)}</div> })}
            </div>
        </li>
    }
}

fn kind_class(kind: NotificationKind) -> &'static str {
    match kind {
        NotificationKind::Info => "info",
        NotificationKind::Success => "success",
        NotificationKind::Warning => "warning",
        NotificationKind::Error => "error",
    }
}

fn kind_icon(kind: NotificationKind) -> &'static str {
    match kind {
        NotificationKind::Info => icon::INFO,
        NotificationKind::Success => icon::SUCCESS,
        NotificationKind::Warning => icon::WARNING,
        NotificationKind::Error => icon::ERROR,
    }
}

/// The badge of `unread` notifications: shown up to 9, then `9+` (F18).
pub fn badge(unread: u32) -> Option<String> {
    match unread {
        0 => None,
        1..=9 => Some(unread.to_string()),
        _ => Some("9+".to_owned()),
    }
}

fn unread_message(notifications: &Notifications) -> Message {
    Message::new(MessageKey::NotificationsUnread).arg("count", notifications.unread)
}

/// How long ago `at` was at `now` (Unix ms).
pub fn time_ago(now: i64, at: i64) -> Message {
    const MINUTE: i64 = 60_000;
    const HOUR: i64 = 60 * MINUTE;
    const DAY: i64 = 24 * HOUR;
    let ago = now.saturating_sub(at).max(0);
    let (key, unit) = if ago < MINUTE {
        return MessageKey::TimeJustNow.into();
    } else if ago < HOUR {
        (MessageKey::TimeMinutesAgo, MINUTE)
    } else if ago < DAY {
        (MessageKey::TimeHoursAgo, HOUR)
    } else {
        (MessageKey::TimeDaysAgo, DAY)
    };
    Message::new(key).arg("count", (ago / unit) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f18_badge_counts_unread_up_to_9() {
        assert_eq!(badge(0), None);
        assert_eq!(badge(1).as_deref(), Some("1"));
        assert_eq!(badge(9).as_deref(), Some("9"));
        assert_eq!(badge(10).as_deref(), Some("9+"));
    }

    #[test]
    fn f18_relative_time() {
        let at = 1_700_000_000_000;
        let text = |ms: i64| LOCALE.text(&time_ago(at + ms, at));
        assert_eq!(text(-5_000), "just now");
        assert_eq!(text(59_999), "just now");
        assert_eq!(text(60_000), "one minute ago");
        assert_eq!(text(5 * 60_000 + 59_000), "5 minutes ago");
        assert_eq!(text(3_600_000), "one hour ago");
        assert_eq!(text(23 * 3_600_000), "23 hours ago");
        assert_eq!(text(3 * 86_400_000), "3 days ago");
    }

    #[test]
    fn unread_count_resolves_through_the_locale() {
        let notifications = Notifications {
            newest: Vec::new(),
            unread: 3,
        };
        assert_eq!(
            LOCALE.text(&unread_message(&notifications)),
            "3 unread notifications"
        );
    }
}
