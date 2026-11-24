//! poe.ninja overview responses mapped to price entries, as legacy maps them
//! (`exilence-ce-legacy/src/utils/price.utils.ts:44-170`,
//! `src/services/poe-ninja.service.ts:126-198`, `src/store/priceStore.ts:213-224`). Pure: the
//! caller fetches each overview of [`overviews`] and deserializes its body; a 404 means no
//! prices.

use std::collections::HashMap;

use serde::Deserialize;

use super::{PriceEntry, PriceKey, Sparkline};
use crate::item_definition::{frame_type, viscera_variant};

/// The endpoint family an overview type is requested from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverviewKind {
    /// `stash/current/currency/overview`, read as [`CurrencyOverview`].
    Currency,
    /// `stash/current/item/overview`, read as [`ItemOverview`].
    Item,
    /// `exchange/current/overview`, read as [`ExchangeOverview`].
    Exchange,
}

pub const CURRENCY_TYPES: [&str; 2] = ["Currency", "Fragment"];

pub const ITEM_TYPES: [&str; 20] = [
    "Viscera",
    "Incubator",
    "SkillGem",
    "UniqueMap",
    "Map",
    "BlightedMap",
    "BlightRavagedMap",
    "UniqueJewel",
    "UniqueFlask",
    "UniqueWeapon",
    "UniqueArmour",
    "UniqueRelic",
    "UniqueAccessory",
    "UniqueTincture",
    "Beast",
    "Vial",
    "Invitation",
    "Memory",
    "Corpse",
    "Wombgift",
];

/// Currency and Fragment again: the stash overview lacks some of them (e.g.
/// Exalted Orb); [`dedupe`] removes the overlap.
pub const EXCHANGE_TYPES: [&str; 19] = [
    "Currency",
    "Fragment",
    "DivinationCard",
    "Scarab",
    "Thread",
    "Essence",
    "Oil",
    "Fossil",
    "Resonator",
    "DeliriumOrb",
    "Artifact",
    "Tattoo",
    "Omen",
    "AllflameEmber",
    "Ducat",
    "EnshroudingCrystal",
    "Runegraft",
    "DjinnCoin",
    "Astrolabe",
];

/// Every overview legacy requests per price league, in the order its price
/// table concatenates their entries.
pub fn overviews() -> impl Iterator<Item = (OverviewKind, &'static str)> {
    let currency = CURRENCY_TYPES.map(|t| (OverviewKind::Currency, t));
    let item = ITEM_TYPES.map(|t| (OverviewKind::Item, t));
    let exchange = EXCHANGE_TYPES.map(|t| (OverviewKind::Exchange, t));
    currency.into_iter().chain(item).chain(exchange)
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrencyOverview {
    pub lines: Vec<CurrencyLine>,
    #[serde(default)]
    pub currency_details: Vec<CurrencyDetail>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrencyLine {
    pub currency_type_name: String,
    pub receive: Option<CurrencyReceive>,
    pub details_id: Option<String>,
    pub receive_spark_line: Option<Sparkline>,
    pub low_confidence_receive_spark_line: Option<Sparkline>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CurrencyReceive {
    pub value: f64,
    pub count: u64,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CurrencyDetail {
    pub name: String,
    pub icon: Option<String>,
}

impl CurrencyOverview {
    /// Entries carry only name, icon and frame type 5 as fields.
    /// `overview_type` is the requested type, e.g. `Fragment`.
    pub fn price_entries(self, overview_type: &str) -> impl Iterator<Item = PriceEntry> {
        let mut icons: HashMap<String, Option<String>> =
            HashMap::with_capacity(self.currency_details.len());
        for d in self.currency_details {
            icons.entry(d.name).or_insert(d.icon);
        }
        let slug = type_slug(overview_type);
        self.lines.into_iter().map(move |line| {
            let icon = icons
                .get(&line.currency_type_name)
                .and_then(|i| i.as_deref())
                .map(str::to_owned);
            let (calculated, count) = match line.receive {
                Some(r) => (r.value, r.count),
                None => (0.0, 0),
            };
            let sparkline = if count > 10 {
                line.receive_spark_line
            } else {
                line.low_confidence_receive_spark_line
            };
            PriceEntry {
                key: PriceKey {
                    name: line.currency_type_name,
                    icon: icon.unwrap_or_default(),
                    frame_type: Some(frame_type::CURRENCY),
                    ..PriceKey::default()
                },
                calculated,
                count,
                sparkline,
                details_page: details_page(&slug, line.details_id.as_deref()),
            }
        })
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ItemOverview {
    pub lines: Vec<ItemLine>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemLine {
    pub name: String,
    pub icon: Option<String>,
    pub chaos_value: Option<f64>,
    pub links: Option<u32>,
    pub variant: Option<String>,
    pub gem_level: Option<u32>,
    pub gem_quality: Option<u32>,
    pub item_class: Option<u32>,
    pub level_required: Option<u32>,
    pub corrupted: Option<bool>,
    pub map_tier: Option<u32>,
    pub count: Option<u64>,
    pub details_id: Option<String>,
    pub spark_line: Option<Sparkline>,
    pub sparkline: Option<Sparkline>,
    pub low_confidence_sparkline: Option<Sparkline>,
}

impl ItemOverview {
    /// `overview_type` is the requested type, e.g. `UniqueArmour`.
    pub fn price_entries(self, overview_type: &str) -> impl Iterator<Item = PriceEntry> {
        let is_viscera = overview_type == "Viscera";
        let slug = type_slug(overview_type);
        self.lines.into_iter().map(move |line| {
            let variant = if is_viscera {
                viscera_variant(line.variant.as_deref().unwrap_or_default())
            } else {
                line.variant.unwrap_or_default()
            };
            let elder = variant == "Elder";
            let shaper = variant == "Shaper";
            let count = line.count.unwrap_or(0);
            let tier = line.map_tier.or_else(|| name_tier(&line.name)).unwrap_or(0);
            let sparkline = if count > 10 {
                line.spark_line.or(line.sparkline)
            } else {
                line.low_confidence_sparkline
                    .or(line.spark_line)
                    .or(line.sparkline)
            };
            let page = details_page(&slug, line.details_id.as_deref());
            PriceEntry {
                key: PriceKey {
                    name: line.name,
                    icon: line.icon.unwrap_or_default(),
                    frame_type: Some(line.item_class.unwrap_or(0)),
                    links: Some(line.links.unwrap_or(0)),
                    quality: Some(line.gem_quality.unwrap_or(0)),
                    level: Some(line.gem_level.unwrap_or(0)),
                    ilvl: Some(line.level_required.unwrap_or(0)),
                    // poe.ninja no longer sends mapTier; the name carries it,
                    // e.g. "Map (Tier 16)".
                    tier: Some(tier),
                    corrupted: Some(line.corrupted.unwrap_or(false)),
                    elder: Some(elder),
                    shaper: Some(shaper),
                    variant: Some(variant),
                },
                calculated: line.chaos_value.unwrap_or(0.0),
                count,
                sparkline,
                details_page: page,
            }
        })
    }
}

/// The tier in a name ending in ` (Tier <n>)`.
fn name_tier(name: &str) -> Option<u32> {
    let digits = name.strip_suffix(')')?.rsplit_once(" (Tier ")?.1;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ExchangeOverview {
    pub lines: Vec<ExchangeLine>,
    #[serde(default)]
    pub items: Option<Vec<ExchangeItem>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExchangeLine {
    pub id: String,
    pub primary_value: Option<f64>,
    pub volume_primary_value: Option<f64>,
    pub sparkline: Option<Sparkline>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExchangeItem {
    pub id: String,
    pub name: String,
    pub image: Option<String>,
    pub details_id: Option<String>,
}

const POE_CDN: &str = "https://web.poecdn.com";

/// Every divination card shares this art; the exchange overview has none per
/// card, and matching tells cards by `Inventory` in the icon.
const DIVINATION_CARD_ICON: &str = "https://web.poecdn.com/gen/image/WzI1LDE0LHsiZiI6IjJESXRlbXMvRGl2aW5hdGlvbi9JbnZlbnRvcnlJY29uIiwidyI6MSwiaCI6MSwic2NhbGUiOjF9XQ/f34bf8cbb5/InventoryIcon.png";

impl ExchangeOverview {
    /// `overview_type` is the requested type, e.g. `DivinationCard`.
    pub fn price_entries(self, overview_type: &str) -> impl Iterator<Item = PriceEntry> {
        // Later items with the same id win, as legacy's lookup object does.
        let mut items: HashMap<String, ExchangeItem> = HashMap::new();
        for item in self.items.into_iter().flatten() {
            items.insert(item.id.clone(), item);
        }
        let is_div_card = overview_type == "DivinationCard";
        let slug = type_slug(overview_type);
        self.lines.into_iter().map(move |line| {
            let item = items.get(&line.id);
            let icon = match item.and_then(|i| i.image.as_deref()) {
                Some(image) if !image.is_empty() && !image.starts_with("http") => {
                    format!("{POE_CDN}{image}")
                }
                Some(image) if !image.is_empty() => image.to_owned(),
                _ if is_div_card => DIVINATION_CARD_ICON.to_owned(),
                _ => String::new(),
            };
            let calculated = line.primary_value.unwrap_or(0.0);
            // No observation count here: approximated from traded volume, but
            // never below the low-confidence cut, as these are completed trades.
            let count = if calculated > 0.0 {
                let traded = (line.volume_primary_value.unwrap_or(0.0) / calculated).round();
                (traded as u64).max(11)
            } else {
                0
            };
            let page = details_page(
                &slug,
                Some(
                    item.and_then(|i| i.details_id.as_deref())
                        .unwrap_or(&line.id),
                ),
            );
            let name = match item {
                Some(i) => i.name.clone(),
                None => line.id,
            };
            PriceEntry {
                key: PriceKey {
                    name,
                    icon,
                    frame_type: Some(frame_type::CURRENCY),
                    variant: Some(String::new()),
                    links: Some(0),
                    quality: Some(0),
                    level: Some(0),
                    ilvl: Some(0),
                    tier: Some(0),
                    corrupted: Some(false),
                    elder: Some(false),
                    shaper: Some(false),
                },
                calculated,
                count,
                sparkline: line.sparkline,
                details_page: page,
            }
        })
    }
}

/// The site serving the poe.ninja pages items link to (D3).
pub const PAGES_URL: &str = "https://poe.ninja";

/// A price entry's page below the league: `<type slug>/<details id>`;
/// `None` without a details id.
fn details_page(slug: &str, details_id: Option<&str>) -> Option<String> {
    details_id.map(|id| format!("{slug}/{id}"))
}

/// The poe.ninja page of a price entry of `price_league` (legacy
/// `detailsUrl`, `exilence-ce-legacy/src/utils/price.utils.ts:46-49`).
pub fn details_url(price_league: &str, details_page: &str) -> String {
    format!(
        "{PAGES_URL}/poe1/economy/{}/{details_page}",
        league_slug(price_league)
    )
}

/// poe.ninja's slug of a league: `Standard` → `standard`, `Hardcore
/// Mirage` → `miragehc` (legacy `getNinjaLeagueUrl`).
fn league_slug(league: &str) -> String {
    let squeezed = |s: &str| -> String {
        s.chars()
            .filter(|c| !c.is_whitespace())
            .flat_map(char::to_lowercase)
            .collect()
    };
    match league.strip_prefix("Hardcore ") {
        Some(rest) => format!("{}hc", squeezed(rest)),
        None => squeezed(league),
    }
}

/// poe.ninja's slug of an overview type: `DivinationCard` →
/// `divination-cards`, `UniqueAccessory` → `unique-accessories` (legacy
/// `getNinjaTypeUrl`).
fn type_slug(overview_type: &str) -> String {
    let mut slug = String::with_capacity(overview_type.len() + 4);
    let mut previous: Option<char> = None;
    for c in overview_type.chars() {
        if c.is_ascii_uppercase() && previous.is_some_and(|p| p.is_ascii_alphabetic()) {
            slug.push('-');
        }
        slug.extend(c.to_lowercase());
        previous = Some(c);
    }
    slug.push('s');
    slug.replace("prophecys", "prophecies")
        .replace("accessorys", "accessories")
        .replace("currencys", "currency")
        .replace("fragmentss", "fragments")
        .replace("visceras", "viscera")
}

/// Removes entries that several overviews share (legacy `dedupePrices`),
/// keeping the position of the first and the one backed by the most
/// listings. Absent fields count as 0, empty or false.
pub fn dedupe(entries: impl IntoIterator<Item = PriceEntry>) -> Vec<PriceEntry> {
    type DedupeKey<'a> = (&'a str, u32, u32, u32, u32, bool, &'a str, u32, u32);
    let entries: Vec<PriceEntry> = entries.into_iter().collect();
    // Per distinct key, in first-seen order: the index of the entry kept.
    let mut kept: Vec<usize> = Vec::new();
    let mut kept_of: HashMap<DedupeKey<'_>, usize> = HashMap::new();
    for (i, entry) in entries.iter().enumerate() {
        let k = &entry.key;
        let key = (
            k.name.as_str(),
            k.frame_type.unwrap_or(0),
            k.quality.unwrap_or(0),
            k.links.unwrap_or(0),
            k.level.unwrap_or(0),
            k.corrupted.unwrap_or(false),
            k.variant.as_deref().unwrap_or_default(),
            k.tier.unwrap_or(0),
            k.ilvl.unwrap_or(0),
        );
        match kept_of.get(&key) {
            Some(&slot) => {
                if entry.count > entries[kept[slot]].count {
                    kept[slot] = i;
                }
            }
            None => {
                kept_of.insert(key, kept.len());
                kept.push(i);
            }
        }
    }
    let mut entries: Vec<Option<PriceEntry>> = entries.into_iter().map(Some).collect();
    kept.into_iter().filter_map(|i| entries[i].take()).collect()
}
