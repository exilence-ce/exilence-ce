//! The legacy import dialog (F21): the pending offer's legacy accounts with their Profiles,
//! Snapshot counts and date ranges, the preselected ones ticked. Import and Skip go to the core,
//! which tells the user about the outcome.

use ipc::command::{
    Command, LogLevel, NoArgs, RunLegacyImport, RunLegacyImportArgs, SkipLegacyImport,
};
use ipc::slices::{LegacyImportOffer, LegacyProfile};
use ipc::{Message, MessageKey};
use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::JsValue;

use crate::LOCALE;
use crate::state::State;
use crate::tauri::{self, InvokeError};

/// The dialog while an offer is pending. Clicking outside does not close it.
#[component]
pub fn LegacyImportDialog(state: State) -> impl IntoView {
    // Only a new offer resets the ticks, not `running`.
    let offer = Memo::new(move |_| state.legacy_import.with(|l| l.offer.clone()));
    move || {
        offer
            .get()
            .map(|offer| view! { <ImportDialog state offer /> })
    }
}

#[component]
fn ImportDialog(state: State, offer: LegacyImportOffer) -> impl IntoView {
    let ticked = RwSignal::new(preselected(&offer));
    let running = move || state.legacy_import.with(|l| l.running);
    let import = {
        let offer = offer.clone();
        move |_| {
            let account_names = ticked_in_order(&offer, &ticked.get_untracked());
            spawn_local(async move {
                let args = RunLegacyImportArgs { account_names };
                if let Err(error) = tauri::invoke::<RunLegacyImport>(&args).await {
                    log_failed(RunLegacyImport::NAME, &error);
                }
            });
        }
    };
    let skip = move |_| {
        spawn_local(async move {
            if let Err(error) = tauri::invoke::<SkipLegacyImport>(&NoArgs {}).await {
                log_failed(SkipLegacyImport::NAME, &error);
            }
        });
    };
    let note = move || {
        let account = state
            .session
            .with(|s| s.account_name.clone().unwrap_or_default());
        LOCALE.text(&Message::new(MessageKey::LegacyImportNote).arg("account", account))
    };
    let accounts = offer
        .accounts
        .into_iter()
        .map(|account| {
            let name = account.name.clone();
            let checked = {
                let name = name.clone();
                move || ticked.with(|t| t.contains(&name))
            };
            let toggle = {
                let name = name.clone();
                move |_| ticked.update(|t| toggle(t, &name))
            };
            let profiles = if account.profiles.is_empty() {
                view! { <li class="label">{LOCALE.key(MessageKey::LegacyImportNoProfiles)}</li> }
                    .into_any()
            } else {
                account
                    .profiles
                    .iter()
                    .map(|profile| {
                        view! {
                            <li>
                                <span class="profile-name">{profile_name(profile)}</span>
                                <span class="label">{snapshots(profile, local_date)}</span>
                            </li>
                        }
                    })
                    .collect_view()
                    .into_any()
            };
            view! {
                <li>
                    <label class="checkbox">
                        <input
                            type="checkbox"
                            prop:checked=checked
                            disabled=running
                            on:change=toggle
                        />
                        <span class="account-name">{name}</span>
                    </label>
                    <ul class="legacy-profiles">{profiles}</ul>
                </li>
            }
        })
        .collect_view();
    view! {
        <div class="modal-backdrop">
            <section class="dialog legacy-import" role="dialog" aria-modal="true">
                <h2>{LOCALE.key(MessageKey::LegacyImportTitle)}</h2>
                <p>{LOCALE.key(MessageKey::LegacyImportIntro)}</p>
                <ul class="legacy-accounts">{accounts}</ul>
                <p class="label">{note}</p>
                <div class="dialog-actions">
                    <button class="text" disabled=running on:click=skip>
                        {LOCALE.key(MessageKey::LegacyImportSkip)}
                    </button>
                    <button class="primary" disabled=running on:click=import>
                        {LOCALE.key(MessageKey::LegacyImportConfirm)}
                    </button>
                </div>
            </section>
        </div>
    }
}

/// Logs a failed command: the core already told the user.
fn log_failed(command: &str, error: &InvokeError) {
    tauri::log(LogLevel::Warn, format!("{command}: {error}"));
}

/// The preselected legacy accounts' names.
fn preselected(offer: &LegacyImportOffer) -> Vec<String> {
    offer
        .accounts
        .iter()
        .filter(|a| a.preselected)
        .map(|a| a.name.clone())
        .collect()
}

/// Ticks or unticks `name`.
fn toggle(ticked: &mut Vec<String>, name: &str) {
    match ticked.iter().position(|t| t == name) {
        Some(i) => {
            ticked.remove(i);
        }
        None => ticked.push(name.to_owned()),
    }
}

/// The ticked names in the offer's order.
fn ticked_in_order(offer: &LegacyImportOffer, ticked: &[String]) -> Vec<String> {
    offer
        .accounts
        .iter()
        .filter(|a| ticked.contains(&a.name))
        .map(|a| a.name.clone())
        .collect()
}

fn profile_name(profile: &LegacyProfile) -> String {
    if profile.name.trim().is_empty() {
        LOCALE.key(MessageKey::LegacyImportUnnamedProfile)
    } else {
        profile.name.clone()
    }
}

/// "N snapshots", with the date range of the Snapshots when there are any; `date` formats a
/// Unix ms time as a date.
fn snapshots(profile: &LegacyProfile, date: impl Fn(i64) -> String) -> String {
    let count = LOCALE.text(
        &Message::new(MessageKey::LegacyImportSnapshotCount).arg("count", profile.snapshot_count),
    );
    match (profile.first_snapshot_at, profile.last_snapshot_at) {
        (Some(first), Some(last)) => {
            let range = Message::new(MessageKey::LegacyImportDateRange)
                .arg("from", date(first))
                .arg("to", date(last));
            format!("{count} · {}", LOCALE.text(&range))
        }
        _ => count,
    }
}

/// `at` (Unix ms) as a local date, `YYYY-MM-DD`.
fn local_date(at: i64) -> String {
    let date = js_sys::Date::new(&JsValue::from_f64(at as f64));
    format!(
        "{:04}-{:02}-{:02}",
        date.get_full_year(),
        date.get_month() + 1,
        date.get_date()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ipc::slices::LegacyAccount;

    fn offer() -> LegacyImportOffer {
        let account = |name: &str, preselected| LegacyAccount {
            name: name.to_owned(),
            preselected,
            profiles: Vec::new(),
        };
        LegacyImportOffer {
            accounts: vec![
                account("Foo", true),
                account("Bar", false),
                account("Foo#1234", true),
            ],
        }
    }

    #[test]
    fn f21_preselected_accounts_start_ticked_and_go_in_offer_order() {
        let offer = offer();
        let mut ticked = preselected(&offer);
        assert_eq!(ticked, ["Foo", "Foo#1234"]);
        toggle(&mut ticked, "Foo");
        toggle(&mut ticked, "Bar");
        toggle(&mut ticked, "Foo");
        assert_eq!(ticked_in_order(&offer, &ticked), ["Foo", "Bar", "Foo#1234"]);
        toggle(&mut ticked, "Bar");
        assert_eq!(ticked_in_order(&offer, &ticked), ["Foo", "Foo#1234"]);
    }

    #[test]
    fn f21_profiles_show_their_snapshot_count_and_date_range() {
        let profile = LegacyProfile {
            name: String::new(),
            snapshot_count: 22,
            first_snapshot_at: Some(1),
            last_snapshot_at: Some(2),
        };
        let date = |at: i64| format!("day {at}");
        assert_eq!(snapshots(&profile, date), "22 snapshots · day 1 – day 2");
        assert_eq!(profile_name(&profile), "Unnamed profile");
        let empty = LegacyProfile {
            name: "Main".to_owned(),
            snapshot_count: 0,
            first_snapshot_at: None,
            last_snapshot_at: None,
        };
        assert_eq!(snapshots(&empty, date), "0 snapshots");
        assert_eq!(profile_name(&empty), "Main");
        let one = LegacyProfile {
            snapshot_count: 1,
            first_snapshot_at: Some(5),
            last_snapshot_at: Some(5),
            ..empty
        };
        assert_eq!(snapshots(&one, date), "One snapshot · day 5 – day 5");
    }
}
