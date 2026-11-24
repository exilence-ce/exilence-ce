//! The Group drawer (F13): Create and Join, or the Group name with Leave
//! (with confirmation), and the player list whose checkboxes choose who
//! counts in the Group-combined views. The core runs every action and tells
//! the user about failures; the dialog shows only its own field errors.

use ipc::command::{
    CheckGroupName, CheckGroupNameArgs, Command, CreateGroup, CreateGroupArgs, GroupForm,
    JoinGroup, JoinGroupArgs, LeaveGroup, LogLevel, NoArgs, SetPlayerSelected,
    SetPlayerSelectedArgs, SettingsPatch, UpdateSettings, UpdateSettingsArgs,
};
use ipc::{Field, IpcError, MessageKey};
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::LOCALE;
use crate::shell::{Icon, icon};
use crate::state::State;
use crate::tauri::{self, InvokeError};

/// The shortest Group password; empty means none (F13).
const MIN_PASSWORD_CHARS: usize = 6;

/// Which Group dialog is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Entry {
    Create,
    Join,
}

/// The toolbar's Group button: opens the Group drawer and closes the
/// navigation drawer (F13).
#[component]
pub fn GroupButton(state: State, open: RwSignal<bool>) -> impl IntoView {
    let toggle = move |_| {
        let opening = !open.get_untracked();
        open.set(opening);
        if opening && state.settings.with_untracked(|s| s.nav_drawer_open) {
            tauri::send::<UpdateSettings>(UpdateSettingsArgs {
                patch: SettingsPatch {
                    nav_drawer_open: Some(false),
                    ..SettingsPatch::default()
                },
            });
        }
    };
    view! {
        <button
            class="control group-button"
            data-tour="group"
            class:active=move || open.get()
            title=LOCALE.key(MessageKey::GroupOpen)
            on:click=toggle
        >
            <Icon path=icon::GROUP />
        </button>
    }
}

/// The Group drawer on the right, while `open`; it closes when the account
/// connected elsewhere (F13).
#[component]
pub fn GroupDrawer(state: State, open: RwSignal<bool>) -> impl IntoView {
    Effect::new(move |_| {
        if state.session.with(|s| s.superseded) {
            open.set(false);
        }
    });
    let dialog = RwSignal::new(None::<Entry>);
    let confirming = RwSignal::new(false);
    // A Snapshot or another operation holds the slot (F13).
    let busy = move || {
        state
            .status
            .with(|s| s.snapshot_running || s.operation_running)
    };
    let name = Memo::new(move |_| state.group.with(|g| g.name.clone()));
    view! {
        <Show when=move || open.get()>
            <aside class="group-drawer">
                <div class="group-header">
                    <button
                        class="control"
                        title=LOCALE.key(MessageKey::GroupClose)
                        on:click=move |_| open.set(false)
                    >
                        <Icon path=icon::CHEVRON_RIGHT />
                    </button>
                </div>
                {move || match name.get() {
                    Some(name) => {
                        view! {
                            <div class="group-actions">
                                <h2 class="group-name">{name}</h2>
                                <button
                                    class="primary"
                                    disabled=busy
                                    on:click=move |_| confirming.set(true)
                                >
                                    {LOCALE.key(MessageKey::GroupLeave)}
                                </button>
                            </div>
                            <PlayerList state />
                        }
                            .into_any()
                    }
                    None => {
                        view! {
                            <div class="group-actions">
                                <button
                                    class="primary"
                                    disabled=busy
                                    on:click=move |_| dialog.set(Some(Entry::Create))
                                >
                                    {LOCALE.key(MessageKey::GroupCreate)}
                                </button>
                                <button
                                    class="primary"
                                    disabled=busy
                                    on:click=move |_| dialog.set(Some(Entry::Join))
                                >
                                    {LOCALE.key(MessageKey::GroupJoin)}
                                </button>
                            </div>
                        }
                            .into_any()
                    }
                }}
            </aside>
        </Show>
        {move || {
            dialog
                .get()
                .map(|entry| {
                    let on_close = Callback::new(move |()| dialog.set(None));
                    view! { <GroupDialog entry on_close /> }
                })
        }}
        <Show when=move || confirming.get()>
            <ConfirmLeave on_close=Callback::new(move |()| confirming.set(false)) />
        </Show>
    }
}

/// The players, each with the checkbox that makes them count (F13).
#[component]
fn PlayerList(state: State) -> impl IntoView {
    let players = move || {
        state.group.with(|g| {
            g.members
                .iter()
                .map(|member| {
                    let account_id = member.account_id.clone();
                    let selected = member.selected;
                    let toggle = move |_| {
                        tauri::send::<SetPlayerSelected>(SetPlayerSelectedArgs {
                            account_id: account_id.clone(),
                            selected: !selected,
                        });
                    };
                    let initial = member.account_name.chars().next().unwrap_or('?');
                    view! {
                        <li>
                            <label>
                                <span class="avatar">{initial.to_string()}</span>
                                <span class="player-name">{member.account_name.clone()}</span>
                                <input type="checkbox" prop:checked=selected on:change=toggle />
                            </label>
                        </li>
                    }
                })
                .collect_view()
        })
    };
    view! {
        <section class="player-list">
            <h3>{LOCALE.key(MessageKey::GroupPlayers)}</h3>
            <ul>{players}</ul>
        </section>
    }
}

/// Asks before leaving the Group (F13).
#[component]
fn ConfirmLeave(on_close: Callback<()>) -> impl IntoView {
    let leaving = RwSignal::new(false);
    let leave = move |_| {
        leaving.set(true);
        spawn_local(async move {
            let left = tauri::invoke::<LeaveGroup>(&NoArgs {}).await;
            leaving.set(false);
            match left {
                Ok(()) => on_close.run(()),
                Err(error) => log_failed(LeaveGroup::NAME, &error),
            }
        });
    };
    view! {
        // Clicking outside does not close it.
        <div class="modal-backdrop">
            <section class="dialog confirm" role="dialog" aria-modal="true">
                <h2>{LOCALE.key(MessageKey::GroupLeaveTitle)}</h2>
                <p>{LOCALE.key(MessageKey::GroupLeaveBody)}</p>
                <div class="dialog-actions">
                    <button class="text" disabled=leaving on:click=move |_| on_close.run(())>
                        {LOCALE.key(MessageKey::Cancel)}
                    </button>
                    <button class="primary" disabled=leaving on:click=leave>
                        {LOCALE.key(MessageKey::GroupLeave)}
                    </button>
                </div>
            </section>
        </div>
    }
}

/// The create or join dialog: the name, checked when its field loses focus
/// (taken for create, not found for join), with a random one and a re-roll
/// when creating; the optional password of at least 6 characters (F13).
#[component]
fn GroupDialog(entry: Entry, on_close: Callback<()>) -> impl IntoView {
    let (title, submit_label) = match entry {
        Entry::Create => (MessageKey::GroupCreateTitle, MessageKey::GroupCreate),
        Entry::Join => (MessageKey::GroupJoinTitle, MessageKey::GroupJoin),
    };
    let initial = match entry {
        Entry::Create => random_name(
            js_sys::Math::random(),
            js_sys::Math::random(),
            js_sys::Math::random(),
        ),
        Entry::Join => String::new(),
    };
    let name = RwSignal::new(initial.clone());
    let password = RwSignal::new(String::new());
    let name_error = RwSignal::new(None::<MessageKey>);
    let password_error = RwSignal::new(None::<MessageKey>);
    let saving = RwSignal::new(false);

    // Asks whether `checked` exists; a late answer for a name no longer in
    // the field is dropped. A failed check shows nothing.
    let check = move |checked: String| {
        if checked.trim().is_empty() {
            return;
        }
        spawn_local(async move {
            let args = CheckGroupNameArgs {
                name: checked.clone(),
            };
            let exists = tauri::invoke::<CheckGroupName>(&args).await;
            if name.get_untracked() != checked {
                return;
            }
            match exists {
                Ok(exists) => name_error.set(name_check_error(entry, exists)),
                Err(error) => log_failed(CheckGroupName::NAME, &error),
            }
        });
    };
    if entry == Entry::Create {
        check(initial);
    }
    let reroll = move |_| {
        let rolled = random_name(
            js_sys::Math::random(),
            js_sys::Math::random(),
            js_sys::Math::random(),
        );
        name_error.set(None);
        name.set(rolled.clone());
        check(rolled);
    };
    let blank = move || name.with(|n| n.trim().is_empty());
    let short = move || password.with(|p| password_too_short(p));
    let submittable = move || {
        !saving.get()
            && !blank()
            && !short()
            && name_error.get().is_none()
            && password_error.get().is_none()
    };
    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        if !submittable() {
            return;
        }
        saving.set(true);
        let group = GroupForm {
            name: name.get_untracked(),
            password: password.get_untracked(),
        };
        spawn_local(async move {
            let entered = match entry {
                Entry::Create => tauri::invoke::<CreateGroup>(&CreateGroupArgs { group }).await,
                Entry::Join => tauri::invoke::<JoinGroup>(&JoinGroupArgs { group }).await,
            };
            saving.set(false);
            match entered {
                Ok(()) => on_close.run(()),
                Err(InvokeError::Command(IpcError::Validation { field, key })) => match field {
                    Field::GroupPassword => password_error.set(Some(key)),
                    _ => name_error.set(Some(key)),
                },
                Err(error) => log_failed("enter group", &error),
            }
        });
    };

    view! {
        // Clicking outside does not close it.
        <div class="modal-backdrop">
            <form class="dialog group-dialog" role="dialog" aria-modal="true" on:submit=submit>
                <h2>{LOCALE.key(title)}</h2>
                <label for="group-name">{LOCALE.key(MessageKey::GroupName)}</label>
                <div class="name-field">
                    <input
                        id="group-name"
                        type="text"
                        required
                        autofocus
                        placeholder=LOCALE.key(MessageKey::GroupNamePlaceholder)
                        prop:value=move || name.get()
                        on:input=move |ev| {
                            name_error.set(None);
                            name.set(event_target_value(&ev));
                        }
                        on:blur=move |_| check(name.get_untracked())
                    />
                    <Show when=move || entry == Entry::Create>
                        <button
                            type="button"
                            class="control"
                            title=LOCALE.key(MessageKey::GroupGenerateName)
                            on:click=reroll
                        >
                            <Icon path=icon::DICE />
                        </button>
                    </Show>
                </div>
                {move || {
                    name_error
                        .get()
                        .or(blank().then_some(MessageKey::ErrorGroupNameRequired))
                        .map(|key| view! { <p class="field-error">{LOCALE.key(key)}</p> })
                }}
                <label for="group-password">{LOCALE.key(MessageKey::GroupPassword)}</label>
                <input
                    id="group-password"
                    type="password"
                    placeholder=LOCALE.key(MessageKey::GroupPasswordPlaceholder)
                    prop:value=move || password.get()
                    on:input=move |ev| {
                        password_error.set(None);
                        password.set(event_target_value(&ev));
                    }
                />
                {move || match password_error
                    .get()
                    .or(short().then_some(MessageKey::ErrorGroupPasswordTooShort))
                {
                    Some(key) => view! { <p class="field-error">{LOCALE.key(key)}</p> }.into_any(),
                    None => {
                        view! { <p class="helper">{LOCALE.key(MessageKey::GroupPasswordHelper)}</p> }
                            .into_any()
                    }
                }}
                <div class="dialog-actions">
                    <button type="button" class="text" disabled=saving on:click=move |_| on_close.run(())>
                        {LOCALE.key(MessageKey::Cancel)}
                    </button>
                    <button type="submit" class="primary" disabled=move || !submittable()>
                        {LOCALE.key(submit_label)}
                    </button>
                </div>
            </form>
        </div>
    }
}

/// The name field's error for a checked name: taken when creating, not
/// found when joining (F13).
fn name_check_error(entry: Entry, exists: bool) -> Option<MessageKey> {
    match (entry, exists) {
        (Entry::Create, true) => Some(MessageKey::ErrorGroupNameTaken),
        (Entry::Join, false) => Some(MessageKey::ErrorGroupNotFound),
        _ => None,
    }
}

/// A password is optional; given, it has at least 6 characters (F13).
fn password_too_short(password: &str) -> bool {
    !password.is_empty() && password.chars().count() < MIN_PASSWORD_CHARS
}

fn log_failed(command: &str, error: &InvokeError) {
    tauri::log(LogLevel::Warn, format!("{command}: {error}"));
}

const NAME_PREFIXES: [&str; 6] = ["Divine", "Strong", "Swift", "Rich", "Humble", "Fine"];
const NAME_SUFFIXES: [&str; 8] = [
    "Exiles",
    "Slayers",
    "Assassins",
    "Duelists",
    "Witches",
    "Farmers",
    "Grinders",
    "Templars",
];

/// A random Group name from three numbers in `[0, 1)`, as legacy makes them
/// (`generateGroupName`): prefix, suffix and a number from 100 to 999.
fn random_name(prefix: f64, suffix: f64, number: f64) -> String {
    let pick = |words: &[&'static str], r: f64| {
        words[((r * words.len() as f64) as usize).min(words.len() - 1)]
    };
    let number = 100 + ((number * 900.0) as u32).min(899);
    format!(
        "{}{}{number}",
        pick(&NAME_PREFIXES, prefix),
        pick(&NAME_SUFFIXES, suffix)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f13_random_names_follow_legacy() {
        assert_eq!(random_name(0.0, 0.0, 0.0), "DivineExiles100");
        assert_eq!(random_name(0.99, 0.99, 0.9999), "FineTemplars999");
        assert_eq!(random_name(0.5, 0.5, 0.5), "RichWitches550");
    }

    #[test]
    fn f13_name_check_errors_depend_on_create_or_join() {
        assert_eq!(
            name_check_error(Entry::Create, true),
            Some(MessageKey::ErrorGroupNameTaken)
        );
        assert_eq!(name_check_error(Entry::Create, false), None);
        assert_eq!(
            name_check_error(Entry::Join, false),
            Some(MessageKey::ErrorGroupNotFound)
        );
        assert_eq!(name_check_error(Entry::Join, true), None);
    }

    #[test]
    fn f13_a_password_is_optional_but_at_least_6_characters() {
        assert!(!password_too_short(""));
        assert!(password_too_short("12345"));
        assert!(!password_too_short("123456"));
        assert!(!password_too_short("äöüßéè"), "characters, not bytes");
    }
}
