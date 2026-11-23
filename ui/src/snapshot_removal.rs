//! The toolbar's remove-Snapshots button and its dialog (F12): the active
//! Profile's Snapshots with their dates and values, marked when their items
//! are gone, select all, cancel and remove. Removing works offline (D26);
//! the core tells the user about failures.

use ipc::command::{
    Command, ListSnapshots, LogLevel, NoArgs, RemoveSnapshots, RemoveSnapshotsArgs, SnapshotEntry,
};
use ipc::{Message, MessageKey};
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::LOCALE;
use crate::shell::{Icon, icon};
use crate::state::State;
use crate::tauri::{self, InvokeError};

/// Remove Snapshots, disabled while a Snapshot runs or another operation
/// holds the slot, and while the active Profile has no Snapshot (F12).
#[component]
pub fn RemoveSnapshotsControl(state: State) -> impl IntoView {
    let open = RwSignal::new(false);
    let disabled = move || {
        state
            .status
            .with(|s| s.snapshot_running || s.operation_running)
            || state.profiles.with(|p| p.active_profile_id.is_none())
            || state.net_worth.with(|n| n.snapshot_count == 0)
    };
    view! {
        <button
            class="control remove-snapshots"
            title=LOCALE.key(MessageKey::SnapshotsRemove)
            disabled=disabled
            on:click=move |_| open.set(true)
        >
            <Icon path=icon::DELETE_SWEEP />
        </button>
        <Show when=move || open.get()>
            <RemoveSnapshotsDialog on_close=Callback::new(move |()| open.set(false)) />
        </Show>
    }
}

/// The removal list, loaded when the dialog opens. A click outside closes
/// it, as legacy's.
#[component]
fn RemoveSnapshotsDialog(on_close: Callback<()>) -> impl IntoView {
    let entries = RwSignal::new(None::<Vec<SnapshotEntry>>);
    let checked = RwSignal::new(Vec::<String>::new());
    let removing = RwSignal::new(false);
    spawn_local(async move {
        match tauri::invoke::<ListSnapshots>(&NoArgs {}).await {
            Ok(listed) => entries.set(Some(listed)),
            Err(error) => {
                log_failed(ListSnapshots::NAME, &error);
                entries.set(Some(Vec::new()));
            }
        }
    });
    let select_all = move |_| {
        let all = entries.with(|e| e.iter().flatten().map(|s| s.id.clone()).collect());
        checked.set(all);
    };
    let remove = move |_| {
        let snapshot_ids = checked.get_untracked();
        removing.set(true);
        spawn_local(async move {
            let removed =
                tauri::invoke::<RemoveSnapshots>(&RemoveSnapshotsArgs { snapshot_ids }).await;
            removing.set(false);
            match removed {
                Ok(()) => on_close.run(()),
                Err(error) => log_failed(RemoveSnapshots::NAME, &error),
            }
        });
    };
    let list = move || {
        let selected = checked.get();
        entries.with(|e| {
            e.iter()
                .flatten()
                .map(|entry| {
                    let id = entry.id.clone();
                    let is_checked = selected.contains(&entry.id);
                    let text = LOCALE.text(&entry_message(
                        &local_time(entry.created_at),
                        entry.total_value,
                    ));
                    view! {
                        <li>
                            <label>
                                <input
                                    type="checkbox"
                                    prop:checked=is_checked
                                    on:change=move |_| checked.update(|c| toggle(c, &id))
                                />
                                <span class="entry">{text}</span>
                                <Show when={
                                    let item_detail = entry.item_detail;
                                    move || !item_detail
                                }>
                                    <span
                                        class="warning"
                                        title=LOCALE.key(MessageKey::SnapshotsItemsMissing)
                                    >
                                        <Icon path=icon::WARNING />
                                    </span>
                                </Show>
                            </label>
                        </li>
                    }
                })
                .collect_view()
        })
    };
    view! {
        <div class="modal-backdrop" on:click=move |_| on_close.run(())>
            <section
                class="dialog remove-snapshots-dialog"
                role="dialog"
                aria-modal="true"
                on:click=|ev| ev.stop_propagation()
            >
                <h2>{LOCALE.key(MessageKey::SnapshotsRemoveTitle)}</h2>
                <ul class="snapshot-list">{list}</ul>
                <div class="dialog-actions">
                    <button class="text" disabled=removing on:click=select_all>
                        {LOCALE.key(MessageKey::SelectAll)}
                    </button>
                    <button class="text" disabled=removing on:click=move |_| on_close.run(())>
                        {LOCALE.key(MessageKey::Cancel)}
                    </button>
                    <button
                        class="primary"
                        disabled=move || removing.get() || checked.with(Vec::is_empty)
                        on:click=remove
                    >
                        {LOCALE.key(MessageKey::SnapshotsRemove)}
                    </button>
                </div>
            </section>
        </div>
    }
}

/// Checks `id` when unchecked, unchecks it otherwise.
fn toggle(checked: &mut Vec<String>, id: &str) {
    match checked.iter().position(|c| c == id) {
        Some(at) => {
            checked.remove(at);
        }
        None => checked.push(id.to_owned()),
    }
}

/// `Snapshot YYYY-MM-DD HH:mm, value: X.XXc` (F12).
fn entry_message(date: &str, total_value: f64) -> Message {
    Message::new(MessageKey::SnapshotsRemoveEntry)
        .arg("date", date)
        .arg("value", format!("{total_value:.2}"))
}

/// `at` (Unix ms) in local time as `YYYY-MM-DD HH:mm`.
fn local_time(at: i64) -> String {
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(at as f64));
    format_date(
        date.get_full_year(),
        date.get_month() + 1,
        date.get_date(),
        date.get_hours(),
        date.get_minutes(),
    )
}

fn format_date(year: u32, month: u32, day: u32, hour: u32, minute: u32) -> String {
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}")
}

fn log_failed(command: &str, error: &InvokeError) {
    tauri::log(LogLevel::Warn, format!("{command}: {error}"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f12_entries_read_snapshot_date_and_value() {
        let date = format_date(2027, 1, 5, 8, 3);
        assert_eq!(date, "2027-01-05 08:03");
        assert_eq!(
            LOCALE.text(&entry_message(&date, 1234.567)),
            "Snapshot 2027-01-05 08:03, value: 1234.57c"
        );
    }

    #[test]
    fn f12_toggle_checks_and_unchecks() {
        let mut checked = Vec::new();
        toggle(&mut checked, "s1");
        toggle(&mut checked, "s2");
        assert_eq!(checked, ["s1", "s2"]);
        toggle(&mut checked, "s1");
        assert_eq!(checked, ["s2"]);
    }
}
