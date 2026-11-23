//! The item table (F11): the Latest view of the newest Snapshot's rows of
//! the `items` slice with every F11 column, rarity colours and poe.ninja
//! links, and the Comparison view of the two newest Snapshots. Filtering,
//! sorting, paging, column widths, the page size and column presets are UI
//! state, the layout remembered through `update_settings`; the plain
//! functions here are tested natively.
//!
//! Rows flow `items.rows` → [`filter_rows`] (tab filter, then [`Filter`]) →
//! [`sort_rows`] → [`cumulative_totals`] → [`page_range`]. The Comparison view feeds
//! `items.comparison_rows` through the filter alone and hides Tabs and
//! Cumulative.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::ops::Range;

use ipc::command::{ExportCsv, ExportCsvArgs, SettingsPatch, UpdateSettings, UpdateSettingsArgs};
use ipc::currency::NumberFormat;
use ipc::slices::{
    ColumnPreset, ITEM_TABLE_PAGE_SIZES, ItemColumn, ItemRow, ItemTab, ItemTableSort, PriceTrend,
    Settings,
};
use ipc::{Message, MessageKey};
use leptos::ev;
use leptos::prelude::*;
use regex::{Regex, RegexBuilder};

use crate::LOCALE;
use crate::charts::Sparkline;
use crate::currency::{amount, number_format};
use crate::menu::OutlinedSelect;
use crate::shell::{Icon, icon, open_link};
use crate::state::State;
use crate::tauri;

/// How a column shows (legacy `itemTableColumns.ts`, `Columns.tsx`,
/// `DefaultColumn.tsx`): header, width in pixels with its resize limits,
/// and whether it holds numbers (right-aligned).
struct ColumnSpec {
    header: MessageKey,
    width: u32,
    min: u32,
    max: u32,
    numeric: bool,
}

fn spec(column: ItemColumn) -> ColumnSpec {
    use ItemColumn as C;
    use MessageKey as K;
    let (header, width, min, max, numeric) = match column {
        C::Icon => (K::ItemTableIcon, 100, 100, 200, false),
        C::Name => (K::ItemTableName, 120, 120, 200, false),
        C::IlvlTier => (K::ItemTableIlvlTier, 60, 60, 60, true),
        C::Tabs => (K::ItemTableTabs, 100, 80, 200, false),
        C::Corrupted => (K::ItemTableCorrupted, 100, 60, 200, false),
        C::Links => (K::ItemTableLinks, 60, 60, 60, false),
        C::Quality => (K::ItemTableQuality, 60, 60, 60, true),
        C::Level => (K::ItemTableLevel, 60, 60, 60, true),
        C::Quantity => (K::ItemTableQuantity, 80, 80, 80, true),
        C::Trend => (K::ItemTableTrend, 190, 190, 190, true),
        C::Price => (K::ItemTablePrice, 100, 80, 200, true),
        C::Total => (K::ItemTableTotal, 100, 80, 200, true),
        C::Cumulative => (K::ItemTableCumulative, 100, 80, 200, true),
    };
    ColumnSpec {
        header,
        width,
        min,
        max,
        numeric,
    }
}

/// Every column sorts but Cumulative, which legacy computes per row.
pub fn sortable(column: ItemColumn) -> bool {
    column != ItemColumn::Cumulative
}

fn resizable(column: ItemColumn) -> bool {
    let spec = spec(column);
    spec.min < spec.max
}

/// The width of `column`: the remembered one within its limits, else its
/// default.
pub fn column_width(widths: &BTreeMap<ItemColumn, u32>, column: ItemColumn) -> u32 {
    let spec = spec(column);
    widths
        .get(&column)
        .map_or(spec.width, |w| (*w).clamp(spec.min, spec.max))
}

/// A column `start` pixels wide dragged by `delta` pixels, within its
/// limits.
pub fn resized_width(column: ItemColumn, start: u32, delta: f64) -> u32 {
    let spec = spec(column);
    (f64::from(start) + delta)
        .round()
        .clamp(f64::from(spec.min), f64::from(spec.max)) as u32
}

/// A click on `column`'s header: another column sorts ascending first,
/// the sorted one flips its direction.
pub fn toggle_sort(sort: ItemTableSort, column: ItemColumn) -> ItemTableSort {
    ItemTableSort {
        column,
        descending: sort.column == column && !sort.descending,
    }
}

/// Sorts `rows` by `sort`. Equal rows keep their order in both directions,
/// as react-table orders ties by row index.
pub fn sort_rows(rows: &mut [ItemRow], tabs: &[ItemTab], sort: ItemTableSort) {
    if !sortable(sort.column) {
        return;
    }
    rows.sort_by(|a, b| {
        let order = compare(a, b, sort.column, tabs);
        if sort.descending {
            order.reverse()
        } else {
            order
        }
    });
}

/// `a` against `b` by `column`, ascending, as legacy's react-table sort
/// types compare its accessors: text alphanumerically, the rest as
/// numbers; no trend sorts first.
fn compare(a: &ItemRow, b: &ItemRow, column: ItemColumn, tabs: &[ItemTab]) -> Ordering {
    let number = |f: fn(&ItemRow) -> f64| f(a).total_cmp(&f(b));
    match column {
        ItemColumn::Icon => alphanumeric(&a.icon, &b.icon),
        ItemColumn::Name => alphanumeric(&a.name, &b.name),
        ItemColumn::Tabs => alphanumeric(&a.tab_names(tabs), &b.tab_names(tabs)),
        ItemColumn::IlvlTier => ilvl_tier(a).cmp(&ilvl_tier(b)),
        ItemColumn::Corrupted => a.corrupted.cmp(&b.corrupted),
        ItemColumn::Links => a.links.cmp(&b.links),
        ItemColumn::Quality => a.quality.cmp(&b.quality),
        ItemColumn::Level => a.level.cmp(&b.level),
        ItemColumn::Quantity => a.quantity.cmp(&b.quantity),
        ItemColumn::Trend => {
            let change = |r: &ItemRow| r.trend.as_ref().map(|t| t.change);
            match (change(a), change(b)) {
                (Some(a), Some(b)) => a.total_cmp(&b),
                (a, b) => a.is_some().cmp(&b.is_some()),
            }
        }
        ItemColumn::Price => number(|r| r.price),
        ItemColumn::Total => number(|r| r.total),
        ItemColumn::Cumulative => Ordering::Equal,
    }
}

/// react-table's `alphanumeric` sort type: digit runs compare as numbers,
/// other runs as text (case-sensitive), text before numbers, and a prefix
/// first.
fn alphanumeric(a: &str, b: &str) -> Ordering {
    let (a, b) = (runs(a), runs(b));
    for (x, y) in a.iter().zip(&b) {
        let order = match (x.parse::<f64>(), y.parse::<f64>()) {
            (Ok(x), Ok(y)) => x.total_cmp(&y),
            (Ok(_), Err(_)) => Ordering::Greater,
            (Err(_), Ok(_)) => Ordering::Less,
            (Err(_), Err(_)) => x.cmp(y),
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    a.len().cmp(&b.len())
}

/// `text` split into its runs of digits and of everything else.
fn runs(text: &str) -> Vec<&str> {
    let mut runs = Vec::new();
    let mut start = 0;
    let mut digits = None;
    for (i, c) in text.char_indices() {
        let digit = c.is_ascii_digit();
        if digits.is_some_and(|d| d != digit) {
            runs.push(&text[start..i]);
            start = i;
        }
        digits = Some(digit);
    }
    if start < text.len() {
        runs.push(&text[start..]);
    }
    runs
}

/// The Ilvl / Tier column: a map's tier, else the item level.
fn ilvl_tier(row: &ItemRow) -> u32 {
    if row.tier > 0 { row.tier } else { row.ilvl }
}

/// The longest filter text (legacy `ItemTableFilter`).
pub const FILTER_MAX_CHARS: usize = 20;

/// The item table's filter text (legacy `filterSnapshotItems`,
/// `filterItems`): a row matches when it is priced and its name holds the
/// text, its tab names hold the text, it is priced and the text names its
/// rarity, or the text as a case-insensitive regular expression matches
/// its name. Case is ignored throughout; an invalid expression matches
/// every name, as legacy then falls back to an empty one.
pub struct Filter {
    text: String,
    rarity: Option<u32>,
    name: Option<Regex>,
    /// Lowercased haystack reused across rows, so a match allocates nothing
    /// once it has grown to the longest name.
    scratch: RefCell<String>,
}

impl Filter {
    pub fn new(text: &str) -> Self {
        let text = text.to_lowercase();
        Self {
            rarity: RARITIES
                .iter()
                .position(|r| *r == text)
                .map(|frame_type| frame_type as u32),
            name: RegexBuilder::new(&text).case_insensitive(true).build().ok(),
            text,
            scratch: RefCell::default(),
        }
    }

    pub fn matches(&self, row: &ItemRow, tabs: &[ItemTab], excluded: &[u32]) -> bool {
        let priced = row.price > 0.0;
        let tab_names = row
            .tabs
            .iter()
            .filter(|t| !excluded.contains(&t.tab))
            .filter_map(|t| tabs.get(t.tab as usize))
            .map(|t| t.name.as_str());
        (priced && self.lowercase_holds_text(std::iter::once(row.name.as_str())))
            || self.lowercase_holds_text(tab_names)
            || (priced && self.rarity == Some(row.frame_type))
            || self.name.as_ref().is_none_or(|n| n.is_match(&row.name))
    }

    /// Whether `parts`, joined by ", " and lowercased, hold the text.
    fn lowercase_holds_text<'a>(&self, parts: impl Iterator<Item = &'a str>) -> bool {
        let mut haystack = self.scratch.borrow_mut();
        haystack.clear();
        for (i, part) in parts.enumerate() {
            if i > 0 {
                haystack.push_str(", ");
            }
            haystack.extend(part.chars().flat_map(char::to_lowercase));
        }
        haystack.contains(&self.text)
    }
}

/// The rows the table shows before sorting: those in the tabs not in
/// `excluded` (the tab filter, legacy `filteredStashTabs`) that match
/// `filter`. Each row keeps its quantity and total in the remaining tabs
/// alone; rows in none of them go. Only the rows that stay are copied.
pub fn filter_rows(
    rows: &[ItemRow],
    tabs: &[ItemTab],
    filter: &Filter,
    excluded: &[u32],
) -> Vec<ItemRow> {
    rows.iter()
        .filter(|row| {
            (excluded.is_empty() || row.tabs.iter().any(|t| !excluded.contains(&t.tab)))
                && filter.matches(row, tabs, excluded)
        })
        .map(|row| {
            let mut row = row.clone();
            if !excluded.is_empty() {
                row.tabs.retain(|t| !excluded.contains(&t.tab));
                row.quantity = row.tabs.iter().map(|t| t.quantity).sum();
                row.total = row.quantity as f64 * row.price;
            }
            row
        })
        .collect()
}

/// The subtotal of the filtered rows, in chaos (legacy
/// `ItemTableFilterSubtotal`).
pub fn subtotal(rows: &[ItemRow]) -> f64 {
    rows.iter().map(|r| r.total).sum()
}

/// Whether the subtotal shows: while there is filter text or the tab
/// filter is open.
pub fn filtering(text: &str, tab_filter_open: bool) -> bool {
    !text.is_empty() || tab_filter_open
}

/// The columns the Comparison view shows: the visible ones but Tabs and
/// Cumulative; Name when none is left.
pub fn comparison_columns(visible: &[ItemColumn]) -> Vec<ItemColumn> {
    let columns: Vec<_> = visible
        .iter()
        .copied()
        .filter(|c| !matches!(c, ItemColumn::Tabs | ItemColumn::Cumulative))
        .collect();
    if columns.is_empty() {
        vec![ItemColumn::Name]
    } else {
        columns
    }
}

/// The columns the Latest view shows in a Group: the visible ones but Tabs,
/// as the merged rows of several players have none (F11); Name when none
/// is left.
pub fn group_columns(visible: &[ItemColumn]) -> Vec<ItemColumn> {
    let columns: Vec<_> = visible
        .iter()
        .copied()
        .filter(|c| *c != ItemColumn::Tabs)
        .collect();
    if columns.is_empty() {
        vec![ItemColumn::Name]
    } else {
        columns
    }
}

/// A Comparison difference: prefixed `+ ` when positive, as legacy's diff
/// cells show it.
pub fn diff_text(text: String, value: f64) -> String {
    if value > 0.0 {
        format!("+ {text}")
    } else {
        text
    }
}

/// The label of a preset: legacy's for its presets, else its name.
pub fn preset_label(name: &str) -> String {
    use MessageKey as K;
    let key = match name {
        ColumnPreset::CUSTOM => K::ItemTablePresetCustom,
        "breachstones" => K::ItemTablePresetBreachstones,
        "catalysts" => K::ItemTablePresetCatalysts,
        "delirium_orbs" => K::ItemTablePresetDeliriumOrbs,
        "divination_cards" => K::ItemTablePresetDivinationCards,
        "essences" => K::ItemTablePresetEssences,
        "fossils" => K::ItemTablePresetFossils,
        "incubators" => K::ItemTablePresetIncubators,
        "prophecies" => K::ItemTablePresetProphecies,
        "scarabs" => K::ItemTablePresetScarabs,
        "sets" => K::ItemTablePresetSets,
        _ => return name.to_owned(),
    };
    LOCALE.key(key)
}

/// Cumulative: each row's total plus the totals of every row sorted
/// before it, across pages.
pub fn cumulative_totals(rows: &[ItemRow]) -> Vec<f64> {
    rows.iter()
        .scan(0.0, |sum, row| {
            *sum += row.total;
            Some(*sum)
        })
        .collect()
}

/// How many pages `rows` rows fill; at least one.
pub fn page_count(rows: usize, page_size: u32) -> usize {
    rows.div_ceil(page_size.max(1) as usize).max(1)
}

/// The rows on `page` (from 0), the last page when beyond it.
pub fn page_range(rows: usize, page: usize, page_size: u32) -> Range<usize> {
    let size = page_size.max(1) as usize;
    let page = page.min(page_count(rows, page_size) - 1);
    let start = page * size;
    start..(start + size).min(rows)
}

/// The page showing the first row of `page` once the page size changes
/// from `from` to `to` (react-table `setPageSize`).
pub fn page_after_resize(page: usize, from: u32, to: u32) -> usize {
    page * from as usize / to.max(1) as usize
}

/// The pager's `1–25 of 300`.
pub fn pager_text(rows: usize, page: usize, page_size: u32) -> Message {
    let range = page_range(rows, page, page_size);
    Message::new(MessageKey::ItemTablePage)
        .arg("from", (range.start + 1).min(rows) as u64)
        .arg("to", range.end as u64)
        .arg("count", rows as u64)
}

/// The columns the active preset hides; none without such a preset.
pub fn hidden_columns(settings: &Settings) -> &[ItemColumn] {
    settings
        .item_table_presets
        .iter()
        .find(|p| p.name == settings.item_table_active_preset)
        .map_or(&[], |p| &p.hidden_columns)
}

/// The columns not in `hidden`, in display order.
pub fn visible_columns(hidden: &[ItemColumn]) -> Vec<ItemColumn> {
    ItemColumn::ALL
        .into_iter()
        .filter(|c| !hidden.contains(c))
        .collect()
}

/// `presets` with `column` shown again or hidden in the `active` preset;
/// `None` when that would hide the last visible column.
pub fn toggle_column(
    presets: &[ColumnPreset],
    active: &str,
    column: ItemColumn,
) -> Option<Vec<ColumnPreset>> {
    let mut presets = presets.to_vec();
    let index = match presets.iter().position(|p| p.name == active) {
        Some(index) => index,
        None => {
            presets.push(ColumnPreset {
                name: active.to_owned(),
                hidden_columns: Vec::new(),
            });
            presets.len() - 1
        }
    };
    let hidden = &mut presets[index].hidden_columns;
    if let Some(at) = hidden.iter().position(|c| *c == column) {
        hidden.remove(at);
    } else if visible_columns(hidden).len() > 1 {
        hidden.push(column);
    } else {
        return None;
    }
    Some(presets)
}

/// Legacy's rarities by frame type: colours and filter keywords.
const RARITIES: [&str; 10] = [
    "normal",
    "magic",
    "rare",
    "unique",
    "gem",
    "currency",
    "divination",
    "quest",
    "unknown",
    "legacy",
];

/// The legacy rarity of a frame type (`getRarity`), which names its colour.
pub fn rarity(frame_type: u32) -> &'static str {
    RARITIES
        .get(frame_type as usize)
        .copied()
        .unwrap_or(RARITIES[0])
}

/// A chaos value with 2 decimals in the system locale; blank for 0, as
/// legacy `ItemValueCell` shows it.
pub fn chaos_text(value: f64, number: NumberFormat) -> String {
    if value == 0.0 {
        String::new()
    } else {
        number.format(value, 2)
    }
}

/// The trend's change as legacy shows it: `{totalChange} %`.
pub fn trend_text(change: f64) -> String {
    format!("{change} %")
}

/// "Price last 7 days" (D3): the sparkline and the change, coloured by its
/// sign; nothing without a day with data.
pub fn trend_cell(trend: &PriceTrend) -> Option<AnyView> {
    if !trend.points.iter().any(Option::is_some) {
        return None;
    }
    let (positive, negative) = (trend.change > 0.0, trend.change < 0.0);
    Some(
        view! {
            <div class="trend">
                <Sparkline values=Signal::stored(trend.points.clone()) />
                <span class="ellipsis" class:positive=positive class:negative=negative>
                    {trend_text(trend.change)}
                </span>
            </div>
        }
        .into_any(),
    )
}

/// Stores a layout change; the pushed `settings` slice shows it.
fn save(patch: SettingsPatch) {
    tauri::send::<UpdateSettings>(UpdateSettingsArgs { patch });
}

/// The item table: the Latest or Comparison view, sortable and pageable,
/// with the filter, tab filter, subtotal, column menu and CSV export.
/// Switching Profile, a new Snapshot, another view, filter or sort start
/// again at the first page (F2, react-table's `autoResetPage`).
#[component]
pub fn ItemTable(state: State) -> impl IntoView {
    let page = RwSignal::new(0usize);
    let sort = Memo::new(move |_| state.settings.with(|s| s.item_table_sort));
    let comparison = RwSignal::new(false);
    let filter_text = RwSignal::new(String::new());
    let tab_filter_open = RwSignal::new(false);
    // The tabs the tab filter leaves out; all tabs are selected again when
    // the tabs change.
    let excluded = RwSignal::new(Vec::<u32>::new());
    let tabs = Memo::new(move |_| state.items.with(|i| i.tabs.clone()));
    Effect::new(move |_| {
        tabs.track();
        excluded.set(Vec::new());
    });
    let first_page_when = Memo::new(move |_| {
        (
            state.profiles.with(|p| p.active_profile_id.clone()),
            state.net_worth.with(|n| n.last_snapshot_at),
            sort.get(),
            comparison.get(),
            filter_text.get(),
            tab_filter_open.get(),
            excluded.get(),
        )
    });
    Effect::new(move |_| {
        first_page_when.track();
        page.set(0);
    });
    let page_size = Memo::new(move |_| state.settings.with(|s| s.item_table_page_size));
    let hidden = Memo::new(move |_| state.settings.with(|s| hidden_columns(s).to_vec()));
    let columns = Memo::new(move |_| hidden.with(|h| visible_columns(h)));
    // The remembered widths, changed here while a column is dragged.
    let widths = RwSignal::new(BTreeMap::new());
    Effect::new(move |_| widths.set(state.settings.with(|s| s.item_table_column_widths.clone())));
    let in_group = Memo::new(move |_| state.items.with(|i| i.group_combined));
    // A Group's rows have no tabs to filter by.
    Effect::new(move |_| {
        if in_group.get() {
            tab_filter_open.set(false);
        }
    });
    let shown = Memo::new(move |_| {
        let visible = columns.get();
        if comparison.get() {
            comparison_columns(&visible)
        } else if in_group.get() {
            group_columns(&visible)
        } else {
            visible
        }
    });
    let rows = Memo::new(move |_| {
        let sort = sort.get();
        let filter = filter_text.with(|text| Filter::new(text));
        let comparison = comparison.get();
        let excluded = if tab_filter_open.get() && !comparison {
            excluded.get()
        } else {
            Vec::new()
        };
        state.items.with(|items| {
            let source = if comparison {
                &items.comparison_rows
            } else {
                &items.rows
            };
            let mut rows = filter_rows(source, &items.tabs, &filter, &excluded);
            sort_rows(&mut rows, &items.tabs, sort);
            rows
        })
    });
    let cumulative = Memo::new(move |_| rows.with(|r| cumulative_totals(r)));
    let count = move || rows.with(Vec::len);

    // Dragging a header's edge resizes its column; letting go remembers it.
    let resizing = StoredValue::new(None::<(ItemColumn, f64, u32)>);
    let moved = window_event_listener(ev::pointermove, move |ev| {
        if let Some((column, x, start)) = resizing.get_value() {
            let width = resized_width(column, start, f64::from(ev.client_x()) - x);
            widths.update(|w| {
                w.insert(column, width);
            });
        }
    });
    let released = window_event_listener(ev::pointerup, move |_| {
        if resizing.get_value().is_some() {
            resizing.set_value(None);
            save(SettingsPatch {
                item_table_column_widths: Some(widths.get_untracked()),
                ..SettingsPatch::default()
            });
        }
    });
    on_cleanup(move || {
        moved.remove();
        released.remove();
    });

    let header = move |column: ItemColumn| {
        let spec = spec(column);
        let label = LOCALE.key(spec.header);
        let sorted = move || sort.with(|s| s.column == column);
        // MUI's TableSortLabel keeps its arrow's place while unsorted, so
        // right-aligned headers keep clear of the divider.
        let arrow = move || {
            let path = if sorted() && !sort.with(|s| s.descending) {
                icon::ARROW_UPWARD
            } else {
                icon::ARROW_DOWNWARD
            };
            view! { <Icon path /> }
        };
        let title = if sortable(column) {
            view! {
                <button
                    class="sort"
                    on:click=move |_| {
                        save(SettingsPatch {
                            item_table_sort: Some(toggle_sort(sort.get_untracked(), column)),
                            ..SettingsPatch::default()
                        });
                    }
                >
                    <span>{label}</span>
                    {arrow}
                </button>
            }
            .into_any()
        } else {
            view! { <span class="label">{label}</span> }.into_any()
        };
        let handle = resizable(column).then(|| {
            view! {
                <span
                    class="resize-handle"
                    on:pointerdown=move |ev| {
                        ev.prevent_default();
                        let start = widths.with_untracked(|w| column_width(w, column));
                        resizing.set_value(Some((column, f64::from(ev.client_x()), start)));
                    }
                ></span>
            }
        });
        view! {
            <th class:number=spec.numeric class:sorted=sorted>
                {title}
                {handle}
            </th>
        }
    };

    let body = move || {
        let range = page_range(count(), page.get(), page_size.get());
        let columns = shown.get();
        let diff = comparison.get();
        let number = number_format();
        state.items.with(|items| {
            rows.with(|rows| {
                cumulative.with(|cumulative| {
                    rows[range.clone()]
                        .iter()
                        .zip(&cumulative[range.clone()])
                        .map(|(row, cumulative)| {
                            let cells = columns
                                .iter()
                                .map(|&c| cell(row, c, *cumulative, &items.tabs, number, diff))
                                .collect_view();
                            view! { <tr class=format!("rarity-{}", rarity(row.frame_type))>{cells}</tr> }
                        })
                        .collect_view()
                })
            })
        })
    };

    let export = move |_| {
        let row_keys = rows.with_untracked(|r| r.iter().map(|r| r.key.clone()).collect());
        tauri::send::<ExportCsv>(ExportCsvArgs { row_keys });
    };
    let view_button = move |to_comparison: bool, title: MessageKey, path: &'static str| {
        view! {
            <button
                class="control"
                class:active=move || comparison.get() == to_comparison
                title=LOCALE.key(title)
                on:click=move |_| comparison.set(to_comparison)
            >
                <Icon path />
            </button>
        }
    };
    let subtotal_text = move || {
        let chaos = rows.with(|r| subtotal(r));
        format!(
            "{} {}",
            LOCALE.key(MessageKey::ItemTableFilterTotal),
            amount(state, chaos, false)
        )
    };
    let tab_checkbox = move |(index, tab): (usize, ItemTab)| {
        let index = index as u32;
        let selected = move || excluded.with(|e| !e.contains(&index));
        view! {
            <label>
                <input
                    type="checkbox"
                    prop:checked=selected
                    on:change=move |_| {
                        excluded.update(|e| match e.iter().position(|&t| t == index) {
                            Some(at) => {
                                e.remove(at);
                            }
                            None => e.push(index),
                        })
                    }
                />
                {tab.name}
            </label>
        }
    };

    view! {
        <div class="item-table">
            <div class="item-table-toolbar">
                <div class="item-table-views">
                    {view_button(false, MessageKey::ItemTableLatest, icon::UPDATE)}
                    {view_button(true, MessageKey::ItemTableComparison, icon::COMPARE)}
                </div>
                <div class="item-table-filter">
                    <Icon path=icon::SEARCH />
                    <input
                        type="text"
                        maxlength=FILTER_MAX_CHARS.to_string()
                        placeholder=LOCALE.key(MessageKey::ItemTableSearch)
                        prop:value=move || filter_text.get()
                        on:input=move |ev| {
                            filter_text.set(event_target_value(&ev).chars().take(FILTER_MAX_CHARS).collect())
                        }
                    />
                    <Show when=move || filter_text.with(|t| !t.is_empty())>
                        <button
                            class="control"
                            title=LOCALE.key(MessageKey::ItemTableClearSearch)
                            on:click=move |_| filter_text.set(String::new())
                        >
                            <Icon path=icon::CLOSE />
                        </button>
                    </Show>
                </div>
                <Show when=move || filter_text.with(|t| filtering(t, tab_filter_open.get()))>
                    <span class="item-table-subtotal">{subtotal_text}</span>
                </Show>
                <div class="item-table-actions">
                    <ColumnMenu state hidden columns disabled=comparison.into() />
                    <button
                        class="action"
                        class:active=move || tab_filter_open.get()
                        disabled=move || comparison.get() || in_group.get()
                        on:click=move |_| tab_filter_open.update(|o| *o = !*o)
                    >
                        <Icon path=icon::FILTER_LIST />
                        {move || {
                            LOCALE.key(if tab_filter_open.get() {
                                MessageKey::ItemTableHideTabFilter
                            } else {
                                MessageKey::ItemTableShowTabFilter
                            })
                        }}
                    </button>
                    <button
                        class="action"
                        disabled=move || comparison.get() || count() == 0
                        on:click=export
                    >
                        <Icon path=icon::GET_APP />
                        {LOCALE.key(MessageKey::ItemTableExport)}
                    </button>
                </div>
            </div>
            <Show when=move || tab_filter_open.get() && !comparison.get() && tabs.with(|t| !t.is_empty())>
                <div class="item-table-tab-filter">
                    <p>{LOCALE.key(MessageKey::ItemTableSelectTabs)}</p>
                    {move || tabs.get().into_iter().enumerate().map(tab_checkbox).collect_view()}
                </div>
            </Show>
            <div class="item-table-scroll">
                <table>
                    <colgroup>
                        {move || {
                            shown
                                .get()
                                .into_iter()
                                .map(|c| {
                                    let width = move || format!("{}px", widths.with(|w| column_width(w, c)));
                                    view! { <col style:width=width /> }
                                })
                                .collect_view()
                        }}
                    </colgroup>
                    <thead>
                        <tr>{move || shown.get().into_iter().map(header).collect_view()}</tr>
                    </thead>
                    <tbody>
                        <Show
                            when=move || count() != 0
                            fallback=move || {
                                view! {
                                    <tr>
                                        <td class="empty" colspan=move || shown.with(Vec::len).to_string()>
                                            {LOCALE.key(MessageKey::ItemTableEmpty)}
                                        </td>
                                    </tr>
                                }
                            }
                        >
                            {body}
                        </Show>
                    </tbody>
                </table>
            </div>
            <Show when=move || count() != 0>
                <Pager
                    id="item-table-page-size"
                    count=Signal::derive(count)
                    page
                    page_size=page_size.into()
                    on_page_size=Callback::new(|size| {
                        save(SettingsPatch {
                            item_table_page_size: Some(size),
                            ..SettingsPatch::default()
                        })
                    })
                />
            </Show>
        </div>
    }
}

/// Legacy's `TablePagination` (MUI): rows per page, the rows shown and
/// first, previous, next and last page. A new page size keeps the first
/// row of the page in view; `on_page_size` stores it.
#[component]
pub fn Pager(
    id: &'static str,
    count: Signal<usize>,
    page: RwSignal<usize>,
    page_size: Signal<u32>,
    on_page_size: Callback<u32>,
) -> impl IntoView {
    let pages = move || page_count(count.get(), page_size.get());
    let sizes = ITEM_TABLE_PAGE_SIZES
        .map(|size| (size.to_string(), size.to_string()))
        .to_vec();
    let resize = move |size: String| {
        if let Ok(size) = size.parse::<u32>() {
            page.update(|p| *p = page_after_resize(*p, page_size.get_untracked(), size));
            on_page_size.run(size);
        }
    };
    view! {
        <div class="pager">
            // `OutlinedSelect` is labelled by `{id}-label`.
            <span id=format!("{id}-label")>{LOCALE.key(MessageKey::ItemTableRowsPerPage)}</span>
            <OutlinedSelect
                id
                standard=true
                options=Signal::stored(sizes)
                value=Signal::derive(move || page_size.get().to_string())
                on_change=Callback::new(resize)
                disabled=Signal::derive(|| false)
                error=Signal::derive(|| false)
            />
            <span>{move || LOCALE.text(&pager_text(count.get(), page.get(), page_size.get()))}</span>
            <div class="pager-actions">
                <PagerButton
                    title=MessageKey::ItemTableFirstPage
                    path=icon::FIRST_PAGE
                    disabled=Signal::derive(move || page.get() == 0)
                    on_click=move || page.set(0)
                />
                <PagerButton
                    title=MessageKey::ItemTablePreviousPage
                    path=icon::CHEVRON_LEFT
                    disabled=Signal::derive(move || page.get() == 0)
                    on_click=move || page.update(|p| *p = p.saturating_sub(1).min(pages() - 1))
                />
                <PagerButton
                    title=MessageKey::ItemTableNextPage
                    path=icon::CHEVRON_RIGHT
                    disabled=Signal::derive(move || page.get() + 1 >= pages())
                    on_click=move || page.update(|p| *p = (*p + 1).min(pages() - 1))
                />
                <PagerButton
                    title=MessageKey::ItemTableLastPage
                    path=icon::LAST_PAGE
                    disabled=Signal::derive(move || page.get() + 1 >= pages())
                    on_click=move || page.set(pages() - 1)
                />
            </div>
        </div>
    }
}

#[component]
fn PagerButton(
    title: MessageKey,
    path: &'static str,
    disabled: Signal<bool>,
    on_click: impl Fn() + 'static,
) -> impl IntoView {
    view! {
        <button class="control" title=LOCALE.key(title) disabled=disabled on:click=move |_| on_click()>
            <Icon path />
        </button>
    }
}

/// "Edit columns": the preset picker and a checkbox per column, shown or
/// hidden in the active preset; the last visible column cannot be hidden.
#[component]
fn ColumnMenu(
    state: State,
    hidden: Memo<Vec<ItemColumn>>,
    columns: Memo<Vec<ItemColumn>>,
    disabled: Signal<bool>,
) -> impl IntoView {
    let open = RwSignal::new(false);
    Effect::new(move |_| {
        if disabled.get() {
            open.set(false);
        }
    });
    let presets = move || {
        state.settings.with(|s| {
            s.item_table_presets
                .iter()
                .map(|p| {
                    let selected = p.name == s.item_table_active_preset;
                    view! {
                        <option value=p.name.clone() prop:selected=selected>
                            {preset_label(&p.name)}
                        </option>
                    }
                })
                .collect_view()
        })
    };
    let toggle = move |column| {
        let presets = state.settings.with_untracked(|s| {
            toggle_column(&s.item_table_presets, &s.item_table_active_preset, column)
        });
        if let Some(presets) = presets {
            save(SettingsPatch {
                item_table_presets: Some(presets),
                ..SettingsPatch::default()
            });
        }
    };
    let checkbox = move |column: ItemColumn| {
        let shown = move || hidden.with(|h| !h.contains(&column));
        let last = move || shown() && columns.with(|c| c.len() == 1);
        view! {
            <label>
                <input
                    type="checkbox"
                    prop:checked=shown
                    disabled=last
                    on:change=move |_| toggle(column)
                />
                {LOCALE.key(spec(column).header)}
            </label>
        }
    };
    view! {
        <div class="column-menu-anchor">
            <button
                class="action"
                class:active=move || open.get()
                disabled=disabled
                on:click=move |_| open.update(|o| *o = !*o)
            >
                <Icon path=icon::VIEW_COLUMN />
                {LOCALE.key(MessageKey::ItemTableEditColumns)}
            </button>
            <Show when=move || open.get()>
                <div class="column-menu">
                    <label class="preset">
                        {LOCALE.key(MessageKey::ItemTableColumnPreset)}
                        <select on:change=move |ev| {
                            save(SettingsPatch {
                                item_table_active_preset: Some(event_target_value(&ev)),
                                ..SettingsPatch::default()
                            });
                        }>{presets}</select>
                    </label>
                    <p>{LOCALE.key(MessageKey::ItemTableVisibleColumns)}</p>
                    {ItemColumn::ALL.map(checkbox)}
                </div>
            </Show>
        </div>
    }
}

/// The cell of `column` in `row`; `cumulative` is the row's Cumulative.
fn cell(
    row: &ItemRow,
    column: ItemColumn,
    cumulative: f64,
    tabs: &[ItemTab],
    number: NumberFormat,
    diff: bool,
) -> AnyView {
    let numeric = spec(column).numeric;
    // Comparison differences: `+ ` when positive, coloured by sign.
    let change = |text: String, value: f64| {
        let (positive, negative) = (value > 0.0, value < 0.0);
        view! {
            <span class:positive=positive class:negative=negative>{diff_text(text, value)}</span>
        }
        .into_any()
    };
    let content = match column {
        ItemColumn::Icon => view! {
            <div class="item-icon-cell">
                <img src=row.icon.clone() alt=LOCALE.key(MessageKey::ItemTableIconAlt) />
            </div>
        }
        .into_any(),
        ItemColumn::Name => {
            let link = row.ninja_url.clone().map(|url| {
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
            view! {
                <div class="item-name">
                    <span class="ellipsis" title=row.name.clone()>{row.name.clone()}</span>
                    {link}
                </div>
            }
            .into_any()
        }
        ItemColumn::IlvlTier => ilvl_tier(row).into_any(),
        ItemColumn::Tabs => {
            let names = row.tab_names(tabs);
            let title = names.clone();
            view! { <span class="ellipsis" title=title>{names}</span> }.into_any()
        }
        ItemColumn::Corrupted => {
            let key = if row.corrupted {
                MessageKey::ItemTableYes
            } else {
                MessageKey::ItemTableNo
            };
            view! { <span class:corrupted=row.corrupted>{LOCALE.key(key)}</span> }.into_any()
        }
        ItemColumn::Links if row.links == 0 => view! {
            <span class="unavailable">{LOCALE.key(MessageKey::ItemTableNotAvailable)}</span>
        }
        .into_any(),
        ItemColumn::Links => row.links.into_any(),
        ItemColumn::Quality => row.quality.into_any(),
        ItemColumn::Level => row.level.into_any(),
        ItemColumn::Quantity if diff => change(row.quantity.to_string(), row.quantity as f64),
        ItemColumn::Quantity => row.quantity.into_any(),
        ItemColumn::Trend => row.trend.as_ref().and_then(trend_cell).into_any(),
        ItemColumn::Price => chaos_text(row.price, number).into_any(),
        ItemColumn::Total if diff => change(chaos_text(row.total, number), row.total),
        ItemColumn::Total => chaos_text(row.total, number).into_any(),
        ItemColumn::Cumulative => chaos_text(cumulative, number).into_any(),
    };
    let icon = column == ItemColumn::Icon;
    view! { <td class:number=numeric class:icon-cell=icon>{content}</td> }.into_any()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ipc::slices::{ItemRowTab, PriceTrend};

    fn row(name: &str, quantity: i64, price: f64) -> ItemRow {
        ItemRow {
            key: name.to_owned(),
            name: name.to_owned(),
            icon: String::new(),
            frame_type: 5,
            ilvl: 0,
            tier: 0,
            corrupted: false,
            links: 0,
            quality: 0,
            level: 0,
            quantity,
            price,
            total: quantity as f64 * price,
            tabs: Vec::new(),
            trend: None,
            ninja_url: None,
        }
    }

    fn names(rows: &[ItemRow]) -> Vec<&str> {
        rows.iter().map(|r| r.name.as_str()).collect()
    }

    fn sorted(rows: &[ItemRow], column: ItemColumn, descending: bool) -> Vec<ItemRow> {
        let mut rows = rows.to_vec();
        sort_rows(&mut rows, &[], ItemTableSort { column, descending });
        rows
    }

    #[test]
    fn f11_sort_by_total_descending_by_default() {
        let rows = [row("a", 1, 5.0), row("b", 10, 1.0), row("c", 1, 20.0)];
        let sort = Settings::default().item_table_sort;
        let mut sorted = rows.to_vec();
        sort_rows(&mut sorted, &[], sort);
        assert_eq!(names(&sorted), ["c", "b", "a"]);
    }

    #[test]
    fn f11_sort_numbers_as_numbers_and_keep_ties_in_order() {
        let rows = [
            row("a", 2, 1.0),
            row("b", 10, 1.0),
            row("c", 2, 1.0),
            row("d", 1, 1.0),
        ];
        assert_eq!(
            names(&sorted(&rows, ItemColumn::Quantity, false)),
            ["d", "a", "c", "b"]
        );
        assert_eq!(
            names(&sorted(&rows, ItemColumn::Quantity, true)),
            ["b", "a", "c", "d"]
        );
        let mut map = row("map", 1, 1.0);
        map.tier = 16;
        map.ilvl = 83;
        let mut ring = row("ring", 1, 1.0);
        ring.ilvl = 84;
        let mut low_map = row("low map", 1, 1.0);
        low_map.tier = 1;
        low_map.ilvl = 85;
        assert_eq!(
            names(&sorted(&[map, ring, low_map], ItemColumn::IlvlTier, false)),
            ["low map", "map", "ring"],
            "a tier counts instead of the item level"
        );
    }

    #[test]
    fn f11_sort_text_alphanumerically() {
        let rows = [
            row("Map (Tier 10)", 1, 1.0),
            row("Map (Tier 9)", 1, 1.0),
            row("Chaos Orb", 1, 1.0),
            row("chaos shard", 1, 1.0),
            row("Map", 1, 1.0),
        ];
        assert_eq!(
            names(&sorted(&rows, ItemColumn::Name, false)),
            [
                "Chaos Orb",
                "Map",
                "Map (Tier 9)",
                "Map (Tier 10)",
                "chaos shard"
            ]
        );
        assert_eq!(alphanumeric("a1", "a"), Ordering::Greater, "a prefix first");
        assert_eq!(
            alphanumeric("1", "a"),
            Ordering::Greater,
            "text before numbers"
        );
    }

    #[test]
    fn f11_sort_by_tabs_trend_and_corruption() {
        let tabs = [
            ItemTab {
                id: "t1".into(),
                name: "B".into(),
            },
            ItemTab {
                id: "t2".into(),
                name: "A".into(),
            },
        ];
        let in_tabs = |name, indices: &[u32]| ItemRow {
            tabs: indices
                .iter()
                .map(|&tab| ItemRowTab { tab, quantity: 1 })
                .collect(),
            ..row(name, 1, 1.0)
        };
        let mut rows = vec![
            in_tabs("x", &[0]),
            in_tabs("y", &[1, 0]),
            in_tabs("z", &[1]),
        ];
        assert_eq!(rows[1].tab_names(&tabs), "A, B");
        sort_rows(
            &mut rows,
            &tabs,
            ItemTableSort {
                column: ItemColumn::Tabs,
                descending: false,
            },
        );
        assert_eq!(names(&rows), ["z", "y", "x"]);

        let trend = |name, change: Option<f64>| ItemRow {
            trend: change.map(|change| PriceTrend {
                points: vec![Some(0.0), Some(change)],
                change,
            }),
            ..row(name, 1, 1.0)
        };
        let rows = [
            trend("up", Some(5.0)),
            trend("none", None),
            trend("down", Some(-3.0)),
        ];
        assert_eq!(
            names(&sorted(&rows, ItemColumn::Trend, false)),
            ["none", "down", "up"],
            "no trend sorts first"
        );

        let corrupted = ItemRow {
            corrupted: true,
            ..row("corrupted", 1, 1.0)
        };
        let rows = [corrupted, row("clean", 1, 1.0)];
        assert_eq!(
            names(&sorted(&rows, ItemColumn::Corrupted, false)),
            ["clean", "corrupted"]
        );
    }

    #[test]
    fn f11_cumulative_does_not_sort_and_sums_in_sorted_order() {
        let rows = [row("a", 1, 5.0), row("b", 1, 20.0)];
        assert_eq!(
            names(&sorted(&rows, ItemColumn::Cumulative, true)),
            ["a", "b"]
        );
        let by_total = sorted(&rows, ItemColumn::Total, true);
        assert_eq!(cumulative_totals(&by_total), [20.0, 25.0]);
    }

    #[test]
    fn f11_header_click_sorts_ascending_then_flips() {
        let by_total = Settings::default().item_table_sort;
        let by_name = toggle_sort(by_total, ItemColumn::Name);
        assert_eq!(
            by_name,
            ItemTableSort {
                column: ItemColumn::Name,
                descending: false,
            }
        );
        assert!(toggle_sort(by_name, ItemColumn::Name).descending);
        assert!(!toggle_sort(toggle_sort(by_name, ItemColumn::Name), ItemColumn::Name).descending);
    }

    #[test]
    fn f11_paging_splits_rows_into_pages_of_the_page_size() {
        assert_eq!(page_count(0, 25), 1);
        assert_eq!(page_count(25, 25), 1);
        assert_eq!(page_count(26, 25), 2);
        assert_eq!(page_range(60, 0, 25), 0..25);
        assert_eq!(page_range(60, 2, 25), 50..60);
        assert_eq!(
            page_range(60, 9, 25),
            50..60,
            "beyond the end: the last page"
        );
        assert_eq!(page_range(3, 0, 5), 0..3);
        assert_eq!(page_range(0, 0, 50), 0..0);
        for size in ITEM_TABLE_PAGE_SIZES {
            assert_eq!(page_range(100, 1, size).len(), size as usize);
        }
    }

    #[test]
    fn f11_page_size_change_keeps_the_first_row_in_view() {
        assert_eq!(
            page_after_resize(3, 10, 25),
            1,
            "row 30 lies on page 1 of 25"
        );
        assert_eq!(page_after_resize(1, 50, 5), 10);
        assert_eq!(page_after_resize(0, 25, 50), 0);
    }

    #[test]
    fn f11_pager_names_the_rows_shown() {
        let text = |rows, page, size| LOCALE.text(&pager_text(rows, page, size));
        assert_eq!(text(300, 0, 25), "1–25 of 300");
        assert_eq!(text(60, 2, 25), "51–60 of 60");
        assert_eq!(text(0, 0, 25), "0–0 of 0");
    }

    #[test]
    fn f11_hidden_columns_come_from_the_active_preset() {
        let mut settings = Settings::default();
        assert_eq!(
            visible_columns(hidden_columns(&settings)),
            [
                ItemColumn::Icon,
                ItemColumn::Name,
                ItemColumn::Tabs,
                ItemColumn::Links,
                ItemColumn::Quantity,
                ItemColumn::Trend,
                ItemColumn::Price,
                ItemColumn::Total,
                ItemColumn::Cumulative,
            ]
        );
        settings.item_table_active_preset = "gone".into();
        assert!(hidden_columns(&settings).is_empty());
    }

    #[test]
    fn f11_toggling_a_column_updates_the_active_preset_but_keeps_one_visible() {
        let settings = Settings::default();
        let active = &settings.item_table_active_preset;
        let presets =
            toggle_column(&settings.item_table_presets, active, ItemColumn::Quality).unwrap();
        assert!(
            !presets[0].hidden_columns.contains(&ItemColumn::Quality),
            "shown again"
        );
        let presets = toggle_column(&presets, active, ItemColumn::Name).unwrap();
        assert!(presets[0].hidden_columns.contains(&ItemColumn::Name));
        assert_eq!(
            presets[1..],
            settings.item_table_presets[1..],
            "other presets unchanged"
        );

        let all_but_icon: Vec<ItemColumn> = ItemColumn::ALL[1..].to_vec();
        let only_icon = [ColumnPreset {
            name: "custom".into(),
            hidden_columns: all_but_icon,
        }];
        assert_eq!(toggle_column(&only_icon, "custom", ItemColumn::Icon), None);
        let missing = toggle_column(&[], "custom", ItemColumn::Tabs).unwrap();
        assert_eq!(missing[0].hidden_columns, [ItemColumn::Tabs]);
    }

    #[test]
    fn f11_widths_are_remembered_within_each_columns_limits() {
        let mut widths = BTreeMap::new();
        assert_eq!(column_width(&widths, ItemColumn::Name), 120);
        assert_eq!(column_width(&widths, ItemColumn::Trend), 190);
        widths.insert(ItemColumn::Name, 180);
        widths.insert(ItemColumn::Tabs, 5000);
        assert_eq!(column_width(&widths, ItemColumn::Name), 180);
        assert_eq!(column_width(&widths, ItemColumn::Tabs), 200);
        assert_eq!(resized_width(ItemColumn::Name, 120, 33.4), 153);
        assert_eq!(resized_width(ItemColumn::Name, 120, -50.0), 120);
        assert_eq!(resized_width(ItemColumn::Price, 100, 500.0), 200);
        assert!(!resizable(ItemColumn::Trend));
        assert!(resizable(ItemColumn::Tabs));
    }

    #[test]
    fn f11_rarity_colours_follow_legacy_frame_types() {
        assert_eq!(rarity(0), "normal");
        assert_eq!(rarity(3), "unique");
        assert_eq!(rarity(4), "gem");
        assert_eq!(rarity(5), "currency");
        assert_eq!(rarity(6), "divination");
        assert_eq!(rarity(9), "legacy");
        assert_eq!(rarity(11), "normal", "unknown frame types show as normal");
    }

    #[test]
    fn f11_values_show_two_decimals_and_blank_for_zero() {
        assert_eq!(chaos_text(1234.5, NumberFormat::ENGLISH), "1,234.50");
        assert_eq!(chaos_text(0.0, NumberFormat::ENGLISH), "");
        assert_eq!(trend_text(4.5), "4.5 %");
        assert_eq!(trend_text(-20.0), "-20 %");
    }

    fn in_tabs(mut row: ItemRow, tabs: &[(u32, i64)]) -> ItemRow {
        row.tabs = tabs
            .iter()
            .map(|&(tab, quantity)| ItemRowTab { tab, quantity })
            .collect();
        row
    }

    fn tabs(names: &[&str]) -> Vec<ItemTab> {
        names
            .iter()
            .map(|name| ItemTab {
                id: String::new(),
                name: (*name).to_owned(),
            })
            .collect()
    }

    fn filtered<'a>(rows: &'a [ItemRow], tabs: &[ItemTab], text: &str) -> Vec<&'a str> {
        let filter = Filter::new(text);
        rows.iter()
            .filter(|r| filter.matches(r, tabs, &[]))
            .map(|r| r.name.as_str())
            .collect()
    }

    #[test]
    fn f11_filter_matches_priced_names_any_tab_name_and_priced_rarity() {
        let tabs = tabs(&["Dump", "Maps"]);
        let mut gem = row("Vaal Grace", 1, 3.0);
        gem.frame_type = 4;
        let mut unpriced_gem = row("Cheap Gem", 1, 0.0);
        unpriced_gem.frame_type = 4;
        let rows = [
            in_tabs(row("Chaos Orb", 5, 1.0), &[(0, 5)]),
            in_tabs(row("Strand Map", 1, 0.0), &[(1, 1)]),
            in_tabs(gem, &[(0, 1)]),
            in_tabs(unpriced_gem, &[(0, 1)]),
        ];
        assert_eq!(
            filtered(&rows, &tabs, "CHAOS"),
            ["Chaos Orb"],
            "case-insensitive"
        );
        assert_eq!(
            filtered(&rows, &tabs, "maps"),
            ["Strand Map"],
            "any item by tab name"
        );
        assert_eq!(
            filtered(&rows, &tabs, "che.p"),
            ["Cheap Gem"],
            "the regex finds the unpriced one"
        );
        assert_eq!(
            filtered(&rows, &tabs, "gem"),
            ["Vaal Grace", "Cheap Gem"],
            "priced items by rarity keyword, the rest by name"
        );
        assert_eq!(filtered(&rows, &tabs, "").len(), rows.len());
    }

    #[test]
    fn f11_filter_rarity_keywords_follow_frame_types() {
        for (frame_type, keyword) in RARITIES.iter().enumerate() {
            let mut item = row("Item", 1, 1.0);
            item.frame_type = frame_type as u32;
            assert_eq!(filtered(&[item], &[], keyword), ["Item"], "{keyword}");
        }
        let mut unique = row("Item", 1, 1.0);
        unique.frame_type = 3;
        assert!(filtered(&[unique], &[], "magic").is_empty());
    }

    #[test]
    fn f11_filter_is_a_case_insensitive_regex_and_an_invalid_one_matches_all() {
        let rows = [
            row("Chaos Orb", 1, 0.0),
            row("Divine Orb", 1, 0.0),
            row("Orb of Alchemy", 1, 0.0),
        ];
        assert_eq!(
            filtered(&rows, &[], "^(chaos|divine)"),
            ["Chaos Orb", "Divine Orb"]
        );
        assert_eq!(filtered(&rows, &[], "ORB$"), ["Chaos Orb", "Divine Orb"]);
        assert_eq!(
            filtered(&rows, &[], "[").len(),
            3,
            "legacy falls back to an empty regex"
        );
    }

    #[test]
    fn f11_filter_keeps_at_most_20_characters() {
        assert_eq!(FILTER_MAX_CHARS, 20);
    }

    #[test]
    fn f11_tab_filter_narrows_rows_to_the_selected_tabs() {
        let rows = [
            in_tabs(row("Chaos Orb", 17, 2.0), &[(0, 7), (1, 10)]),
            in_tabs(row("Divine Orb", 2, 150.0), &[(1, 2)]),
        ];
        let all = Filter::new("");
        assert_eq!(
            filter_rows(&rows, &[], &all, &[]),
            rows,
            "all tabs selected by default"
        );
        let narrowed = filter_rows(&rows, &[], &all, &[1]);
        assert_eq!(names(&narrowed), ["Chaos Orb"]);
        assert_eq!((narrowed[0].quantity, narrowed[0].total), (7, 14.0));
        assert_eq!(
            narrowed[0].tabs,
            [ItemRowTab {
                tab: 0,
                quantity: 7
            }]
        );

        let tabs = tabs(&["Currency", "Dump"]);
        let both = filter_rows(&rows, &tabs, &Filter::new("dump"), &[0]);
        assert_eq!(names(&both), ["Chaos Orb", "Divine Orb"]);
        assert_eq!(
            names(&filter_rows(&rows, &tabs, &Filter::new("dump"), &[1])),
            Vec::<&str>::new()
        );
    }

    #[test]
    fn f11_subtotal_sums_the_filtered_rows_while_a_filter_is_in_use() {
        let rows = [row("Chaos Orb", 17, 1.0), row("Divine Orb", 2, 150.0)];
        let shown = filter_rows(&rows, &[], &Filter::new("divine"), &[]);
        assert_eq!(subtotal(&shown), 300.0);
        assert_eq!(subtotal(&rows), 317.0);
        assert!(!filtering("", false));
        assert!(filtering("d", false));
        assert!(filtering("", true), "the open tab filter shows it too");
    }

    #[test]
    fn f11_comparison_hides_tabs_and_cumulative_and_prefixes_gains() {
        assert_eq!(
            comparison_columns(&ItemColumn::ALL),
            ItemColumn::ALL
                .into_iter()
                .filter(|c| !matches!(c, ItemColumn::Tabs | ItemColumn::Cumulative))
                .collect::<Vec<_>>()
        );
        assert_eq!(comparison_columns(&[ItemColumn::Tabs]), [ItemColumn::Name]);
        assert_eq!(diff_text("3".to_owned(), 3.0), "+ 3");
        assert_eq!(diff_text("-3".to_owned(), -3.0), "-3");
    }

    #[test]
    fn f11_group_latest_hides_tabs() {
        assert_eq!(
            group_columns(&ItemColumn::ALL),
            ItemColumn::ALL
                .into_iter()
                .filter(|c| *c != ItemColumn::Tabs)
                .collect::<Vec<_>>()
        );
        assert_eq!(group_columns(&[ItemColumn::Tabs]), [ItemColumn::Name]);
    }

    #[test]
    fn f11_presets_show_legacy_labels() {
        assert_eq!(preset_label(ColumnPreset::CUSTOM), "Custom");
        assert_eq!(preset_label("divination_cards"), "Divination Cards");
        assert_eq!(preset_label("mine"), "mine");
        for preset in ColumnPreset::legacy() {
            assert_ne!(preset_label(&preset.name), preset.name);
        }
    }
}
