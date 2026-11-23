//! The Net worth overlay window (F14): net worth and income of the active
//! Profile, or the Group's while in a Group, in the chosen currency. Dragged
//! by its header, closed with its X.

use ipc::command::{CloseOverlay, NoArgs};
use ipc::{MessageKey, SliceName};
use leptos::prelude::*;

use crate::LOCALE;
use crate::currency::amount;
use crate::shell::{Icon, icon};
use crate::state::State;
use crate::tauri;

/// The overlay's route.
pub const PATH: &str = "/overlay";

/// The slices the overlay shows: values, their rates and the currency.
pub const SLICES: &[SliceName] = &[SliceName::Prices, SliceName::NetWorth, SliceName::Settings];

#[component]
pub fn Overlay(state: State) -> impl IntoView {
    let total = move || amount(state, state.net_worth.with(|n| n.total), false);
    let income = move || amount(state, state.net_worth.with(|n| n.income), true);
    view! {
        <div class="overlay">
            <header class="overlay-header" data-tauri-drag-region="deep">
                <span class="title">{LOCALE.key(MessageKey::NetWorth)}</span>
                <button
                    class="control"
                    title=LOCALE.key(MessageKey::OverlayClose)
                    on:click=|_| { tauri::send::<CloseOverlay>(NoArgs {}) }
                >
                    <Icon path=icon::CLOSE />
                </button>
            </header>
            <dl class="overlay-values">
                <dt>{LOCALE.key(MessageKey::NetWorthTotal)}</dt>
                <dd class="currency">{total}</dd>
                <dt>{LOCALE.key(MessageKey::NetWorthIncome)}</dt>
                <dd>
                    <span class="currency">{income}</span>
                    " "
                    <span class="secondary">{LOCALE.key(MessageKey::NetWorthHourSuffix)}</span>
                </dd>
            </dl>
        </div>
    }
}
