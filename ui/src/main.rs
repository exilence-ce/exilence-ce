//! The Leptos UI: renders the slices the core pushes (ADR 0005).

mod charts;
mod currency;
mod group;
mod item_table;
mod legacy_import;
mod login;
mod markdown;
mod menu;
mod notifications;
mod overlay;
mod profiles;
mod settings;
mod shell;
mod snapshot;
mod snapshot_removal;
mod state;
mod tauri;
mod tour;

use std::sync::LazyLock;
use std::time::Duration;

use ipc::command::{
    LogLevel, NoArgs, OpenOverlay, SettingsPatch, UpdateSettings, UpdateSettingsArgs,
};
use ipc::slices::{LoginState, PriceAvailability, Prices, Status};
use ipc::{Locale, Message, MessageKey, SliceName, links};
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::{Redirect, Route, Router, Routes};
use leptos_router::hooks::{use_location, use_navigate};
use leptos_router::path;

use crate::charts::{ChartSection, ChartSections};
use crate::group::{GroupButton, GroupDrawer};
use crate::item_table::ItemTable;
use crate::legacy_import::LegacyImportDialog;
use crate::login::Login;
use crate::markdown::{InfoDialog, InfoPage};
use crate::notifications::{NotificationBell, Toasts, time_ago};
use crate::overlay::Overlay;
use crate::profiles::ProfileControls;
use crate::settings::SettingsView;
use crate::shell::{AccountMenu, Icon, NavDrawer, Onboarding, TitleBar, icon, open_link};
use crate::snapshot::{NetWorthWidgets, SnapshotControls};
use crate::snapshot_removal::RemoveSnapshotsControl;
use crate::state::State;
use crate::tour::Tour;

/// The locale resources (D10).
static LOCALE: LazyLock<Locale> = LazyLock::new(Locale::english);

/// The slices the main window shows.
const MAIN_SLICES: &[SliceName] = SliceName::ALL;

fn main() {
    install_panic_hook();
    suppress_context_menu();
    let state = State::new();
    // Mounting starts the executor `spawn_local` needs.
    let path = web_sys::window().and_then(|w| w.location().pathname().ok());
    // The overlay window (F14) loads the same app on its own route.
    if path.as_deref() == Some(overlay::PATH) {
        mount_to_body(move || view! { <Overlay state /> });
        spawn_local(state.connect(overlay::SLICES));
    } else {
        mount_to_body(move || view! { <Main state /> });
        spawn_local(state.connect(MAIN_SLICES));
    }
    // `tools/ui-smoke.sh` waits for this line.
    tauri::log(LogLevel::Info, "UI mounted".to_owned());
}

/// Text fields keep the webview's menu, for cut, copy and paste.
const CONTEXT_MENU_FIELDS: &str = "textarea, [contenteditable], \
    input:not([type=checkbox]):not([type=radio]):not([type=range])";

/// Turns off the webview's right-click menu (Back, Reload, Inspect…) in
/// every window, except on text fields.
fn suppress_context_menu() {
    // Lives as long as the window.
    _ = window_event_listener(leptos::ev::contextmenu, |ev| {
        let in_field = ev
            .target()
            .and_then(|t| wasm_bindgen::JsCast::dyn_into::<web_sys::Element>(t).ok())
            .and_then(|el| el.closest(CONTEXT_MENU_FIELDS).ok().flatten())
            .is_some();
        if !in_field {
            ev.prevent_default();
        }
    });
}

/// Sends a panic to the core's log (F20), then replaces the page with an
/// error state that points to Export diagnostics, so the window never just
/// freezes.
fn install_panic_hook() {
    // Resolved now: the hook must not depend on anything a panic may have
    // left half-done.
    let crashed = LOCALE.key(MessageKey::UiCrashed);
    std::panic::set_hook(Box::new(move |info| {
        tauri::log(LogLevel::Error, info.to_string());
        if let Some(body) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.body())
        {
            body.set_text_content(Some(&crashed));
        }
    }));
}

/// The main window: title bar, then the login screen, or the navigation
/// drawer, toolbar and current view; the legacy import dialog (F21), the
/// tour, What's New and Credits (F19) and toasts over everything.
#[component]
fn Main(state: State) -> impl IntoView {
    let signed_in = move || state.session.with(|s| s.login == LoginState::SignedIn);
    // The Group drawer (F13).
    let group_open = RwSignal::new(false);
    let onboarding = Onboarding::default();
    tour::start_on_first_run(state, onboarding.tour);
    view! {
        <Router>
            <SessionRedirect state />
            <TitleBar state onboarding />
            <div class="body">
                <Show when=signed_in>
                    <NavDrawer state />
                </Show>
                <main class="content">
                    <Show when=signed_in>
                        <Toolbar state group_open />
                    </Show>
                    <Routes fallback=|| view! { <Redirect path="/" /> }>
                        <Route path=path!("/") view=move || view! { <NetWorth state /> } />
                        <Route path=path!("/settings") view=move || view! { <SettingsView state /> } />
                        <Route path=path!("/login") view=move || view! { <Login state /> } />
                    </Routes>
                </main>
                <Show when=signed_in>
                    <GroupDrawer state open=group_open />
                </Show>
            </div>
            <Show when=signed_in>
                <LegacyImportDialog state />
                <Tour open=onboarding.tour whats_new=onboarding.whats_new />
            </Show>
            <InfoDialog page=InfoPage::WhatsNew open=onboarding.whats_new />
            <InfoDialog page=InfoPage::Credits open=onboarding.credits />
            <Toasts state />
        </Router>
    }
}

/// Follows the session: login screen while signed out, Net worth view after
/// signing in (F1).
#[component]
fn SessionRedirect(state: State) -> impl IntoView {
    let navigate = use_navigate();
    let location = use_location();
    Effect::new(move |_| {
        let path = location.pathname.get();
        if let Some(to) = state.session.with(|s| login::redirect(s, &path)) {
            navigate(to, Default::default());
        }
    });
}

/// The signed-in toolbar: account name, status (or the rate-limit countdown
/// while a cooldown runs), "Not connected to server" while a Group's
/// connection is lost (F18), the no-prices warning (F5), the debug marker
/// while Debug mode is on (F20), the Profile controls (F2), Snapshot and
/// Cancel (F3), Remove Snapshots (F12), the overlay button (F14), the Group
/// button (F13) and the notification bell.
#[component]
fn Toolbar(state: State, group_open: RwSignal<bool>) -> impl IntoView {
    // The countdown ticks here from the pushed end time (F7).
    let now = RwSignal::new(js_sys::Date::now() as i64);
    if let Ok(tick) = set_interval_with_handle(
        move || now.set(js_sys::Date::now() as i64),
        Duration::from_secs(1),
    ) {
        on_cleanup(move || tick.clear());
    }
    // Changes once a second while a cooldown runs, never otherwise.
    let cooldown = Memo::new(move |_| {
        state
            .rate_limit
            .with(|r| cooldown_seconds(r.cooldown_until, now.get()))
    });
    view! {
        <header class="toolbar">
            {move || match cooldown.get() {
                Some(seconds) => {
                    let wait = cooldown_message(seconds);
                    view! {
                        <span
                            class="status rate-limit warning"
                            role="status"
                            title=LOCALE.key(MessageKey::RateLimitExceeded)
                        >
                            <Icon path=icon::WARNING />
                            {LOCALE.text(&wait)}
                        </span>
                    }
                        .into_any()
                }
                None => {
                    view! {
                        <span class="status">
                            {move || state.status.with(status_message).map(|m| LOCALE.text(&m))}
                        </span>
                    }
                        .into_any()
                }
            }}
            // Only ever set in a Group (F18, D25).
            <Show when=move || state.session.with(|s| s.offline)>
                <span class="offline warning" role="status">
                    <Icon path=icon::WARNING />
                    {LOCALE.key(MessageKey::ErrorOffline)}
                </span>
            </Show>
            // Legacy shows it only outside a cooldown (F5).
            <Show when=move || {
                cooldown.get().is_none()
                    && state.prices.with(|p| p.availability == PriceAvailability::Unavailable)
            }>
                <span
                    class="no-prices warning"
                    role="status"
                    title=move || state.prices.with(|p| LOCALE.text(&no_prices_title(p)))
                >
                    <Icon path=icon::WARNING />
                    {LOCALE.key(MessageKey::PricesUnavailable)}
                </span>
            </Show>
            <Show when=move || state.status.with(|s| s.debug_mode)>
                <span
                    class="debug-marker"
                    role="status"
                    title=LOCALE.key(MessageKey::DebugMarkerTitle)
                >
                    {LOCALE.key(MessageKey::DebugMarker)}
                </span>
            </Show>
            // Legacy's sections, divided by lines (`Toolbar.tsx`).
            <div class="toolbar-section">
                <ProfileControls state />
            </div>
            <span class="toolbar-divider"></span>
            <div class="toolbar-section">
                <SnapshotControls
                    state
                    cooling_down=Signal::derive(move || cooldown.get().is_some())
                />
                <RemoveSnapshotsControl state />
            </div>
            <span class="toolbar-divider"></span>
            <div class="toolbar-section">
                <button
                    class="control overlay-button"
                    data-tour="overlay"
                    title=LOCALE.key(MessageKey::OverlayOpen)
                    on:click=|_| { tauri::send::<OpenOverlay>(NoArgs {}) }
                >
                    <Icon path=icon::ADD_TO_PHOTOS />
                </button>
            </div>
            <span class="toolbar-divider"></span>
            <div class="toolbar-section">
                <GroupButton state open=group_open />
            </div>
            <span class="toolbar-divider"></span>
            <div class="toolbar-section">
                <NotificationBell state />
                <AccountMenu state />
            </div>
        </header>
    }
}

/// The Net worth view, with the item table's header: where and how long
/// ago prices were fetched (F5), ticked here from the pushed time; and the
/// item table (F11).
#[component]
fn NetWorth(state: State) -> impl IntoView {
    let now = RwSignal::new(js_sys::Date::now() as i64);
    if let Ok(tick) = set_interval_with_handle(
        move || now.set(js_sys::Date::now() as i64),
        Duration::from_secs(10),
    ) {
        on_cleanup(move || tick.clear());
    }
    let items_open = Signal::derive(move || state.settings.with(|s| s.items_open));
    let toggle_items = move || {
        tauri::send::<UpdateSettings>(UpdateSettingsArgs {
            patch: SettingsPatch {
                items_open: Some(!items_open.get_untracked()),
                ..SettingsPatch::default()
            },
        })
    };
    // Legacy: "Prices fetched from https://poe.ninja an hour ago".
    let fetched = move || {
        state.prices.with(|p| p.fetched_at).map(|at| {
            view! {
                <span class="prices-fetched" title=LOCALE.key(MessageKey::PricesFetchedInterval)>
                    {LOCALE.key(MessageKey::PricesFetchedFrom)}
                    " "
                    <a
                        href=links::POE_NINJA
                        on:click=|ev| {
                            ev.prevent_default();
                            ev.stop_propagation();
                            open_link(links::POE_NINJA);
                        }
                    >
                        {links::POE_NINJA}
                    </a>
                    " "
                    {LOCALE.text(&time_ago(now.get(), at))}
                </span>
            }
        })
    };
    view! {
        <section class="net-worth">
            <NetWorthWidgets state />
            <ChartSections state />
            <ChartSection
                class="item-table-section"
                title=MessageKey::ItemTableTitle
                icon=icon::UPDATE
                aside=fetched
                open=items_open
                toggle=toggle_items
            >
                <ItemTable state />
            </ChartSection>
        </section>
    }
}

/// The no-prices warning's title, naming the price league (F5).
fn no_prices_title(prices: &Prices) -> Message {
    Message::new(MessageKey::PricesUnavailableTitle)
        .arg("league", prices.price_league_id.clone().unwrap_or_default())
}

/// The status line: the status message, else the Snapshot's progress.
fn status_message(status: &Status) -> Option<Message> {
    status.message.clone().or_else(|| {
        status.snapshot_progress.map(|p| {
            Message::new(MessageKey::StatusSnapshotProgress)
                .arg("tab", p.tab)
                .arg("count", p.count)
        })
    })
}

/// Whole seconds left of a cooldown ending at `until` (Unix ms) at `now`,
/// rounded up; `None` once it has ended (F7).
fn cooldown_seconds(until: Option<i64>, now: i64) -> Option<u64> {
    let left = u64::try_from(until? - now).ok().filter(|&ms| ms > 0)?;
    Some(left.div_ceil(1000))
}

/// "Rate limit exceeded, please wait: N seconds" (F7).
fn cooldown_message(seconds: u64) -> Message {
    Message::new(MessageKey::RateLimitWait).arg("seconds", seconds)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ipc::slices::SnapshotProgress;

    #[test]
    fn status_line_shows_message_else_progress() {
        let mut status = Status {
            message: None,
            snapshot_running: true,
            operation_running: true,
            snapshot_progress: Some(SnapshotProgress { tab: 2, count: 5 }),
            snapshot_saving: false,
            debug_mode: false,
        };
        let text = |s: &Status| status_message(s).map(|m| LOCALE.text(&m));
        assert_eq!(text(&status).as_deref(), Some("Fetching stash tab 2 / 5"));
        status.message = Some(MessageKey::ErrorOffline.into());
        assert_eq!(text(&status).as_deref(), Some("Not connected to server"));
        assert_eq!(text(&Status::default()), None);
    }

    #[test]
    fn f7_countdown_counts_whole_seconds_down_to_the_end() {
        let until = Some(1_800_000_060_000);
        let text = |now| cooldown_seconds(until, now).map(|s| LOCALE.text(&cooldown_message(s)));
        assert_eq!(
            text(1_800_000_000_000).as_deref(),
            Some("Rate limit exceeded, please wait: 60 seconds")
        );
        assert_eq!(
            text(1_800_000_059_500).as_deref(),
            Some("Rate limit exceeded, please wait: 1 seconds")
        );
        assert_eq!(text(1_800_000_060_000), None);
        assert_eq!(cooldown_seconds(None, 0), None);
    }

    #[test]
    fn f5_no_prices_title_names_the_price_league() {
        let prices = Prices {
            price_league_id: Some("Settlers".to_owned()),
            ..Prices::default()
        };
        assert_eq!(
            LOCALE.text(&no_prices_title(&prices)),
            "No prices fetched for `Settlers`, you can change pricing league in your profile"
        );
    }

    /// A rule left open swallows every rule after it without an error: the
    /// webview reads them as nested rules that never match.
    #[test]
    fn style_css_closes_every_rule() {
        let mut css = include_str!("../style.css");
        let mut text = String::new();
        while let Some(start) = css.find("/*") {
            text.push_str(&css[..start]);
            let end = css[start..].find("*/").expect("comment closed");
            css = &css[start + end + 2..];
        }
        text.push_str(css);
        // Per open block: whether it is an at-rule such as `@media`.
        let mut blocks: Vec<bool> = Vec::new();
        let mut prelude = String::new();
        for c in text.chars() {
            match c {
                '{' => {
                    let selector = prelude.trim();
                    assert!(
                        blocks.last().is_none_or(|&at_rule| at_rule),
                        "`{selector}` opens inside a rule left open"
                    );
                    blocks.push(selector.starts_with('@'));
                    prelude.clear();
                }
                '}' => {
                    assert!(blocks.pop().is_some(), "`}}` closes nothing");
                    prelude.clear();
                }
                ';' => prelude.clear(),
                c => prelude.push(c),
            }
        }
        assert!(blocks.is_empty(), "a rule is left open at the end");
    }
}
