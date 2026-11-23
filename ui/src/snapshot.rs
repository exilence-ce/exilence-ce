//! The toolbar's Snapshot and Cancel buttons with the allowance hint (F3,
//! F7), and the Net worth widgets: total with its change and the currency
//! switch, income with Reset, Snapshot count with the time since the last
//! Snapshot (F8, F9).

use std::time::Duration;

use ipc::command::{CancelSnapshot, NoArgs, ResetIncome, TakeSnapshot};
use ipc::slices::{NetWorth, PriceAvailability, Prices, RateLimit, Session, Status};
use ipc::{Message, MessageKey};
use leptos::prelude::*;

use crate::LOCALE;
use crate::charts::Sparkline;
use crate::currency::{CurrencySwitch, amount};
use crate::notifications::time_ago;
use crate::shell::{Icon, icon};
use crate::state::State;
use crate::tauri;

/// From this many Snapshots on, the count shows `1000+`: retention keeps no
/// more (F8, F12).
const SNAPSHOT_CAP: u32 = 1000;

/// Snapshot, enabled only when a Snapshot can start (F3) and no cooldown
/// runs (`cooling_down`, F7); Cancel while a Snapshot runs until it is being
/// saved; the allowance hint while one runs.
#[component]
pub fn SnapshotControls(state: State, cooling_down: Signal<bool>) -> impl IntoView {
    let can_take = move || {
        !cooling_down.get()
            && state.session.with(|session| {
                state.prices.with(|prices| {
                    state
                        .status
                        .with(|status| can_snapshot(session, prices, status))
                })
            })
    };
    let cancellable = move || {
        state
            .status
            .with(|s| s.snapshot_running && !s.snapshot_saving)
    };
    let running = move || state.status.with(|s| s.snapshot_running);
    view! {
        <div class="snapshot-area" data-tour="snapshot">
            <Show when=running>
                <span
                    class="allowance-hint"
                    title=move || state.rate_limit.with(|r| LOCALE.text(&allowance_hint(r)))
                >
                    <Icon path=icon::INFO />
                </span>
            </Show>
            <button
                class="snapshot"
                disabled=move || !can_take()
                on:click=move |_| { tauri::send::<TakeSnapshot>(NoArgs {}) }
            >
                <Icon path=icon::UPDATE />
                {LOCALE.key(MessageKey::SnapshotTake)}
            </button>
            <button
                class="control"
                title=LOCALE.key(MessageKey::SnapshotCancel)
                disabled=move || !cancellable()
                on:click=move |_| { tauri::send::<CancelSnapshot>(NoArgs {}) }
            >
                <Icon path=icon::CANCEL />
            </button>
        </div>
    }
}

/// Every start condition of F3 the slices show, the cooldown aside: the
/// session is ready, the league has tabs, prices of the price league are
/// loaded and not updating, and the operation slot is free.
fn can_snapshot(session: &Session, prices: &Prices, status: &Status) -> bool {
    session.ready
        && session.league_has_tabs
        && prices.availability == PriceAvailability::Available
        && !status.snapshot_running
        && !status.operation_running
}

/// "GGG allows {max} requests per {period} seconds; …", without numbers
/// before GGG's are known (F7).
fn allowance_hint(rate_limit: &RateLimit) -> Message {
    match rate_limit.allowance {
        Some(allowance) => Message::new(MessageKey::SnapshotAllowance)
            .arg("max", allowance.max)
            .arg("period", allowance.period),
        None => MessageKey::SnapshotAllowanceUnknown.into(),
    }
}

/// The Net worth widgets of the active Profile (F8): total with its change
/// and the currency switch (F9), income with Reset (hidden in a Group), the
/// Snapshot count with the time since the last Snapshot, which ticks here
/// every second from the pushed time. Values convert in the chosen currency.
#[component]
pub fn NetWorthWidgets(state: State) -> impl IntoView {
    let now = RwSignal::new(js_sys::Date::now() as i64);
    if let Ok(tick) = set_interval_with_handle(
        move || now.set(js_sys::Date::now() as i64),
        Duration::from_secs(1),
    ) {
        on_cleanup(move || tick.clear());
    }
    let net_worth = move |read: fn(&NetWorth) -> String| state.net_worth.with(read);
    let total = move || amount(state, state.net_worth.with(|n| n.total), false);
    let change = move || amount(state, state.net_worth.with(|n| n.change), true);
    let change_class = move || change_class(state.net_worth.with(|n| n.change));
    let income = move || amount(state, state.net_worth.with(|n| n.income), true);
    let in_group = move || state.net_worth.with(|n| n.group_combined);
    let since = move || {
        state
            .net_worth
            .with(|n| n.last_snapshot_at)
            .map(|at| LOCALE.text(&time_ago(now.get(), at)))
    };
    view! {
        <div class="widgets">
            <div class="widget">
                <div class="widget-top">
                    <Icon path=icon::MONETIZATION_ON />
                    <Sparkline values=Signal::derive(move || {
                        state.net_worth.with(|n| n.sparkline.iter().copied().map(Some).collect())
                    }) />
                    <span class="widget-value currency">{total}</span>
                    <CurrencySwitch state />
                </div>
                <div class="widget-bottom">
                    <span class="widget-title">{LOCALE.key(MessageKey::NetWorthTotal)}</span>
                    <span class=change_class title=LOCALE.key(MessageKey::NetWorthChangeTooltip)>
                        {change}
                    </span>
                </div>
            </div>
            <div class="widget">
                <div class="widget-top">
                    <Icon path=icon::TRENDING_UP />
                    <span class="widget-value currency">{income}</span>
                    <span class="widget-suffix">
                        {LOCALE.key(MessageKey::NetWorthHourSuffix)}
                    </span>
                    <Show when=move || !in_group()>
                        <button
                            class="adornment"
                            data-tour="reset-income"
                            title=LOCALE.key(MessageKey::NetWorthIncomeReset)
                            on:click=move |_| { tauri::send::<ResetIncome>(NoArgs {}) }
                        >
                            <Icon path=icon::CANCEL />
                        </button>
                    </Show>
                </div>
                <div class="widget-bottom">
                    <span class="widget-title">{LOCALE.key(MessageKey::NetWorthIncome)}</span>
                    <span class="since" title=LOCALE.key(MessageKey::NetWorthIncomeTooltip)>
                        {LOCALE.key(MessageKey::NetWorthIncomeBasedOn)}
                    </span>
                </div>
            </div>
            <div class="widget">
                <div class="widget-top">
                    <Icon path=icon::UPDATE />
                    <span class="widget-value">
                        {move || net_worth(|n| snapshot_count(n.snapshot_count))}
                    </span>
                </div>
                <div class="widget-bottom">
                    <span class="widget-title">
                        {LOCALE.key(MessageKey::NetWorthSnapshotCount)}
                    </span>
                    <span class="since" title=LOCALE.key(MessageKey::NetWorthTimeSinceTooltip)>
                        {since}
                    </span>
                </div>
            </div>
        </div>
    }
}

/// Green for a gain, red for a loss (F8).
fn change_class(change: f64) -> &'static str {
    if change > 0.0 {
        "change positive"
    } else if change < 0.0 {
        "change negative"
    } else {
        "change"
    }
}

/// The Snapshot count, `1000+` at the cap (F8).
fn snapshot_count(count: u32) -> String {
    if count >= SNAPSHOT_CAP {
        format!("{SNAPSHOT_CAP}+")
    } else {
        count.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ipc::slices::Allowance;

    #[test]
    fn f3_snapshot_is_enabled_only_under_the_start_conditions() {
        let session = Session {
            ready: true,
            league_has_tabs: true,
            ..Session::default()
        };
        let prices = Prices {
            availability: PriceAvailability::Available,
            ..Prices::default()
        };
        let idle = Status::default();
        assert!(can_snapshot(&session, &prices, &idle));

        let not_ready = Session {
            ready: false,
            ..session.clone()
        };
        let no_tabs = Session {
            league_has_tabs: false,
            ..session.clone()
        };
        assert!(!can_snapshot(&not_ready, &prices, &idle));
        assert!(!can_snapshot(&no_tabs, &prices, &idle));
        for availability in [
            PriceAvailability::NotLoaded,
            PriceAvailability::Loading,
            PriceAvailability::Unavailable,
        ] {
            let prices = Prices {
                availability,
                ..Prices::default()
            };
            assert!(!can_snapshot(&session, &prices, &idle), "{availability:?}");
        }
        let running = Status {
            snapshot_running: true,
            operation_running: true,
            ..Status::default()
        };
        let other_operation = Status {
            operation_running: true,
            ..Status::default()
        };
        assert!(!can_snapshot(&session, &prices, &running));
        assert!(!can_snapshot(&session, &prices, &other_operation));
    }

    #[test]
    fn f7_allowance_hint_names_ggg_numbers_once_known() {
        let known = RateLimit {
            allowance: Some(Allowance {
                max: 30,
                period: 300,
            }),
            ..RateLimit::default()
        };
        assert_eq!(
            LOCALE.text(&allowance_hint(&known)),
            "GGG allows 30 requests per 300 seconds; large snapshots can take a while."
        );
        assert_eq!(
            LOCALE.text(&allowance_hint(&RateLimit::default())),
            "GGG limits how many requests Exilence CE NG may send; large snapshots can take a while."
        );
    }

    #[test]
    fn f8_widget_values() {
        assert_eq!(change_class(12.0), "change positive");
        assert_eq!(change_class(-3.0), "change negative");
        assert_eq!(change_class(0.0), "change");
        assert_eq!(snapshot_count(999), "999");
        assert_eq!(snapshot_count(1000), "1000+");
    }
}
