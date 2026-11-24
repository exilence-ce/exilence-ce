//! Legacy v1.3.0 pricing (F5, F6): which poe.ninja price entries survive the
//! pricing options, how an item matches one, and how a Snapshot's tabs are
//! merged into rows and valued. Ports `exilence-ce-legacy/src/store/domains/profile.ts:640-688`,
//! `src/services/pricing.service.ts` and `src/utils/price.utils.ts`, with
//! D1, D2 and D29–D31 applied.

pub mod ninja;

use std::collections::hash_map::{Entry, HashMap};

use serde::Deserialize;

use crate::ggg::Item;
use crate::item_definition::{ItemDefinition, frame_type};

/// The fields that identify a price entry (legacy `findPrice`). `None` is a
/// field the entry lacks (legacy `undefined`): currency-overview entries carry
/// only name, icon and frame type.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct PriceKey {
    pub name: String,
    /// Empty when poe.ninja sends none.
    pub icon: String,
    pub frame_type: Option<u32>,
    pub variant: Option<String>,
    pub links: Option<u32>,
    pub quality: Option<u32>,
    pub level: Option<u32>,
    pub ilvl: Option<u32>,
    pub tier: Option<u32>,
    pub corrupted: Option<bool>,
    pub elder: Option<bool>,
    pub shaper: Option<bool>,
}

/// One poe.ninja price of a price league.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PriceEntry {
    pub key: PriceKey,
    /// Chaos.
    pub calculated: f64,
    /// Listings (or exchange trades) backing the price.
    pub count: u64,
    /// The price's trend over the last 7 days; `None` when poe.ninja sends
    /// none (D3).
    pub sparkline: Option<Sparkline>,
    /// The entry's poe.ninja page below the league, `<type>/<details id>`
    /// (D3, [`ninja::details_url`]).
    pub details_page: Option<String>,
}

/// poe.ninja's trend of a price over the last 7 days (D3).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sparkline {
    /// Change in percent against the first day, one point per day, oldest
    /// first; `None` for a day without data.
    #[serde(default)]
    pub data: Vec<Option<f64>>,
    /// Change in percent over the 7 days.
    #[serde(default)]
    pub total_change: f64,
}

/// A user's price for one price entry (F6).
#[derive(Debug, Clone, PartialEq)]
pub struct CustomPrice {
    pub key: PriceKey,
    /// Chaos. 0 leaves the poe.ninja price in place, as in legacy
    /// (`customPrice || calculated`).
    pub value: f64,
}

/// Chaos per unit of the currency named `name` (F6, F9): the first entry so
/// named, at its custom price when that is set and above 0 (legacy
/// `priceStore.exaltedPrice`, `divinePrice`). `None` without such an entry.
pub fn currency_rate(
    entries: &[PriceEntry],
    custom_prices: &[CustomPrice],
    name: &str,
) -> Option<f64> {
    let entry = entries.iter().find(|p| p.key.name == name)?;
    let custom = custom_prices
        .iter()
        .find(|c| c.key == entry.key)
        .map(|c| c.value)
        .filter(|v| *v > 0.0);
    Some(custom.unwrap_or(entry.calculated))
}

/// `entries` without older map generations: maps of an older atlas
/// generation linger on poe.ninja as `, Gen-<n>` variants, and only the
/// newest generation among `entries` stays (legacy `excludeLegacyMaps`).
pub fn newest_map_generation<'a>(
    entries: impl IntoIterator<Item = &'a PriceEntry>,
) -> Vec<&'a PriceEntry> {
    let mut entries: Vec<&PriceEntry> = entries.into_iter().collect();
    let newest = entries
        .iter()
        .filter_map(|p| map_generation(&p.key))
        .max()
        .unwrap_or(0);
    entries.retain(|p| map_generation(&p.key).is_none_or(|g| g == newest));
    entries
}

/// The pricing options of F5, as of the Snapshot being taken.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PricingOptions {
    /// Off: prices backed by 10 or fewer listings are ignored.
    pub low_confidence_pricing: bool,
    /// Per-item threshold in chaos (0–100); applies only while
    /// `total_price_threshold` is 0.
    pub price_threshold: f64,
    /// Total threshold in chaos (0–5000) for merged item rows.
    pub total_price_threshold: f64,
    /// On: Harvest Crystallised Lifeforce and Rogue's Markers bypass the
    /// per-item `price_threshold`.
    pub exempt_lifeforce_and_markers: bool,
}

/// Item names legacy never prices.
const INVALID_ITEMS: [&str; 1] = ["Charged Compass"];

/// Stack-dropping currencies that can bypass the per-item `price_threshold`.
const THRESHOLD_EXEMPT_ITEMS: [&str; 4] = [
    "Vivid Crystallised Lifeforce",
    "Wild Crystallised Lifeforce",
    "Primal Crystallised Lifeforce",
    "Rogue's Marker",
];

/// The price entries that survive the pricing options, custom prices
/// applied, indexed for matching. Every legacy matching rule compares the
/// entry's name first, so entries are grouped by name in price-list order.
#[derive(Debug)]
pub struct PriceTable<'a> {
    by_name: HashMap<&'a str, Candidates<'a>>,
    total_price_threshold: f64,
}

#[derive(Debug)]
enum Candidates<'a> {
    One(Candidate<'a>),
    Many(Vec<Candidate<'a>>),
}

impl<'a> Candidates<'a> {
    fn push(&mut self, candidate: Candidate<'a>) {
        match self {
            Self::One(first) => *self = Self::Many(vec![*first, candidate]),
            Self::Many(rest) => rest.push(candidate),
        }
    }

    fn as_slice(&self) -> &[Candidate<'a>] {
        match self {
            Self::One(first) => std::slice::from_ref(first),
            Self::Many(rest) => rest.as_slice(),
        }
    }

    fn as_mut_slice(&mut self) -> &mut [Candidate<'a>] {
        match self {
            Self::One(first) => std::slice::from_mut(first),
            Self::Many(rest) => rest.as_mut_slice(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Candidate<'a> {
    entry: &'a PriceEntry,
    key: &'a PriceKey,
    /// Custom price if set and non-zero, else the poe.ninja price.
    unit_price: f64,
}

impl<'a> PriceTable<'a> {
    /// Filters the price league's entries as legacy does before pricing
    /// (`profile.ts:646-670`): low-confidence, per-item threshold, older map
    /// generations, invalid items; then applies each custom price to the
    /// first surviving entry it identifies (F6).
    pub fn new(
        entries: &'a [PriceEntry],
        options: &PricingOptions,
        custom_prices: &[CustomPrice],
    ) -> Self {
        let mut surviving = newest_map_generation(
            entries
                .iter()
                .filter(|p| options.low_confidence_pricing || p.count > 10)
                .filter(|p| {
                    options.total_price_threshold != 0.0
                        || (p.calculated != 0.0
                            && (p.calculated >= options.price_threshold
                                || (options.exempt_lifeforce_and_markers
                                    && THRESHOLD_EXEMPT_ITEMS.contains(&p.key.name.as_str()))))
                }),
        );
        surviving.retain(|p| !INVALID_ITEMS.contains(&p.key.name.as_str()));

        let mut by_name: HashMap<&str, Candidates<'_>> = HashMap::with_capacity(surviving.len());
        for entry in surviving {
            let candidate = Candidate {
                entry,
                key: &entry.key,
                unit_price: entry.calculated,
            };
            match by_name.entry(entry.key.name.as_str()) {
                Entry::Vacant(slot) => {
                    slot.insert(Candidates::One(candidate));
                }
                Entry::Occupied(mut slot) => slot.get_mut().push(candidate),
            }
        }
        for price in custom_prices {
            if let Some(candidates) = by_name.get_mut(price.key.name.as_str())
                && let Some(candidate) = candidates
                    .as_mut_slice()
                    .iter_mut()
                    .find(|c| *c.key == price.key)
            {
                candidate.unit_price = if price.value != 0.0 {
                    price.value
                } else {
                    candidate.entry.calculated
                };
            }
        }
        Self {
            by_name,
            total_price_threshold: options.total_price_threshold,
        }
    }

    /// The unit price of an item, `None` when it matches no price entry
    /// (legacy `pricingService.priceItem`): a Chaos Orb is always 1c; otherwise
    /// the item kind's rule must leave exactly one entry. A matched price of 0
    /// values the item at 0 (D2).
    pub fn unit_price(&self, item: &ItemDefinition) -> Option<f64> {
        if item.name == "Chaos Orb" {
            return Some(1.0);
        }
        self.matching_entry(item).map(|c| c.unit_price)
    }

    /// The price entry an item matches under the rules of [`Self::unit_price`],
    /// whose trend and poe.ninja page the item table shows (D3). A Chaos Orb
    /// matches its entry like any currency.
    pub fn matching_price(&self, item: &ItemDefinition) -> Option<&'a PriceEntry> {
        self.matching_entry(item).map(|c| c.entry)
    }

    fn named(&self, name: &str) -> impl Iterator<Item = &Candidate<'a>> {
        self.by_name
            .get(name)
            .map(Candidates::as_slice)
            .into_iter()
            .flatten()
    }

    fn matching_entry(&self, item: &ItemDefinition) -> Option<&Candidate<'a>> {
        let named = self.named(&item.name);
        if item.name.ends_with(" Wombgift") {
            return named
                .filter(|p| p.key.ilvl.is_some_and(|ilvl| ilvl <= item.ilvl))
                .max_by_key(|p| p.key.ilvl);
        }
        match item.frame_type {
            frame_type::NORMAL | frame_type::MAGIC | frame_type::RARE => {
                if item.name.contains(" Map") {
                    only(named.filter(|p| p.key.tier == Some(item.tier)))
                } else if item.beast {
                    only(self.named(&item.type_line))
                } else {
                    // e.g. fragments, scarabs
                    only(named)
                }
            }
            frame_type::UNIQUE => {
                let mut qual_match: Option<&Candidate<'a>> = None;
                let mut qual_count = 0usize;
                let mut q0_match: Option<&Candidate<'a>> = None;
                let mut q0_count = 0usize;
                for p in named.filter(|p| unique_matches(p.key, item)) {
                    if p.key.quality.is_none_or(|q| q == 0 || q == item.quality) {
                        qual_count += 1;
                        if qual_count == 1 {
                            qual_match = Some(p);
                        }
                    }
                    if p.key.quality == Some(0) {
                        q0_count += 1;
                        if q0_count == 1 {
                            q0_match = Some(p);
                        }
                    }
                }
                if qual_count == 1 {
                    qual_match
                } else if q0_count == 1 {
                    q0_match
                } else {
                    None
                }
            }
            frame_type::GEM => only(named.filter(|p| gem_matches(p.key, item))),
            frame_type::CURRENCY if item.name.contains(" Seed") => {
                // Seeds are told apart by tier (stored in `ilvl`): below 76 or not.
                let low = item.ilvl > 0 && item.ilvl < 76;
                only(named.filter(|p| match p.key.ilvl {
                    Some(ilvl) if ilvl > 0 => (ilvl < 76) == low,
                    _ => false,
                }))
            }
            frame_type::DIVINATION_CARD => only(named.filter(|p| p.key.icon.contains("Inventory"))),
            frame_type::PROPHECY => only(named.filter(|p| p.key.icon.contains("Prophecy"))),
            // The highest item level not above the ember's; the last such entry.
            frame_type::ALLFLAME_EMBER => named
                .filter(|p| p.key.ilvl.is_some_and(|ilvl| ilvl <= item.ilvl))
                .max_by_key(|p| p.key.ilvl),
            frame_type::VISCERA => {
                if item.variant.is_empty() {
                    return None;
                }
                only(named.filter(|p| {
                    p.key.frame_type == Some(frame_type::VISCERA)
                        && p.key.variant.as_deref() == Some(item.variant.as_str())
                }))
            }
            // Currency (D29: a Filled Coffin too), relics and everything else.
            _ => only(named),
        }
    }

    /// Merges each tab's items into one row per definition (F3), values the
    /// rows, drops rows whose total is 0 or below the total threshold (F5),
    /// and sums the rest into the tab value.
    pub fn price_tabs(&self, tabs: &[TabItems]) -> PricedSnapshot {
        let mut interned: HashMap<ItemDefinition, usize> = HashMap::new();
        let mut pool: Vec<(Option<f64>, Option<usize>)> = Vec::new();
        let mut next_def_index = 0usize;
        let tabs = tabs
            .iter()
            .map(|tab| {
                let mut merged: Vec<(usize, u32)> = Vec::new();
                let mut row_of: HashMap<usize, usize> = HashMap::new();
                for item in &tab.items {
                    let definition = ItemDefinition::from_item(item);
                    let id = match interned.get(&definition) {
                        Some(&id) => id,
                        None => {
                            let price = self.unit_price(&definition);
                            let id = pool.len();
                            pool.push((price, None));
                            interned.insert(definition, id);
                            id
                        }
                    };
                    let stack_size = item.stack_size.filter(|s| *s > 0).unwrap_or(1);
                    match row_of.get(&id) {
                        Some(&row) => merged[row].1 = merged[row].1.saturating_add(stack_size),
                        None => {
                            row_of.insert(id, merged.len());
                            merged.push((id, stack_size));
                        }
                    }
                }

                let mut rows = Vec::new();
                for (id, stack_size) in merged {
                    let (unit_price, out_idx) = &mut pool[id];
                    let Some(price) = *unit_price else { continue };
                    let total = f64::from(stack_size) * price;
                    if total <= 0.0 || total < self.total_price_threshold {
                        continue;
                    }
                    let definition = *out_idx.get_or_insert_with(|| {
                        let idx = next_def_index;
                        next_def_index += 1;
                        idx
                    });
                    rows.push(ItemRow {
                        definition,
                        stack_size,
                        total,
                    });
                }
                PricedTab {
                    id: tab.id.clone(),
                    value: rows.iter().map(|r| r.total).sum(),
                    rows,
                }
            })
            .collect();
        let mut slots: Vec<Option<PricedDefinition>> = vec![None; next_def_index];
        for (definition, id) in interned {
            if let (Some(price), Some(out_idx)) = pool[id] {
                slots[out_idx] = Some(PricedDefinition { definition, price });
            }
        }
        let definitions = slots.into_iter().flatten().collect();
        PricedSnapshot { definitions, tabs }
    }
}

/// The one item of `items`, if there is exactly one: an item is priced only
/// when exactly one price entry matches.
fn only<T>(mut items: impl Iterator<Item = T>) -> Option<T> {
    match (items.next(), items.next()) {
        (Some(item), None) => Some(item),
        _ => None,
    }
}

/// The generation of an older-map entry (variant `, Gen-<n>`).
fn map_generation(key: &PriceKey) -> Option<u64> {
    let generation = key.variant.as_deref()?.strip_prefix(", Gen-")?;
    if generation.is_empty() || !generation.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // Digits only, so parsing fails only on overflow: treat as newest.
    Some(generation.parse().unwrap_or(u64::MAX))
}

/// Uniques: links (any below 5 match each other), unique frame, and the
/// item's variant unless the entry has none.
fn unique_matches(key: &PriceKey, item: &ItemDefinition) -> bool {
    let links =
        (item.links < 5 && key.links.is_some_and(|l| l < 5)) || key.links == Some(item.links);
    let variant = key
        .variant
        .as_deref()
        .is_none_or(|v| v.is_empty() || v == item.variant);
    links && key.frame_type == Some(frame_type::UNIQUE) && variant
}

/// Gems: corruption, quality (any below 20 match each other) and level (any
/// below 20 match each other). Legacy's extra clause for Empower, Enhance and
/// Enlighten requires an equal level, which the last clause already accepts.
fn gem_matches(key: &PriceKey, item: &ItemDefinition) -> bool {
    let quality = (item.quality < 20 && key.quality.is_some_and(|q| q < 20))
        || key.quality == Some(item.quality);
    let level = (item.level < 20 && key.level.is_some_and(|l| l > 0 && l < 20))
        || key.level == Some(item.level);
    key.corrupted == Some(item.corrupted) && quality && level
}

/// Which tab items belong to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TabId {
    /// A GGG stash tab, its child tabs' items included (F3).
    Stash(String),
    /// The character's inventory and/or equipment (F3).
    Character,
}

/// A tab's items as fetched, before merging.
#[derive(Debug, Clone)]
pub struct TabItems<'i> {
    pub id: TabId,
    pub items: Vec<&'i Item>,
}

impl<'i> TabItems<'i> {
    /// The Character pseudo-tab: the inventory, then the equipment, each
    /// only when the Profile includes it.
    pub fn character(
        inventory: &'i [Item],
        equipment: &'i [Item],
        include_inventory: bool,
        include_equipment: bool,
    ) -> Self {
        let inventory = inventory.iter().filter(|_| include_inventory);
        let equipment = equipment.iter().filter(|_| include_equipment);
        Self {
            id: TabId::Character,
            items: inventory.chain(equipment).collect(),
        }
    }
}

/// A priced Snapshot: its tabs and the definitions their rows reference.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PricedSnapshot {
    /// Each referenced definition once, with the unit price used for the
    /// totals (custom price included), in order of first reference.
    pub definitions: Vec<PricedDefinition>,
    /// In input order.
    pub tabs: Vec<PricedTab>,
}

impl PricedSnapshot {
    /// The Snapshot's value: the sum of its tab values.
    pub fn total_value(&self) -> f64 {
        self.tabs.iter().map(|t| t.value).sum()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PricedDefinition {
    pub definition: ItemDefinition,
    /// Chaos.
    pub price: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PricedTab {
    pub id: TabId,
    /// Rows surviving the total threshold, in order of the items' first
    /// appearance.
    pub rows: Vec<ItemRow>,
    /// Chaos: the sum of the row totals.
    pub value: f64,
}

/// One merged item row: all of a tab's items with one definition.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemRow {
    /// Index into [`PricedSnapshot::definitions`].
    pub definition: usize,
    pub stack_size: u32,
    /// Chaos: `stack_size` times the definition's price.
    pub total: f64,
}

#[cfg(test)]
mod tests;
