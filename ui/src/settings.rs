//! The Settings view (F17) as legacy's `SettingsTabs`: vertical tabs
//! General, Interface, Prices and Advanced, each of titled sections whose
//! settings sit side by side, each a label, a control and its help. Every
//! setting change goes to the core through `update_settings`; the controls
//! show the pushed `settings` slice. The Prices tab (F6) has commands of its
//! own.

use ipc::command::{
    Command, ExportDiagnostics, GetPriceTable, GetPriceTableArgs, LogLevel, NoArgs, OpenLogFolder,
    PriceKey, PriceTableRow, RemoveCustomPrice, RemoveCustomPriceArgs, ResetData, ScanLegacyImport,
    SetCustomPrice, SetCustomPriceArgs, SettingsPatch, UpdateSettings, UpdateSettingsArgs,
};
use ipc::slices::{ExitAction, ReleaseChannel};
use ipc::{Field, IpcError, MessageKey};
use leptos::html;
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::LOCALE;
use crate::item_table::{Pager, page_range, rarity, trend_cell};
use crate::menu::OutlinedSelect;
use crate::shell::{Icon, icon, open_link};
use crate::state::State;
use crate::tauri::{self, InvokeError};

/// The UI scale's default, range and step in percent (F17).
const UI_SCALE_DEFAULT: u32 = 100;
const UI_SCALE_MIN: u32 = 50;
const UI_SCALE_MAX: u32 = 150;
const UI_SCALE_STEP: u32 = 10;

/// Rows per page of the price table at first (legacy's default page size).
const PRICE_TABLE_PAGE_SIZE: u32 = 25;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    General,
    Ui,
    Prices,
    Advanced,
}

const TABS: [(Tab, MessageKey); 4] = [
    (Tab::General, MessageKey::SettingsTabGeneral),
    (Tab::Ui, MessageKey::SettingsTabUi),
    (Tab::Prices, MessageKey::SettingsTabPrices),
    (Tab::Advanced, MessageKey::SettingsTabAdvanced),
];

#[component]
pub fn SettingsView(state: State) -> impl IntoView {
    let tab = RwSignal::new(Tab::General);
    let tabs = TABS.map(|(value, key)| {
        view! {
            <button
                role="tab"
                class:selected=move || tab.get() == value
                aria-selected=move || (tab.get() == value).to_string()
                on:click=move |_| tab.set(value)
            >
                {LOCALE.key(key)}
            </button>
        }
    });
    view! {
        <section class="settings">
            <nav class="settings-tabs" role="tablist" aria-orientation="vertical">
                {tabs}
            </nav>
            <div class="settings-panel" role="tabpanel">
                <p class="settings-requires-snapshot">
                    {LOCALE.key(MessageKey::SettingsRequiresSnapshot)}
                </p>
                {move || match tab.get() {
                    Tab::General => view! { <GeneralTab state /> }.into_any(),
                    Tab::Ui => view! { <UiTab state /> }.into_any(),
                    Tab::Prices => view! { <PricesTab state /> }.into_any(),
                    Tab::Advanced => view! { <AdvancedTab state /> }.into_any(),
                }}
            </div>
        </section>
    }
}

/// Saves `patch`; an out-of-range value shows in `error` until the next save.
fn save(patch: SettingsPatch, error: RwSignal<Option<String>>) {
    spawn_local(async move {
        match tauri::invoke::<UpdateSettings>(&UpdateSettingsArgs { patch }).await {
            Ok(()) => error.set(None),
            Err(InvokeError::Command(e @ IpcError::Validation { .. })) => {
                error.set(Some(LOCALE.text(&e.message())));
            }
            Err(e) => {
                tauri::log(
                    ipc::command::LogLevel::Error,
                    format!("update_settings: {e}"),
                );
            }
        }
    });
}

/// A titled section of a tab (legacy's `subSection`): an overline title
/// over its settings, side by side.
#[component]
fn Section(title: MessageKey, children: Children) -> impl IntoView {
    view! {
        <section class="settings-section">
            <h2>{LOCALE.key(title)}</h2>
            <div class="settings-section-body">{children()}</div>
        </section>
    }
}

/// A setting's label, marked * when it applies from the next Snapshot.
fn label_text(key: MessageKey, requires_snapshot: bool) -> String {
    let label = LOCALE.key(key);
    if requires_snapshot {
        format!("{label} *")
    } else {
        label
    }
}

/// A setting's help (legacy `FormHelperText`); a failed save's error
/// replaces it.
fn helper_text(help: Option<MessageKey>, error: RwSignal<Option<String>>) -> impl IntoView {
    let failed = move || error.with(Option::is_some);
    view! {
        <p class="helper-text" class:error=failed role=move || failed().then_some("alert")>
            {move || error.get().or_else(|| help.map(|key| LOCALE.key(key)))}
        </p>
    }
}

/// A checkbox setting (legacy `CheckboxSetting`): its label over an MUI
/// checkbox with its value, then its help.
#[component]
fn CheckboxSetting<P>(
    id: &'static str,
    label: MessageKey,
    value: MessageKey,
    help: MessageKey,
    #[prop(optional)] requires_snapshot: bool,
    checked: Signal<bool>,
    patch: P,
) -> impl IntoView
where
    P: Fn(bool) -> SettingsPatch + Send + Sync + 'static,
{
    let error = RwSignal::new(None::<String>);
    view! {
        <div class="setting" role="group" aria-labelledby=format!("{id}-label")>
            <span class="setting-label" id=format!("{id}-label")>
                {label_text(label, requires_snapshot)}
            </span>
            <label class="checkbox-field" class:checked=checked>
                <span class="checkbox-box">
                    <input
                        id=id
                        type="checkbox"
                        prop:checked=checked
                        on:change=move |ev| save(patch(event_target_checked(&ev)), error)
                    />
                    {move || {
                        let path = if checked.get() { icon::CHECKBOX } else { icon::CHECKBOX_BLANK };
                        view! { <Icon path /> }
                    }}
                </span>
                <span>{LOCALE.key(value)}</span>
            </label>
            {helper_text(Some(help), error)}
        </div>
    }
}

/// A number setting (legacy `NumberInputSetting`): an underlined field
/// showing the value and its `unit`. On change the number in it is saved as
/// `patch` makes it, unparsable input counting as out of range, and shown
/// with its unit again.
#[component]
fn NumberSetting<P>(
    id: &'static str,
    label: MessageKey,
    help: MessageKey,
    unit: MessageKey,
    #[prop(optional)] requires_snapshot: bool,
    value: Signal<String>,
    #[prop(into, optional)] disabled: Signal<bool>,
    patch: P,
) -> impl IntoView
where
    P: Fn(&str) -> Option<SettingsPatch> + Send + Sync + 'static,
{
    let error = RwSignal::new(None::<String>);
    let input = NodeRef::<html::Input>::new();
    let unit = LOCALE.key(unit);
    let shown = {
        let unit = unit.clone();
        move || format!("{} {unit}", value.get())
    };
    let on_change = move |_| {
        let Some(field) = input.get_untracked() else {
            return;
        };
        match number_text(&field.value()).and_then(|number| Some((patch(&number)?, number))) {
            Some((p, number)) => {
                field.set_value(&format!("{number} {unit}"));
                save(p, error);
            }
            None => error.set(Some(LOCALE.key(MessageKey::ErrorSettingOutOfRange))),
        }
    };
    view! {
        <div class="setting">
            <label class="setting-label" for=id>
                {label_text(label, requires_snapshot)}
            </label>
            <div class="standard-field" class:disabled=disabled class:error=move || error.with(Option::is_some)>
                <input
                    id=id
                    type="text"
                    inputmode="decimal"
                    autocomplete="off"
                    node_ref=input
                    prop:value=shown
                    disabled=disabled
                    on:change=on_change
                />
            </div>
            {helper_text(Some(help), error)}
        </div>
    }
}

/// A select setting (legacy `SelectSetting`): its label over an underlined
/// select at most `max_width` wide, then its help, if any.
#[component]
fn SelectSetting<P>(
    id: &'static str,
    label: MessageKey,
    #[prop(optional)] help: Option<MessageKey>,
    max_width: u32,
    options: Vec<(&'static str, MessageKey)>,
    value: Signal<String>,
    patch: P,
) -> impl IntoView
where
    P: Fn(&str) -> SettingsPatch + Send + Sync + 'static,
{
    let error = RwSignal::new(None::<String>);
    let options: Vec<(String, String)> = options
        .into_iter()
        .map(|(value, key)| (value.to_owned(), LOCALE.key(key)))
        .collect();
    view! {
        <div class="setting">
            // `OutlinedSelect` is labelled by `{id}-label`.
            <span class="setting-label" id=format!("{id}-label")>
                {LOCALE.key(label)}
            </span>
            <div class="setting-select" style:max-width=format!("{max_width}px")>
                <OutlinedSelect
                    id
                    standard=true
                    options=Signal::derive(move || options.clone())
                    value
                    on_change=Callback::new(move |chosen: String| save(patch(&chosen), error))
                    disabled=Signal::derive(|| false)
                    error=Signal::derive(move || error.with(Option::is_some))
                />
            </div>
            {helper_text(help, error)}
        </div>
    }
}

#[component]
fn GeneralTab(state: State) -> impl IntoView {
    let auto_snapshot = Signal::derive(move || state.settings.with(|s| s.auto_snapshot));
    view! {
        <Section title=MessageKey::SettingsSectionExitAction>
            <SelectSetting
                id="exit-action"
                label=MessageKey::SettingsExitAction
                max_width=340
                options=vec![
                    (
                        exit_action_value(ExitAction::MinimizeToTray),
                        MessageKey::SettingsExitActionMinimizeToTray,
                    ),
                    (exit_action_value(ExitAction::Exit), MessageKey::SettingsExitActionExit),
                ]
                value=Signal::derive(move || {
                    state.settings.with(|s| exit_action_value(s.exit_action).to_owned())
                })
                patch=|value| SettingsPatch {
                    exit_action: Some(parse_exit_action(value)),
                    ..SettingsPatch::default()
                }
            />
        </Section>
        <Section title=MessageKey::SettingsSectionSnapshot>
            <CheckboxSetting
                id="auto-snapshot"
                label=MessageKey::SettingsAutoSnapshotLabel
                value=MessageKey::SettingsAutoSnapshot
                help=MessageKey::SettingsAutoSnapshotHelp
                checked=auto_snapshot
                patch=|on| SettingsPatch { auto_snapshot: Some(on), ..SettingsPatch::default() }
            />
            <NumberSetting
                id="auto-snapshot-minutes"
                label=MessageKey::SettingsAutoSnapshotMinutes
                help=MessageKey::SettingsAutoSnapshotMinutesHelp
                unit=MessageKey::SettingsUnitMinutes
                value=Signal::derive(move || {
                    state.settings.with(|s| s.auto_snapshot_minutes.to_string())
                })
                disabled=Signal::derive(move || !auto_snapshot.get())
                patch=|v| {
                    v.parse().ok().map(|m| SettingsPatch {
                        auto_snapshot_minutes: Some(m),
                        ..SettingsPatch::default()
                    })
                }
            />
        </Section>
        <Section title=MessageKey::SettingsSectionPricing>
            <CheckboxSetting
                id="low-confidence-pricing"
                label=MessageKey::SettingsLowConfidencePricingLabel
                value=MessageKey::SettingsLowConfidencePricing
                help=MessageKey::SettingsLowConfidencePricingHelp
                requires_snapshot=true
                checked=Signal::derive(move || state.settings.with(|s| s.low_confidence_pricing))
                patch=|on| SettingsPatch {
                    low_confidence_pricing: Some(on),
                    ..SettingsPatch::default()
                }
            />
            <NumberSetting
                id="price-threshold"
                label=MessageKey::SettingsPriceThreshold
                help=MessageKey::SettingsPriceThresholdHelp
                unit=MessageKey::CurrencyChaos
                requires_snapshot=true
                value=Signal::derive(move || state.settings.with(|s| s.price_threshold.to_string()))
                patch=|v| {
                    parse_chaos(v).map(|t| SettingsPatch {
                        price_threshold: Some(t),
                        ..SettingsPatch::default()
                    })
                }
            />
            <NumberSetting
                id="total-price-threshold"
                label=MessageKey::SettingsTotalPriceThreshold
                help=MessageKey::SettingsTotalPriceThresholdHelp
                unit=MessageKey::CurrencyChaos
                requires_snapshot=true
                value=Signal::derive(move || {
                    state.settings.with(|s| s.total_price_threshold.to_string())
                })
                patch=|v| {
                    parse_chaos(v).map(|t| SettingsPatch {
                        total_price_threshold: Some(t),
                        ..SettingsPatch::default()
                    })
                }
            />
        </Section>
    }
}

/// Legacy's `SliderSetting`: a slider, the percent in a small field and,
/// off the default, Reset to default. The slider and field show the
/// dragged value; the scale is saved when the drag ends.
#[component]
fn UiTab(state: State) -> impl IntoView {
    let error = RwSignal::new(None::<String>);
    let saved = move || state.settings.with(|s| s.ui_scale);
    let shown = RwSignal::new(state.settings.with_untracked(|s| s.ui_scale));
    Effect::new(move |_| shown.set(saved()));
    // The core zooms every window at once (F17).
    let set_scale = move |percent: u32| {
        shown.set(percent);
        save(
            SettingsPatch {
                ui_scale: Some(percent),
                ..SettingsPatch::default()
            },
            error,
        );
    };
    // MUI's track: the rail filled up to the thumb.
    let filled = move || format!("{}% 100%", scale_fill(shown.get()));
    view! {
        <Section title=MessageKey::SettingsSectionInterface>
            <div class="setting scale-setting">
                <label class="setting-label" for="ui-scale">
                    {LOCALE.key(MessageKey::SettingsUiScale)}
                </label>
                <div class="scale-controls">
                    <Icon path=icon::ZOOM_IN />
                    <input
                        id="ui-scale"
                        class="slider"
                        type="range"
                        min=UI_SCALE_MIN.to_string()
                        max=UI_SCALE_MAX.to_string()
                        step=UI_SCALE_STEP.to_string()
                        prop:value=move || shown.get().to_string()
                        style:background-size=filled
                        on:input=move |ev| {
                            if let Ok(percent) = event_target_value(&ev).parse() {
                                shown.set(percent);
                            }
                        }
                        on:change=move |ev| {
                            if let Ok(percent) = event_target_value(&ev).parse() {
                                set_scale(percent);
                            }
                        }
                    />
                    <div class="standard-field scale-field" class:error=move || error.with(Option::is_some)>
                        <input
                            type="number"
                            aria-label=LOCALE.key(MessageKey::SettingsUiScale)
                            min=UI_SCALE_MIN.to_string()
                            max=UI_SCALE_MAX.to_string()
                            step=UI_SCALE_STEP.to_string()
                            prop:value=move || shown.get().to_string()
                            on:change=move |ev| match event_target_value(&ev).trim().parse() {
                                Ok(percent) => set_scale(percent),
                                Err(_) => error.set(Some(LOCALE.key(MessageKey::ErrorSettingOutOfRange))),
                            }
                        />
                    </div>
                    <Show when=move || shown.get() != UI_SCALE_DEFAULT>
                        <button class="contained" on:click=move |_| set_scale(UI_SCALE_DEFAULT)>
                            {LOCALE.key(MessageKey::SettingsUiScaleReset)}
                        </button>
                    </Show>
                </div>
                {helper_text(Some(MessageKey::SettingsUiScaleHelp), error)}
            </div>
        </Section>
    }
}

/// How far along the slider `percent` sits, in percent of its length.
fn scale_fill(percent: u32) -> u32 {
    (percent.clamp(UI_SCALE_MIN, UI_SCALE_MAX) - UI_SCALE_MIN) * 100 / (UI_SCALE_MAX - UI_SCALE_MIN)
}

/// The price table of a chosen price league, the active one at first, with
/// a name filter, edit and delete (F6), as the Net worth item table looks.
/// Until the core answers it shows the waiting text; when the answer for a
/// league fails it keeps showing it.
#[component]
fn PricesTab(state: State) -> impl IntoView {
    let league = RwSignal::new(String::new());
    let rows = RwSignal::new(None::<Vec<PriceTableRow>>);
    let filter = RwSignal::new(String::new());
    let page = RwSignal::new(0usize);
    let page_size = RwSignal::new(PRICE_TABLE_PAGE_SIZE);
    let editing = RwSignal::new(None::<PriceTableRow>);

    // The active price league until one is picked.
    Effect::new(move |_| {
        let active = state.prices.with(|p| p.price_league_id.clone());
        if let Some(active) = active
            && league.with_untracked(String::is_empty)
        {
            league.set(active);
        }
    });
    // Loads the chosen league's table; a late answer for a league no longer
    // chosen is dropped.
    Effect::new(move |_| {
        let chosen = league.get();
        if chosen.is_empty() {
            return;
        }
        rows.set(None);
        page.set(0);
        spawn_local(async move {
            let args = GetPriceTableArgs {
                price_league: chosen.clone(),
            };
            let loaded = tauri::invoke::<GetPriceTable>(&args).await;
            if league.with_untracked(|l| *l != chosen) {
                return;
            }
            match loaded {
                Ok(loaded) => rows.set(Some(loaded)),
                Err(error) => log_failed(GetPriceTable::NAME, &error),
            }
        });
    });

    // A stored change shows in the table of the league it was made in.
    let apply = move |price_league: &str, key: &PriceKey, value: Option<f64>| {
        if league.with_untracked(|l| l == price_league) {
            rows.update(|r| {
                if let Some(r) = r {
                    set_custom_price(r, key, value);
                }
            });
        }
    };
    let remove = move |key: PriceKey| {
        let price_league = league.get_untracked();
        spawn_local(async move {
            let args = RemoveCustomPriceArgs {
                price_league: price_league.clone(),
                key: key.clone(),
            };
            match tauri::invoke::<RemoveCustomPrice>(&args).await {
                Ok(()) => apply(&price_league, &key, None),
                Err(error) => log_failed(RemoveCustomPrice::NAME, &error),
            }
        });
    };

    let matching = Memo::new(move |_| {
        let filter = filter.get();
        rows.with(|r| r.as_deref().map(|r| matching_rows(r, &filter)))
    });
    let count = move || matching.with(|m| m.as_ref().map_or(0, Vec::len));
    let page_rows = move || {
        let range = page_range(count(), page.get(), page_size.get());
        matching.with(|m| {
            rows.with(|r| match (m, r) {
                (Some(m), Some(r)) => m[range].iter().map(|&i| r[i].clone()).collect(),
                _ => Vec::new(),
            })
        })
    };
    let league_options = Signal::derive(move || {
        let chosen = league.get();
        let mut leagues = state.profiles.with(|p| p.price_leagues.clone());
        if !chosen.is_empty() && !leagues.contains(&chosen) {
            leagues.insert(0, chosen);
        }
        leagues.into_iter().map(|l| (l.clone(), l)).collect()
    });
    // Legacy's `ItemCell`: a field without a value reads N/A.
    let or_na = |value: Option<String>| match value {
        Some(value) => view! { <span>{value}</span> }.into_any(),
        None => view! {
            <span class="unavailable">{LOCALE.key(MessageKey::ItemTableNotAvailable)}</span>
        }
        .into_any(),
    };
    let row_view = move |row: PriceTableRow| {
        let key = row.key.clone();
        let edited = row.clone();
        let has_custom = row.custom_price.is_some();
        let PriceTableRow {
            key: shown,
            price,
            custom_price,
            trend,
            ninja_url,
        } = row;
        let icon_cell =
            (!shown.icon.is_empty()).then(|| view! { <img src=shown.icon.clone() alt="" /> });
        let positive = |v: Option<u32>| v.filter(|v| *v > 0).map(|v| v.to_string());
        let corrupted = shown.corrupted.map_or_else(
            || or_na(None),
            |corrupted| {
                let text = LOCALE.key(if corrupted {
                    MessageKey::ItemTableYes
                } else {
                    MessageKey::ItemTableNo
                });
                view! { <span class:corrupted=corrupted>{text}</span> }.into_any()
            },
        );
        let link = ninja_url.map(|url| {
            view! {
                <button
                    class="control"
                    title=LOCALE.key(MessageKey::ItemTableOpenOnNinja)
                    on:click=move |_| open_link(&url)
                >
                    <Icon path=icon::TIMELINE />
                </button>
            }
        });
        let trend = trend
            .as_ref()
            .and_then(trend_cell)
            .unwrap_or_else(|| or_na(None));
        let rarity = shown.frame_type.map_or("unknown", rarity);
        view! {
            <tr class=format!("rarity-{rarity}")>
                <td class="icon-cell">
                    <div class="item-icon-cell">{icon_cell}</div>
                </td>
                <td>
                    <div class="item-name">
                        <span class="ellipsis" title=shown.name.clone()>{shown.name.clone()}</span>
                        {link}
                    </div>
                </td>
                <td class="number">{or_na(positive(shown.tier).or(positive(shown.ilvl)))}</td>
                <td>{or_na(shown.variant.clone().filter(|v| !v.is_empty()))}</td>
                <td>{corrupted}</td>
                <td>{or_na(positive(shown.links))}</td>
                <td class="number">{or_na(positive(shown.quality))}</td>
                <td class="number">{or_na(positive(shown.level))}</td>
                <td class="number trend-cell">{trend}</td>
                <td class="number">{chaos_or(price, MessageKey::CustomPricesNoData)}</td>
                <td class="number">
                    <div class="custom-price">
                        {custom_price.map_or_else(
                            || LOCALE.key(MessageKey::CustomPricesNotSet),
                            |v| format!("{v:.2}"),
                        )}
                        <button
                            class="control"
                            title=LOCALE.key(MessageKey::CustomPricesSet)
                            on:click=move |_| editing.set(Some(edited.clone()))
                        >
                            <Icon path=icon::EDIT />
                        </button>
                        <button
                            class="control"
                            title=LOCALE.key(MessageKey::CustomPricesRemove)
                            disabled=!has_custom
                            on:click=move |_| remove(key.clone())
                        >
                            <Icon path=icon::DELETE />
                        </button>
                    </div>
                </td>
            </tr>
        }
    };
    let header = |key: MessageKey, numeric: bool| {
        view! {
            <th class:number=numeric>
                <span class="label">{LOCALE.key(key)}</span>
            </th>
        }
    };

    view! {
        <Section title=MessageKey::SettingsSectionPrices>
            <div class="item-table price-table">
                <div class="item-table-toolbar">
                    <div class="item-table-filter">
                        <Icon path=icon::SEARCH />
                        <input
                            type="text"
                            placeholder=LOCALE.key(MessageKey::CustomPricesFilter)
                            prop:value=filter
                            on:input=move |ev| {
                                filter.set(event_target_value(&ev));
                                page.set(0);
                            }
                        />
                        <Show when=move || filter.with(|f| !f.is_empty())>
                            <button
                                class="control"
                                title=LOCALE.key(MessageKey::CustomPricesClearFilter)
                                on:click=move |_| {
                                    filter.set(String::new());
                                    page.set(0);
                                }
                            >
                                <Icon path=icon::CLOSE />
                            </button>
                        </Show>
                    </div>
                    <div class="price-league-select">
                        <OutlinedSelect
                            id="custom-prices-league"
                            label=LOCALE.key(MessageKey::CustomPricesPriceLeague)
                            options=league_options
                            value=league.into()
                            on_change=Callback::new(move |chosen| league.set(chosen))
                            disabled=Signal::derive(|| false)
                            error=Signal::derive(|| false)
                        />
                    </div>
                </div>
                <Show
                    when=move || rows.with(Option::is_some)
                    fallback=|| {
                        view! { <p class="helper-text">{LOCALE.key(MessageKey::CustomPricesWaiting)}</p> }
                    }
                >
                    <div class="item-table-scroll">
                        <table>
                            <thead>
                                <tr>
                                    {header(MessageKey::CustomPricesIcon, false)}
                                    {header(MessageKey::CustomPricesName, false)}
                                    {header(MessageKey::CustomPricesIlvlTier, true)}
                                    {header(MessageKey::CustomPricesVariant, false)}
                                    {header(MessageKey::CustomPricesCorrupted, false)}
                                    {header(MessageKey::CustomPricesLinks, false)}
                                    {header(MessageKey::CustomPricesQuality, true)}
                                    {header(MessageKey::CustomPricesLevel, true)}
                                    {header(MessageKey::ItemTableTrend, true)}
                                    {header(MessageKey::CustomPricesPrice, true)}
                                    {header(MessageKey::CustomPricesCustomPrice, true)}
                                </tr>
                            </thead>
                            <tbody>
                                <Show
                                    when=move || count() != 0
                                    fallback=|| {
                                        view! {
                                            <tr>
                                                <td class="empty" colspan="11">
                                                    {LOCALE.key(MessageKey::CustomPricesEmpty)}
                                                </td>
                                            </tr>
                                        }
                                    }
                                >
                                    {move || page_rows().into_iter().map(row_view).collect_view()}
                                </Show>
                            </tbody>
                        </table>
                    </div>
                    <Show when=move || count() != 0>
                        <Pager
                            id="price-table-page-size"
                            count=Signal::derive(count)
                            page
                            page_size=page_size.into()
                            on_page_size=Callback::new(move |size| page_size.set(size))
                        />
                    </Show>
                </Show>
            </div>
        </Section>
        {move || {
            editing
                .get()
                .map(|row| {
                    let price_league = league.get_untracked();
                    let key = row.key.clone();
                    let saved_in = price_league.clone();
                    let on_close = Callback::new(move |()| editing.set(None));
                    let on_saved = Callback::new(move |value: f64| {
                        apply(&saved_in, &key, Some(value));
                        editing.set(None);
                    });
                    view! { <CustomPriceDialog price_league row on_close on_saved /> }
                })
        }}
    }
}

/// The custom-price dialog: a required number of chaos, prefilled with the
/// poe.ninja price rounded to 2 decimals (F6). Clicking outside does not
/// close it.
#[component]
fn CustomPriceDialog(
    price_league: String,
    row: PriceTableRow,
    on_close: Callback<()>,
    on_saved: Callback<f64>,
) -> impl IntoView {
    let input = RwSignal::new(prefill(row.price));
    let saving = RwSignal::new(false);
    let core_error = RwSignal::new(None::<MessageKey>);
    let error = move || {
        core_error
            .get()
            .or_else(|| input.with(|v| parse_custom_price(v).err()))
    };
    let name = row.key.name.clone();
    let key = row.key;
    let submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let Ok(value) = input.with_untracked(|v| parse_custom_price(v)) else {
            return;
        };
        if saving.get_untracked() {
            return;
        }
        saving.set(true);
        let args = SetCustomPriceArgs {
            price_league: price_league.clone(),
            key: key.clone(),
            value,
        };
        spawn_local(async move {
            let saved = tauri::invoke::<SetCustomPrice>(&args).await;
            saving.set(false);
            match saved {
                Ok(()) => on_saved.run(value),
                Err(InvokeError::Command(IpcError::Validation {
                    field: Field::CustomPrice,
                    key,
                })) => core_error.set(Some(key)),
                Err(error) => log_failed(SetCustomPrice::NAME, &error),
            }
        });
    };
    view! {
        <div class="modal-backdrop">
            <form class="dialog" role="dialog" aria-modal="true" on:submit=submit>
                <h2>{LOCALE.key(MessageKey::CustomPriceDialogTitle)}</h2>
                <p>{name}</p>
                <label for="custom-price">{LOCALE.key(MessageKey::CustomPricesCustomPrice)}</label>
                <input
                    id="custom-price"
                    type="number"
                    min="0"
                    step="any"
                    required
                    autofocus
                    placeholder=LOCALE.key(MessageKey::CustomPriceDialogPlaceholder)
                    prop:value=input
                    on:input=move |ev| {
                        core_error.set(None);
                        input.set(event_target_value(&ev));
                    }
                />
                {move || error().map(|key| view! { <p class="field-error">{LOCALE.key(key)}</p> })}
                <div class="dialog-actions">
                    <button type="button" class="text" on:click=move |_| on_close.run(())>
                        {LOCALE.key(MessageKey::CustomPriceDialogClose)}
                    </button>
                    <button
                        type="submit"
                        class="primary"
                        disabled=move || saving.get() || error().is_some()
                    >
                        {LOCALE.key(MessageKey::CustomPriceDialogSave)}
                    </button>
                </div>
            </form>
        </div>
    }
}

/// Logs a failed command: the core already logged why.
fn log_failed(command: &str, error: &InvokeError) {
    tauri::log(LogLevel::Warn, format!("{command}: {error}"));
}

/// The indices of `rows` whose name contains `filter`, ignoring case (F6).
fn matching_rows(rows: &[PriceTableRow], filter: &str) -> Vec<usize> {
    let filter = filter.to_lowercase();
    rows.iter()
        .enumerate()
        .filter(|(_, row)| row.key.name.to_lowercase().contains(&filter))
        .map(|(i, _)| i)
        .collect()
}

/// Shows `value` as the custom price of every row of the entry `key`.
fn set_custom_price(rows: &mut [PriceTableRow], key: &PriceKey, value: Option<f64>) {
    for row in rows.iter_mut().filter(|row| row.key == *key) {
        row.custom_price = value;
    }
}

/// A chaos amount with 2 decimals, or the text of `none` for 0.
fn chaos_or(value: f64, none: MessageKey) -> String {
    if value == 0.0 {
        LOCALE.key(none)
    } else {
        format!("{value:.2}")
    }
}

/// The dialog's prefill: `price` rounded to 2 decimals, without trailing
/// zeros (legacy `+price.toFixed(2)`).
fn prefill(price: f64) -> String {
    ((price * 100.0).round() / 100.0).to_string()
}

/// The dialog's custom price: a number of 0 or more.
fn parse_custom_price(value: &str) -> Result<f64, MessageKey> {
    let value = value.trim();
    if value.is_empty() {
        return Err(MessageKey::ErrorCustomPriceRequired);
    }
    value
        .parse()
        .ok()
        .filter(|v: &f64| v.is_finite() && *v >= 0.0)
        .ok_or(MessageKey::ErrorCustomPriceInvalid)
}

#[component]
fn AdvancedTab(state: State) -> impl IntoView {
    view! {
        <Section title=MessageKey::SettingsSectionReleaseChannel>
            // A channel change takes effect after a restart (F16).
            <SelectSetting
                id="release-channel"
                label=MessageKey::SettingsReleaseChannel
                help=MessageKey::SettingsReleaseChannelNote
                max_width=150
                options=vec![
                    (
                        release_channel_value(ReleaseChannel::Latest),
                        MessageKey::SettingsReleaseChannelLatest,
                    ),
                    (
                        release_channel_value(ReleaseChannel::Beta),
                        MessageKey::SettingsReleaseChannelBeta,
                    ),
                ]
                value=Signal::derive(move || {
                    state.settings.with(|s| release_channel_value(s.release_channel).to_owned())
                })
                patch=|value| SettingsPatch {
                    release_channel: Some(parse_release_channel(value)),
                    ..SettingsPatch::default()
                }
            />
        </Section>
        // WKWebView has no switch (D7); the stored value shows (D16).
        <Show when=|| !is_macos()>
            <Section title=MessageKey::SettingsSectionHardwareAcceleration>
                <CheckboxSetting
                    id="hardware-acceleration"
                    label=MessageKey::SettingsHardwareAccelerationLabel
                    value=MessageKey::SettingsHardwareAcceleration
                    help=MessageKey::SettingsHardwareAccelerationNote
                    checked=Signal::derive(move || state.settings.with(|s| s.hardware_acceleration))
                    patch=|on| SettingsPatch {
                        hardware_acceleration: Some(on),
                        ..SettingsPatch::default()
                    }
                />
            </Section>
        </Show>
        <LegacyImportSection state />
        <DiagnosticsSection state />
        <ResetDataSection />
    }
}

/// Import from Exilence CE (F21): the core scans and opens the import
/// dialog, or says no data was found. Disabled while the import or another
/// operation runs.
#[component]
fn LegacyImportSection(state: State) -> impl IntoView {
    let busy = move || {
        state.legacy_import.with(|l| l.running)
            || state
                .status
                .with(|s| s.snapshot_running || s.operation_running)
    };
    let scan = |_| tauri::send::<ScanLegacyImport>(NoArgs {});
    view! {
        <Section title=MessageKey::LegacyImportTitle>
            <div class="setting button-setting">
                <div class="setting-buttons">
                    <button class="contained" disabled=busy on:click=scan>
                        {LOCALE.key(MessageKey::LegacyImportTitle)}
                    </button>
                </div>
                <p class="helper-text">{LOCALE.key(MessageKey::SettingsLegacyImportNote)}</p>
            </div>
        </Section>
    }
}

/// Debug mode, Export diagnostics and Open log folder (F20). Both buttons
/// leave the dialog and the folder to the core.
#[component]
fn DiagnosticsSection(state: State) -> impl IntoView {
    let export = |_| tauri::send::<ExportDiagnostics>(NoArgs {});
    let open_log_folder = |_| tauri::send::<OpenLogFolder>(NoArgs {});
    view! {
        <Section title=MessageKey::SettingsDiagnostics>
            <CheckboxSetting
                id="debug-mode"
                label=MessageKey::SettingsDebugModeLabel
                value=MessageKey::SettingsDebugMode
                help=MessageKey::SettingsDebugModeNote
                checked=Signal::derive(move || state.settings.with(|s| s.debug_mode))
                patch=|on| SettingsPatch {
                    debug_mode: Some(on),
                    ..SettingsPatch::default()
                }
            />
            <div class="setting button-setting">
                <div class="setting-buttons">
                    <button class="contained" on:click=export>
                        {LOCALE.key(MessageKey::ExportDiagnostics)}
                    </button>
                    <button class="contained" on:click=open_log_folder>
                        {LOCALE.key(MessageKey::OpenLogFolder)}
                    </button>
                </div>
                <p class="helper-text">{LOCALE.key(MessageKey::SettingsDiagnosticsNote)}</p>
            </div>
        </Section>
    }
}

/// Reset data (F17) as legacy's `ResetIndexedDbSettings`: the button opens
/// a confirmation dialog. On success the app restarts; a failure closes the
/// dialog and shows under the button.
#[component]
fn ResetDataSection() -> impl IntoView {
    let confirming = RwSignal::new(false);
    let resetting = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let reset = move |_| {
        resetting.set(true);
        spawn_local(async move {
            if let Err(e) = tauri::invoke::<ResetData>(&NoArgs {}).await {
                tauri::log(ipc::command::LogLevel::Error, format!("reset_data: {e}"));
                if let InvokeError::Command(e) = e {
                    error.set(Some(LOCALE.text(&e.message())));
                }
                resetting.set(false);
                confirming.set(false);
            }
        });
    };
    view! {
        <Section title=MessageKey::SettingsResetData>
            <div class="setting button-setting">
                <div class="setting-buttons">
                    <button class="contained" on:click=move |_| confirming.set(true)>
                        {LOCALE.key(MessageKey::SettingsResetData)}
                    </button>
                </div>
                {helper_text(Some(MessageKey::SettingsResetDataDescription), error)}
            </div>
        </Section>
        <Show when=move || confirming.get()>
            <div class="modal-backdrop">
                <div class="dialog" role="alertdialog" aria-modal="true">
                    <h2>{LOCALE.key(MessageKey::SettingsResetDataQuestion)}</h2>
                    <p>{LOCALE.key(MessageKey::SettingsResetDataDescription)}</p>
                    <div class="dialog-actions">
                        <button
                            class="text"
                            autofocus
                            disabled=resetting
                            on:click=move |_| confirming.set(false)
                        >
                            {LOCALE.key(MessageKey::SettingsResetDataCancel)}
                        </button>
                        <button class="primary" disabled=resetting on:click=reset>
                            {LOCALE.key(MessageKey::SettingsResetDataConfirm)}
                        </button>
                    </div>
                </div>
            </div>
        </Show>
    }
}

/// Whether the webview runs on macOS (WKWebView's user agent).
fn is_macos() -> bool {
    web_sys::window()
        .and_then(|w| w.navigator().user_agent().ok())
        .is_some_and(|ua| ua.contains("Macintosh"))
}

/// A chaos amount; not a finite number is `None`.
fn parse_chaos(value: &str) -> Option<f64> {
    value.trim().parse().ok().filter(|v: &f64| v.is_finite())
}

/// The number in a number setting's text, without its unit and thousands
/// separators (legacy `NumberFormat`); `None` when there is none.
fn number_text(text: &str) -> Option<String> {
    let number: String = text
        .chars()
        .filter(|c| c.is_ascii_digit() || matches!(c, '.' | '-'))
        .collect();
    let number: f64 = number.parse().ok().filter(|v: &f64| v.is_finite())?;
    Some(number.to_string())
}

/// The `<option>` value of an exit action.
fn exit_action_value(action: ExitAction) -> &'static str {
    match action {
        ExitAction::MinimizeToTray => "minimize_to_tray",
        ExitAction::Exit => "exit",
    }
}

fn parse_exit_action(value: &str) -> ExitAction {
    match value {
        "exit" => ExitAction::Exit,
        _ => ExitAction::MinimizeToTray,
    }
}

/// The `<option>` value of a release channel: its settings value.
fn release_channel_value(channel: ReleaseChannel) -> &'static str {
    match channel {
        ReleaseChannel::Latest => "latest",
        ReleaseChannel::Beta => "beta",
    }
}

fn parse_release_channel(value: &str) -> ReleaseChannel {
    match value {
        "beta" => ReleaseChannel::Beta,
        _ => ReleaseChannel::Latest,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f17_number_settings_read_the_number_without_unit_or_separators() {
        assert_eq!(number_text("20 min").as_deref(), Some("20"));
        assert_eq!(number_text("1,000 c").as_deref(), Some("1000"));
        assert_eq!(number_text(" 2.5c").as_deref(), Some("2.5"));
        assert_eq!(number_text("05").as_deref(), Some("5"));
        assert_eq!(number_text("-5 c").as_deref(), Some("-5"));
        assert_eq!(number_text("min"), None);
        assert_eq!(number_text("1.2.3"), None);
    }

    #[test]
    fn exit_action_option_values_round_trip() {
        for action in [ExitAction::MinimizeToTray, ExitAction::Exit] {
            assert_eq!(parse_exit_action(exit_action_value(action)), action);
        }
    }

    #[test]
    fn f16_release_channel_option_values_round_trip() {
        for channel in [ReleaseChannel::Latest, ReleaseChannel::Beta] {
            assert_eq!(
                parse_release_channel(release_channel_value(channel)),
                channel
            );
        }
    }

    #[test]
    fn f5_chaos_amounts_parse_only_as_finite_numbers() {
        assert_eq!(parse_chaos(" 2.5 "), Some(2.5));
        assert_eq!(parse_chaos(""), None);
        assert_eq!(parse_chaos("inf"), None);
        assert_eq!(parse_chaos("NaN"), None);
    }

    fn row(name: &str, price: f64) -> PriceTableRow {
        PriceTableRow {
            key: PriceKey {
                name: name.to_owned(),
                ..PriceKey::default()
            },
            price,
            custom_price: None,
            trend: None,
            ninja_url: None,
        }
    }

    #[test]
    fn f6_name_filter_ignores_case() {
        let rows = [row("Divine Orb", 150.0), row("Exalted Orb", 12.0)];
        assert_eq!(matching_rows(&rows, "DIVINE"), [0]);
        assert_eq!(matching_rows(&rows, "orb"), [0, 1]);
        assert_eq!(matching_rows(&rows, ""), [0, 1]);
        assert!(matching_rows(&rows, "mirror").is_empty());
    }

    #[test]
    fn f6_dialog_prefills_the_price_rounded_to_2_decimals() {
        assert_eq!(prefill(3.456), "3.46");
        assert_eq!(prefill(2.5), "2.5");
        assert_eq!(prefill(150.0), "150");
        assert_eq!(prefill(0.0), "0");
    }

    #[test]
    fn f6_custom_price_is_a_required_number_of_0_or_more() {
        assert_eq!(parse_custom_price(" 12.5 "), Ok(12.5));
        assert_eq!(parse_custom_price("0"), Ok(0.0));
        assert_eq!(
            parse_custom_price(" "),
            Err(MessageKey::ErrorCustomPriceRequired)
        );
        for invalid in ["-1", "abc", "inf", "NaN"] {
            assert_eq!(
                parse_custom_price(invalid),
                Err(MessageKey::ErrorCustomPriceInvalid)
            );
        }
    }

    #[test]
    fn f6_a_stored_custom_price_shows_on_its_entry_only() {
        let mut rows = [row("Divine Orb", 150.0), row("Exalted Orb", 12.0)];
        let key = rows[1].key.clone();
        set_custom_price(&mut rows, &key, Some(20.0));
        assert_eq!(rows[0].custom_price, None);
        assert_eq!(rows[1].custom_price, Some(20.0));
        set_custom_price(&mut rows, &key, None);
        assert_eq!(rows[1].custom_price, None);
    }
}
