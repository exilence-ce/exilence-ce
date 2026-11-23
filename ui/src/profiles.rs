//! The toolbar's Profile controls and the Profile dialog (F2): switch, edit,
//! create and remove (with confirmation). Every change goes through the
//! core, which tells the user about failures; the form only shows its own
//! field errors.

use ipc::command::{
    Command, CreateProfile, CreateProfileArgs, GetProfileOptions, GetProfileOptionsArgs, LogLevel,
    NewProfile, ProfileOptions, RemoveProfile, RemoveProfileArgs, SetActiveProfile,
    SetActiveProfileArgs, StashTab, UpdateProfile, UpdateProfileArgs,
};
use ipc::slices::Profile;
use ipc::{Field, IpcError, MessageKey};
use leptos::html;
use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::JsCast;

use crate::LOCALE;
use crate::menu::{OutlinedSelect, every_frame_while};
use crate::shell::{Icon, icon};
use crate::state::State;
use crate::tauri::{self, InvokeError};

/// Selected tabs from which the dialog warns about a long Snapshot (F2).
const MANY_TABS: usize = 5;

/// What the Profile dialog edits.
#[derive(Clone, Debug, PartialEq)]
enum Dialog {
    Create,
    Edit(Profile),
}

/// Edit, the switcher, create and remove, all disabled while a Snapshot
/// runs or another operation holds the slot (F2). Remove is disabled with a single Profile.
#[component]
pub fn ProfileControls(state: State) -> impl IntoView {
    let dialog = RwSignal::new(None::<Dialog>);
    let confirming = RwSignal::new(false);
    // A running Snapshot, or another operation holding the slot.
    let busy = move || {
        state
            .status
            .with(|s| s.snapshot_running || s.operation_running)
    };
    let active = move || {
        state.profiles.with(|p| {
            p.profiles
                .iter()
                .find(|profile| Some(&profile.id) == p.active_profile_id.as_ref())
                .cloned()
        })
    };
    // The shown choice follows the profiles slice, so a failed switch shows
    // the unchanged active Profile.
    let switch = Callback::new(move |profile_id: String| {
        spawn_local(async move {
            let args = SetActiveProfileArgs { profile_id };
            if let Err(error) = tauri::invoke::<SetActiveProfile>(&args).await {
                log_failed(SetActiveProfile::NAME, &error);
            }
        });
    });
    let options = Signal::derive(move || {
        state.profiles.with(|p| {
            p.profiles
                .iter()
                .map(|profile| (profile.id.clone(), profile.name.clone()))
                .collect()
        })
    });
    view! {
        <div class="profile-area" data-tour="profile">
            <button
                class="control"
                title=LOCALE.key(MessageKey::ProfileEdit)
                disabled=move || busy() || active().is_none()
                on:click=move |_| dialog.set(active().map(Dialog::Edit))
            >
                <Icon path=icon::SETTINGS />
            </button>
            <OutlinedSelect
                id="profile-switcher"
                title=LOCALE.key(MessageKey::ProfileSwitcher)
                standard=true
                options
                value=Signal::derive(move || {
                    state.profiles.with(|p| p.active_profile_id.clone().unwrap_or_default())
                })
                on_change=switch
                disabled=Signal::derive(move || {
                    busy() || state.profiles.with(|p| p.profiles.is_empty())
                })
                error=Signal::stored(false)
            />
            <button
                class="control"
                title=LOCALE.key(MessageKey::ProfileCreate)
                disabled=busy
                on:click=move |_| dialog.set(Some(Dialog::Create))
            >
                <Icon path=icon::ADD />
            </button>
            <button
                class="control"
                title=LOCALE.key(MessageKey::ProfileRemove)
                disabled=move || {
                    busy() || state.profiles.with(|p| p.profiles.len() < 2)
                }
                on:click=move |_| confirming.set(true)
            >
                <Icon path=icon::DELETE />
            </button>
        </div>
        {move || {
            dialog
                .get()
                .map(|editing| {
                    let on_close = Callback::new(move |()| dialog.set(None));
                    view! { <ProfileDialog state editing on_close /> }
                })
        }}
        <Show when=move || confirming.get()>
            <ConfirmRemove state on_close=Callback::new(move |()| confirming.set(false)) />
        </Show>
    }
}

/// Asks before removing the active Profile; the core switches to another
/// one first and deletes the removed one's Snapshots (F2).
#[component]
fn ConfirmRemove(state: State, on_close: Callback<()>) -> impl IntoView {
    let removing = RwSignal::new(false);
    let remove = move |_| {
        let Some(profile_id) = state.profiles.with(|p| p.active_profile_id.clone()) else {
            return;
        };
        removing.set(true);
        spawn_local(async move {
            let removed = tauri::invoke::<RemoveProfile>(&RemoveProfileArgs { profile_id }).await;
            removing.set(false);
            match removed {
                Ok(()) => on_close.run(()),
                Err(error) => log_failed(RemoveProfile::NAME, &error),
            }
        });
    };
    view! {
        // Clicking outside does not close it.
        <div class="modal-backdrop">
            <section class="dialog confirm" role="dialog" aria-modal="true">
                <h2>{LOCALE.key(MessageKey::ProfileRemoveTitle)}</h2>
                <p>{LOCALE.key(MessageKey::ProfileRemoveBody)}</p>
                <div class="dialog-actions">
                    <button class="text" disabled=removing on:click=move |_| on_close.run(())>
                        {LOCALE.key(MessageKey::Cancel)}
                    </button>
                    <button class="primary" disabled=removing on:click=remove>
                        {LOCALE.key(MessageKey::ProfileRemoveConfirm)}
                    </button>
                </div>
            </section>
        </div>
    }
}

/// The Profile dialog: create a Profile or edit `editing`, with the F2 field
/// rules of [`ProfileForm`]. Clicking outside does not close it.
#[component]
fn ProfileDialog(state: State, editing: Dialog, on_close: Callback<()>) -> impl IntoView {
    let (title, submit_label, edited_id, initial) = match &editing {
        Dialog::Create => (
            MessageKey::ProfileDialogCreateTitle,
            MessageKey::ProfileDialogCreate,
            None,
            ProfileForm::new(random_name(js_sys::Math::random(), js_sys::Math::random())),
        ),
        Dialog::Edit(profile) => (
            MessageKey::ProfileDialogEditTitle,
            MessageKey::ProfileDialogSave,
            Some(profile.id.clone()),
            ProfileForm::from_profile(profile),
        ),
    };
    let form = RwSignal::new(initial);
    let options = RwSignal::new(None::<ProfileOptions>);
    let loading = RwSignal::new(true);
    let saving = RwSignal::new(false);
    let name_error = RwSignal::new(None::<MessageKey>);

    // Loads the choices for `league` (the first league when `None`); a
    // late answer for a league no longer chosen is dropped.
    let load = move |league: Option<String>| {
        loading.set(true);
        spawn_local(async move {
            let args = GetProfileOptionsArgs {
                league: league.clone(),
            };
            let loaded = tauri::invoke::<GetProfileOptions>(&args).await;
            let current = form.with_untracked(|f| f.league.clone());
            if league.as_ref().is_some_and(|l| *l != current) {
                return;
            }
            match loaded {
                Ok(loaded) => {
                    let price_leagues = state.profiles.with_untracked(|p| p.price_leagues.clone());
                    form.update(|f| f.options_loaded(&loaded, &price_leagues));
                    options.set(Some(loaded));
                }
                Err(error) => log_failed(GetProfileOptions::NAME, &error),
            }
            loading.set(false);
        });
    };
    load(form.with_untracked(|f| Some(f.league.clone()).filter(|l| !l.is_empty())));

    let characters =
        move || options.with(|o| o.as_ref().map(|o| o.characters.clone()).unwrap_or_default());
    let no_characters =
        move || options.with(|o| o.as_ref().is_some_and(|o| o.characters.is_empty()));
    let submittable =
        move || !loading.get() && !saving.get() && form.with(|f| f.submittable(&characters()));
    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        if !submittable() {
            return;
        }
        saving.set(true);
        name_error.set(None);
        let edited_id = edited_id.clone();
        let filled = form.get_untracked();
        spawn_local(async move {
            let saved = match edited_id {
                None => {
                    let args = CreateProfileArgs {
                        profile: filled.into_new(),
                    };
                    tauri::invoke::<CreateProfile>(&args).await
                }
                Some(id) => {
                    let args = UpdateProfileArgs {
                        profile: filled.into_profile(id),
                    };
                    tauri::invoke::<UpdateProfile>(&args).await
                }
            };
            saving.set(false);
            match saved {
                Ok(()) => on_close.run(()),
                Err(InvokeError::Command(IpcError::Validation {
                    field: Field::ProfileName,
                    key,
                })) => name_error.set(Some(key)),
                Err(error) => log_failed("save profile", &error),
            }
        });
    };

    let price_league_options =
        Signal::derive(move || state.profiles.with(|p| pairs(p.price_leagues.clone())));
    let league_options = Signal::derive(move || {
        let chosen = form.with(|f| f.league.clone());
        let mut leagues =
            options.with(|o| o.as_ref().map(|o| o.leagues.clone()).unwrap_or_default());
        // A Profile's `Standard` fallback without characters there.
        if !chosen.is_empty() && !leagues.contains(&chosen) {
            leagues.push(chosen);
        }
        pairs(leagues)
    });
    let change_league = Callback::new(move |league: String| {
        form.update(|f| f.change_league(league.clone()));
        options.set(None);
        load(Some(league));
    });
    // The empty value is "None".
    let character_options = Signal::derive(move || {
        let none = (String::new(), LOCALE.key(MessageKey::ProfileNoCharacter));
        std::iter::once(none).chain(pairs(characters())).collect()
    });
    let includes_disabled = move || !form.with(ProfileForm::includes_enabled);
    let name_error_key = move || {
        let blank = form.with(|f| f.name.trim().is_empty());
        name_error
            .get()
            .or(blank.then_some(MessageKey::ErrorProfileNameRequired))
    };
    let checkbox = move |label: MessageKey,
                         get: fn(&ProfileForm) -> bool,
                         set: fn(&mut ProfileForm, bool)| {
        let checked = move || form.with(get);
        view! {
            <label
                class="checkbox-field"
                class:checked=checked
                class:disabled=includes_disabled
            >
                <span class="checkbox-box">
                    <input
                        type="checkbox"
                        disabled=includes_disabled
                        prop:checked=checked
                        on:change=move |ev| {
                            let value = event_target_checked(&ev);
                            form.update(|f| set(f, value));
                        }
                    />
                    {move || {
                        let path = if checked() { icon::CHECKBOX } else { icon::CHECKBOX_BLANK };
                        view! { <Icon path /> }
                    }}
                </span>
                <span>{LOCALE.key(label)}</span>
            </label>
        }
    };

    view! {
        // Clicking outside does not close it.
        <div class="modal-backdrop">
            <form class="dialog profile-dialog" role="dialog" aria-modal="true" on:submit=submit>
                <h2>{LOCALE.key(title)}</h2>
                <div class="form-field">
                    <div class="outlined-field" class:error=move || name_error_key().is_some()>
                        <label for="profile-name">{required(MessageKey::ProfileName)}</label>
                        <input
                            id="profile-name"
                            type="text"
                            required
                            autofocus
                            placeholder=LOCALE.key(MessageKey::ProfileNamePlaceholder)
                            prop:value=move || form.with(|f| f.name.clone())
                            on:input=move |ev| {
                                let name = event_target_value(&ev);
                                name_error.set(None);
                                form.update(|f| f.name = name);
                            }
                        />
                        <button
                            type="button"
                            class="adornment"
                            title=LOCALE.key(MessageKey::ProfileGenerateName)
                            on:click=move |_| {
                                let name = random_name(js_sys::Math::random(), js_sys::Math::random());
                                name_error.set(None);
                                form.update(|f| f.name = name);
                            }
                        >
                            <Icon path=icon::DICE />
                        </button>
                    </div>
                    {move || {
                        name_error_key()
                            .map(|key| view! { <p class="helper-text error">{LOCALE.key(key)}</p> })
                    }}
                </div>
                <div class="form-field">
                    <OutlinedSelect
                        id="profile-price-league"
                        label=required(MessageKey::ProfilePriceLeague)
                        options=price_league_options
                        value=Signal::derive(move || form.with(|f| f.price_league.clone()))
                        on_change=Callback::new(move |league| form.update(|f| f.price_league = league))
                        disabled=Signal::stored(false)
                        error=Signal::stored(false)
                    />
                </div>
                <div class="form-field">
                    <OutlinedSelect
                        id="profile-league"
                        label=required(MessageKey::ProfileLeague)
                        options=league_options
                        value=Signal::derive(move || form.with(|f| f.league.clone()))
                        on_change=change_league
                        disabled=Signal::derive(move || loading.get())
                        error=Signal::derive(no_characters)
                    />
                    <Show when=no_characters>
                        <p class="helper-text error">
                            {LOCALE.key(MessageKey::ErrorNoCharactersInLeague)}
                        </p>
                    </Show>
                </div>
                <div class="form-field">
                    <StashTabPicker form options loading />
                    <Show when=move || form.with(ProfileForm::many_tabs)>
                        <p class="warning-note">
                            <Icon path=icon::WARNING />
                            {LOCALE.key(MessageKey::ProfileStashTabsWarning)}
                        </p>
                    </Show>
                </div>
                <div class="form-field">
                    <OutlinedSelect
                        id="profile-character"
                        label=LOCALE.key(MessageKey::ProfileCharacter)
                        options=character_options
                        value=Signal::derive(move || {
                            form.with(|f| f.character.clone().unwrap_or_default())
                        })
                        on_change=Callback::new(move |name: String| {
                            form.update(|f| f.choose_character(Some(name).filter(|n| !n.is_empty())));
                        })
                        disabled=Signal::stored(false)
                        error=Signal::stored(false)
                    />
                </div>
                {checkbox(
                    MessageKey::ProfileIncludeEquipment,
                    |f| f.include_equipment,
                    |f, v| f.include_equipment = v,
                )}
                {checkbox(
                    MessageKey::ProfileIncludeInventory,
                    |f| f.include_inventory,
                    |f, v| f.include_inventory = v,
                )}
                <div class="dialog-actions">
                    <button type="button" class="text" on:click=move |_| on_close.run(())>
                        {LOCALE.key(MessageKey::Cancel)}
                    </button>
                    <button type="submit" class="primary" disabled=move || !submittable()>
                        {LOCALE.key(submit_label)}
                        <Icon path=icon::SAVE />
                    </button>
                </div>
            </form>
        </div>
    }
}

/// A required field's label: legacy's thin space and asterisk after it.
fn required(key: MessageKey) -> String {
    format!("{}\u{2009}*", LOCALE.key(key))
}

/// Select options whose values are their texts.
fn pairs(values: Vec<String>) -> Vec<(String, String)> {
    values.into_iter().map(|v| (v.clone(), v)).collect()
}

/// Legacy's stash tab picker (`StashTabDropdown.tsx`, an MUI Autocomplete):
/// the selected tabs as chips bordered in their colour, only two of them
/// while it is not focused; typing filters the league's tabs, a click on
/// one toggles it and the list stays open.
#[component]
fn StashTabPicker(
    form: RwSignal<ProfileForm>,
    options: RwSignal<Option<ProfileOptions>>,
    loading: RwSignal<bool>,
) -> impl IntoView {
    let open = RwSignal::new(false);
    let focused = RwSignal::new(false);
    let query = RwSignal::new(String::new());
    // The option the arrow keys point at, in `shown`.
    let highlighted = RwSignal::new(None::<usize>);
    let input = NodeRef::<html::Input>::new();
    let root = NodeRef::<html::Div>::new();
    // Where the open list goes: over the dialog, not clipped by its scroll.
    let anchor = RwSignal::new(None::<Anchor>);
    Effect::new(move |_| {
        if open.get() {
            every_frame_while(open, move || place_list(root, anchor));
        }
    });

    let tabs =
        move || options.with(|o| o.as_ref().map(|o| o.stash_tabs.clone()).unwrap_or_default());
    let shown = move || {
        let query = query.get().to_lowercase();
        tabs()
            .into_iter()
            .filter(|tab| tab_matches(tab, &query))
            .collect::<Vec<_>>()
    };
    let selected = move || {
        let ids = form.with(|f| f.stash_tab_ids.clone());
        let tabs = tabs();
        ids.iter()
            .filter_map(|id| tabs.iter().find(|tab| tab.id == *id).cloned())
            .collect::<Vec<_>>()
    };
    let toggle = move |id: &str| {
        form.update(|f| f.toggle_tab(id));
        query.set(String::new());
    };
    let close = move || {
        open.set(false);
        highlighted.set(None);
    };
    let focus_input = move || {
        if let Some(input) = input.get_untracked() {
            _ = input.focus();
        }
    };
    // Moves the highlight `by` options, wrapping around.
    let step = move |by: isize| {
        let count = shown().len() as isize;
        if count == 0 {
            return;
        }
        highlighted.update(|h| {
            let from = h.map_or(if by > 0 { -1 } else { count }, |h| h as isize);
            *h = Some((from + by).rem_euclid(count) as usize);
        });
    };
    let keydown = move |ev: leptos::ev::KeyboardEvent| match ev.key().as_str() {
        "ArrowDown" => {
            ev.prevent_default();
            open.set(true);
            step(1);
        }
        "ArrowUp" => {
            ev.prevent_default();
            open.set(true);
            step(-1);
        }
        "Enter" => {
            let at = highlighted.get_untracked().filter(|_| open.get_untracked());
            if let Some(tab) = at.and_then(|at| shown().into_iter().nth(at)) {
                ev.prevent_default();
                toggle(&tab.id);
            }
        }
        "Escape" if open.get_untracked() => {
            ev.prevent_default();
            ev.stop_propagation();
            close();
        }
        "Backspace" if query.with_untracked(String::is_empty) => {
            if let Some(last) = form.with_untracked(|f| f.stash_tab_ids.last().cloned()) {
                toggle(&last);
            }
        }
        _ => {}
    };

    let chip = move |tab: StashTab| {
        let id = tab.id.clone();
        let colour = tab.colour.map(|c| format!("#{c}"));
        view! {
            <span class="chip" style:border-color=colour>
                <span class="chip-label">{tab.name}</span>
                <button
                    type="button"
                    class="chip-delete"
                    tabindex="-1"
                    on:click=move |_| toggle(&id)
                >
                    <Icon path=icon::CANCEL />
                </button>
            </span>
        }
    };
    let chips = move || {
        let selected = selected();
        // Legacy's `limitTags`: two chips and a count while not focused.
        let limit = if focused.get() {
            selected.len()
        } else {
            CLOSED_CHIPS
        };
        let more = selected.len().saturating_sub(limit);
        let chips = selected.into_iter().take(limit).map(chip).collect_view();
        (
            chips,
            (more > 0).then(|| view! { <span class="more">{format!("+{more}")}</span> }),
        )
    };
    let option = move |(at, tab): (usize, StashTab)| {
        let id = tab.id.clone();
        let is_selected = form.with(|f| f.stash_tab_ids.contains(&tab.id));
        let colour = format!("#{}", tab.colour.as_deref().unwrap_or("000000"));
        view! {
            <li
                role="option"
                aria-selected=is_selected.to_string()
                class:selected=is_selected
                class:highlighted=move || highlighted.get() == Some(at)
                on:mousedown=|ev| ev.prevent_default()
                on:mousemove=move |_| highlighted.set(Some(at))
                on:click=move |_| toggle(&id)
            >
                <svg class="swatch" viewBox="0 0 10 10" aria-hidden="true">
                    <rect width="10" height="10" fill=colour />
                </svg>
                <span class="tab-name">{tab.name}</span>
                <span class="tab-index">{tab.index}</span>
            </li>
        }
    };
    let list = move || {
        let shown = shown();
        if loading.get() {
            view! { <li class="empty">{LOCALE.key(MessageKey::ProfileOptionsLoading)}</li> }
                .into_any()
        } else if shown.is_empty() {
            view! { <li class="empty">{LOCALE.key(MessageKey::ProfileStashTabsNone)}</li> }
                .into_any()
        } else {
            shown
                .into_iter()
                .enumerate()
                .map(option)
                .collect_view()
                .into_any()
        }
    };

    view! {
        <div
            class="outlined-field autocomplete"
            class:open=open
            node_ref=root
            // Presses keep the focus in the input, as MUI's do.
            on:mousedown=move |ev| {
                let on_input = ev
                    .target()
                    .is_some_and(|target| target.dyn_ref::<web_sys::HtmlInputElement>().is_some());
                if !on_input {
                    ev.prevent_default();
                }
            }
            on:click=move |_| focus_input()
        >
            <label for="profile-stash-tabs">{LOCALE.key(MessageKey::ProfileStashTabs)}</label>
            {chips}
            <input
                id="profile-stash-tabs"
                type="text"
                role="combobox"
                aria-expanded=move || open.get().to_string()
                autocomplete="off"
                node_ref=input
                placeholder=LOCALE.key(MessageKey::ProfileStashTabsPlaceholder)
                prop:value=query
                on:input=move |ev| {
                    query.set(event_target_value(&ev));
                    highlighted.set(None);
                    open.set(true);
                }
                on:mousedown=move |_| open.update(|o| *o = !*o)
                on:focus=move |_| focused.set(true)
                on:blur=move |_| {
                    focused.set(false);
                    query.set(String::new());
                    close();
                }
                on:keydown=keydown
            />
            <div class="picker-buttons">
                <Show when=move || form.with(|f| !f.stash_tab_ids.is_empty())>
                    <button
                        type="button"
                        class="clear"
                        tabindex="-1"
                        title=LOCALE.key(MessageKey::ProfileStashTabsClear)
                        on:click=move |_| form.update(|f| f.stash_tab_ids.clear())
                    >
                        <Icon path=icon::CLOSE />
                    </button>
                </Show>
                <button
                    type="button"
                    class="popup"
                    tabindex="-1"
                    title=move || {
                        LOCALE.key(if open.get() {
                            MessageKey::ProfileStashTabsClose
                        } else {
                            MessageKey::ProfileStashTabsOpen
                        })
                    }
                    on:click=move |_| {
                        focus_input();
                        open.update(|o| *o = !*o);
                    }
                >
                    <Icon path=icon::ARROW_DROP_DOWN />
                </button>
            </div>
            <Show when=move || open.get() && anchor.get().is_some()>
                <ul
                    class="autocomplete-list"
                    role="listbox"
                    style:left=move || anchor.get().map(|a| format!("{}px", a.left))
                    style:top=move || anchor.get().map(|a| format!("{}px", a.top))
                    style:width=move || anchor.get().map(|a| format!("{}px", a.width))
                    style:max-height=move || anchor.get().map(|a| format!("{}px", a.max_height))
                >
                    {list}
                </ul>
            </Show>
        </div>
    }
}

/// Chips the stash tab picker shows while not focused (legacy `limitTags`).
const CLOSED_CHIPS: usize = 2;

/// The picker's open list: under the field, as wide, and no taller than
/// 40 % of the window or the room below the field.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Anchor {
    left: f64,
    top: f64,
    width: f64,
    max_height: f64,
}

/// Puts `anchor` under the picker's field `root`.
fn place_list(root: NodeRef<html::Div>, anchor: RwSignal<Option<Anchor>>) {
    let Some(Some(element)) = root.try_get_untracked() else {
        return;
    };
    let rect = element.get_bounding_client_rect();
    let height = window()
        .inner_height()
        .ok()
        .and_then(|h| h.as_f64())
        .unwrap_or_default();
    let top = rect.bottom() + 2.0;
    let next = Some(Anchor {
        left: rect.left(),
        top,
        width: rect.width(),
        max_height: (height * 0.4).min(height - top - 8.0),
    });
    if anchor.try_get_untracked() != Some(next) {
        anchor.try_set(next);
    }
}

/// Whether typing `query` (lower case) keeps `tab` in the picker's list:
/// legacy matched it in the name and the id.
fn tab_matches(tab: &StashTab, query: &str) -> bool {
    format!("{} (id: {})", tab.name, tab.id)
        .to_lowercase()
        .contains(query)
}

/// Logs a failed command: the core already told the user why.
fn log_failed(command: &str, error: &InvokeError) {
    tauri::log(LogLevel::Warn, format!("{command}: {error}"));
}

/// The Profile dialog's fields and their F2 rules.
#[derive(Clone, Debug, Default, PartialEq)]
struct ProfileForm {
    name: String,
    price_league: String,
    league: String,
    /// In selection order.
    stash_tab_ids: Vec<String>,
    character: Option<String>,
    include_equipment: bool,
    include_inventory: bool,
}

impl ProfileForm {
    /// A new Profile's form: `name`, and no tabs (F2).
    fn new(name: String) -> Self {
        Self {
            name,
            ..Self::default()
        }
    }

    fn from_profile(profile: &Profile) -> Self {
        Self {
            name: profile.name.clone(),
            price_league: profile.price_league_id.clone(),
            league: profile.league_id.clone(),
            stash_tab_ids: profile.stash_tab_ids.clone(),
            character: profile.character_name.clone(),
            include_equipment: profile.include_equipment,
            include_inventory: profile.include_inventory,
        }
    }

    /// The choices for the form's league arrived: an empty league becomes
    /// theirs, an empty price league the first price league, and selected
    /// tabs the league no longer has are dropped.
    fn options_loaded(&mut self, options: &ProfileOptions, price_leagues: &[String]) {
        if self.league.is_empty()
            && let Some(league) = &options.league
        {
            self.league = league.clone();
        }
        if self.price_league.is_empty()
            && let Some(first) = price_leagues.first()
        {
            self.price_league = first.clone();
        }
        self.stash_tab_ids
            .retain(|id| options.stash_tabs.iter().any(|tab| tab.id == *id));
    }

    /// Another league: its tabs, character and include options start over
    /// (F2).
    fn change_league(&mut self, league: String) {
        self.league = league;
        self.stash_tab_ids.clear();
        self.choose_character(None);
    }

    /// Without a character, nothing of it is included.
    fn choose_character(&mut self, character: Option<String>) {
        if character.is_none() {
            self.include_equipment = false;
            self.include_inventory = false;
        }
        self.character = character;
    }

    /// The include options need a character (F2).
    fn includes_enabled(&self) -> bool {
        self.character.is_some()
    }

    fn toggle_tab(&mut self, id: &str) {
        match self.stash_tab_ids.iter().position(|t| t == id) {
            Some(index) => {
                self.stash_tab_ids.remove(index);
            }
            None => self.stash_tab_ids.push(id.to_owned()),
        }
    }

    /// Enough tabs to warn that Snapshots take long (F2).
    fn many_tabs(&self) -> bool {
        self.stash_tab_ids.len() >= MANY_TABS
    }

    /// A name, a price league and a league that has characters (F2).
    fn submittable(&self, league_characters: &[String]) -> bool {
        !self.name.trim().is_empty()
            && !self.price_league.is_empty()
            && !self.league.is_empty()
            && !league_characters.is_empty()
    }

    fn into_new(self) -> NewProfile {
        NewProfile {
            name: self.name,
            league_id: self.league,
            price_league_id: self.price_league,
            stash_tab_ids: self.stash_tab_ids,
            character_name: self.character,
            include_equipment: self.include_equipment,
            include_inventory: self.include_inventory,
        }
    }

    fn into_profile(self, id: String) -> Profile {
        self.into_new().with_id(id)
    }
}

const NAME_PREFIXES: [&str; 10] = [
    "Divine", "Majestic", "Strong", "Exalted", "Swift", "Rich", "Humble", "Fine", "Nice", "Fiery",
];
const NAME_SUFFIXES: [&str; 13] = [
    "Exile",
    "Slayer",
    "Assassin",
    "Duelist",
    "Witch",
    "Trader",
    "Farmer",
    "Grinder",
    "Templar",
    "Merchant",
    "Flipper",
    "Stash",
    "Collection",
];

/// A random Profile name from two numbers in `[0, 1)`, as legacy makes them
/// (`generateProfileName`).
fn random_name(prefix: f64, suffix: f64) -> String {
    let pick = |words: &[&'static str], r: f64| {
        words[((r * words.len() as f64) as usize).min(words.len() - 1)]
    };
    format!(
        "{} {}",
        pick(&NAME_PREFIXES, prefix),
        pick(&NAME_SUFFIXES, suffix)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ipc::command::StashTab;

    fn options(league: &str, tabs: &[&str], characters: &[&str]) -> ProfileOptions {
        ProfileOptions {
            league: Some(league.to_owned()),
            leagues: vec!["Settlers".to_owned(), "Standard".to_owned()],
            stash_tabs: tabs
                .iter()
                .map(|id| StashTab {
                    id: (*id).to_owned(),
                    name: id.to_uppercase(),
                    index: 0,
                    colour: None,
                })
                .collect(),
            characters: characters.iter().map(|c| (*c).to_owned()).collect(),
        }
    }

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_owned()).collect()
    }

    #[test]
    fn f2_a_new_profile_gets_the_first_league_and_price_league_and_no_tabs() {
        let mut form = ProfileForm::new("Swift Trader".to_owned());
        form.options_loaded(
            &options("Settlers", &["a", "b"], &["Hero"]),
            &strings(&["Settlers", "Standard"]),
        );
        assert_eq!(form.league, "Settlers");
        assert_eq!(form.price_league, "Settlers");
        assert!(form.stash_tab_ids.is_empty());
        assert!(form.submittable(&strings(&["Hero"])));
    }

    #[test]
    fn f2_name_price_league_league_and_characters_are_required() {
        let mut form = ProfileForm::new("Swift Trader".to_owned());
        form.options_loaded(
            &options("Settlers", &[], &["Hero"]),
            &strings(&["Settlers"]),
        );
        let heroes = strings(&["Hero"]);
        assert!(form.submittable(&heroes));
        assert!(!form.submittable(&[]), "a league without characters");
        for broken in [
            ProfileForm {
                name: "  ".to_owned(),
                ..form.clone()
            },
            ProfileForm {
                price_league: String::new(),
                ..form.clone()
            },
            ProfileForm {
                league: String::new(),
                ..form.clone()
            },
        ] {
            assert!(!broken.submittable(&heroes), "{broken:?}");
        }
    }

    #[test]
    fn f2_include_options_need_a_character() {
        let mut form = ProfileForm::new("Fine Stash".to_owned());
        assert!(!form.includes_enabled());
        form.choose_character(Some("Hero".to_owned()));
        assert!(form.includes_enabled());
        form.include_equipment = true;
        form.include_inventory = true;
        form.choose_character(None);
        assert!(!form.includes_enabled());
        assert!(!form.include_equipment && !form.include_inventory);
    }

    #[test]
    fn f2_changing_the_league_resets_character_includes_and_tabs() {
        let mut form = ProfileForm::from_profile(&Profile {
            id: "p1".to_owned(),
            name: "Rich Farmer".to_owned(),
            league_id: "Settlers".to_owned(),
            price_league_id: "Settlers".to_owned(),
            stash_tab_ids: strings(&["a", "b"]),
            character_name: Some("Hero".to_owned()),
            include_equipment: true,
            include_inventory: true,
        });

        form.change_league("Standard".to_owned());

        assert_eq!(
            form,
            ProfileForm {
                name: "Rich Farmer".to_owned(),
                price_league: "Settlers".to_owned(),
                league: "Standard".to_owned(),
                ..ProfileForm::default()
            }
        );
    }

    #[test]
    fn f2_editing_keeps_only_tabs_the_league_still_has_in_selection_order() {
        let mut form = ProfileForm {
            league: "Settlers".to_owned(),
            price_league: "Settlers".to_owned(),
            stash_tab_ids: strings(&["c", "gone", "a"]),
            ..ProfileForm::new("Nice Grinder".to_owned())
        };
        form.options_loaded(&options("Settlers", &["a", "b", "c"], &["Hero"]), &[]);
        assert_eq!(form.stash_tab_ids, ["c", "a"]);
        form.toggle_tab("b");
        form.toggle_tab("c");
        assert_eq!(form.stash_tab_ids, ["a", "b"]);
    }

    #[test]
    fn f2_five_or_more_tabs_warn() {
        let mut form = ProfileForm::new("Humble Exile".to_owned());
        for id in ["a", "b", "c", "d"] {
            form.toggle_tab(id);
        }
        assert!(!form.many_tabs());
        form.toggle_tab("e");
        assert!(form.many_tabs());
        let text = LOCALE.key(MessageKey::ProfileStashTabsWarning);
        assert!(text.starts_with("Selecting many tabs"), "{text}");
    }

    #[test]
    fn f2_random_names_pair_a_legacy_prefix_and_suffix() {
        assert_eq!(random_name(0.0, 0.0), "Divine Exile");
        assert_eq!(random_name(0.999_999, 0.999_999), "Fiery Collection");
        assert_eq!(random_name(0.45, 0.4), "Swift Trader");
    }

    #[test]
    fn f2_the_form_becomes_the_profile_it_edits() {
        let form = ProfileForm {
            name: "Fiery Witch".to_owned(),
            price_league: "Settlers".to_owned(),
            league: "Standard".to_owned(),
            stash_tab_ids: strings(&["s1"]),
            character: Some("Old".to_owned()),
            include_equipment: true,
            include_inventory: false,
        };
        let profile = form.clone().into_profile("p1".to_owned());
        assert_eq!(ProfileForm::from_profile(&profile), form);
        assert_eq!(profile.id, "p1");
        assert_eq!(profile.league_id, "Standard");
    }
}
