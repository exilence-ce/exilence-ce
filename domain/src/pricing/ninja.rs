//! poe.ninja overview responses mapped to price entries, as legacy maps them
//! (`exilence-ce-legacy/src/utils/price.utils.ts:44-170`,
//! `src/services/poe-ninja.service.ts:126-198`, `src/store/priceStore.ts:213-224`). Pure: the
//! caller fetches each overview of [`overviews`] and deserializes its body; a 404 means no
//! prices.

use std::collections::HashMap;

use serde::Deserialize;

use super::{PriceEntry, PriceKey, Sparkline};
use crate::item_definition::frame_type;

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

pub const ITEM_TYPES: [&str; 17] = [
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
];

/// Currency and Fragment again: the stash overview lacks some of them (e.g.
/// Exalted Orb); [`dedupe`] removes the overlap.
pub const EXCHANGE_TYPES: [&str; 16] = [
    "Currency",
    "Fragment",
    "DivinationCard",
    "Scarab",
    "Essence",
    "Oil",
    "Fossil",
    "Resonator",
    "DeliriumOrb",
    "Artifact",
    "Tattoo",
    "Omen",
    "AllflameEmber",
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
    pub fn price_entries<'a>(
        &'a self,
        overview_type: &'a str,
    ) -> impl Iterator<Item = PriceEntry> + 'a {
        self.lines.iter().map(move |line| {
            let icon = self
                .currency_details
                .iter()
                .find(|d| d.name == line.currency_type_name)
                .and_then(|d| d.icon.clone());
            let count = line.receive.as_ref().map_or(0, |r| r.count);
            PriceEntry {
                key: PriceKey {
                    name: line.currency_type_name.clone(),
                    icon: icon.unwrap_or_default(),
                    frame_type: Some(frame_type::CURRENCY),
                    ..PriceKey::default()
                },
                calculated: line.receive.as_ref().map_or(0.0, |r| r.value),
                count,
                sparkline: if count > 10 {
                    line.receive_spark_line.clone()
                } else {
                    line.low_confidence_receive_spark_line.clone()
                },
                details_page: details_page(overview_type, line.details_id.as_deref()),
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
    pub fn price_entries<'a>(
        &'a self,
        overview_type: &'a str,
    ) -> impl Iterator<Item = PriceEntry> + 'a {
        self.lines.iter().map(move |line| {
            let variant = line.variant.clone().unwrap_or_default();
            let count = line.count.unwrap_or(0);
            let sparkline = line.spark_line.as_ref().or(line.sparkline.as_ref());
            PriceEntry {
                key: PriceKey {
                    name: line.name.clone(),
                    icon: line.icon.clone().unwrap_or_default(),
                    frame_type: Some(line.item_class.unwrap_or(0)),
                    links: Some(line.links.unwrap_or(0)),
                    quality: Some(line.gem_quality.unwrap_or(0)),
                    level: Some(line.gem_level.unwrap_or(0)),
                    ilvl: Some(line.level_required.unwrap_or(0)),
                    // poe.ninja no longer sends mapTier; the name carries it,
                    // e.g. "Map (Tier 16)".
                    tier: Some(line.map_tier.or_else(|| name_tier(&line.name)).unwrap_or(0)),
                    corrupted: Some(line.corrupted.unwrap_or(false)),
                    elder: Some(variant == "Elder"),
                    shaper: Some(variant == "Shaper"),
                    variant: Some(variant),
                },
                calculated: line.chaos_value.unwrap_or(0.0),
                count,
                sparkline: if count > 10 {
                    sparkline
                } else {
                    line.low_confidence_sparkline.as_ref().or(sparkline)
                }
                .cloned(),
                details_page: details_page(overview_type, line.details_id.as_deref()),
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
    pub fn price_entries<'a>(
        &'a self,
        overview_type: &'a str,
    ) -> impl Iterator<Item = PriceEntry> + 'a {
        // Later items with the same id win, as legacy's lookup object does.
        let items: HashMap<&str, &ExchangeItem> = self
            .items
            .iter()
            .flatten()
            .map(|item| (item.id.as_str(), item))
            .collect();
        self.lines.iter().map(move |line| {
            let item = items.get(line.id.as_str());
            let icon = match item.and_then(|i| i.image.as_deref()) {
                Some(image) if !image.is_empty() && !image.starts_with("http") => {
                    format!("{POE_CDN}{image}")
                }
                Some(image) if !image.is_empty() => image.to_owned(),
                _ if overview_type == "DivinationCard" => DIVINATION_CARD_ICON.to_owned(),
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
            PriceEntry {
                key: PriceKey {
                    name: item.map_or_else(|| line.id.clone(), |i| i.name.clone()),
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
                sparkline: line.sparkline.clone(),
                details_page: details_page(
                    overview_type,
                    Some(
                        item.and_then(|i| i.details_id.as_deref())
                            .unwrap_or(&line.id),
                    ),
                ),
            }
        })
    }
}

/// The site serving the poe.ninja pages items link to (D3).
pub const PAGES_URL: &str = "https://poe.ninja";

/// A price entry's page below the league: `<type slug>/<details id>`;
/// `None` without a details id.
fn details_page(overview_type: &str, details_id: Option<&str>) -> Option<String> {
    details_id.map(|id| format!("{}/{id}", type_slug(overview_type)))
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
}

/// Removes entries that several overviews share (legacy `dedupePrices`),
/// keeping the position of the first and the one backed by the most
/// listings. Absent fields count as 0, empty or false.
pub fn dedupe(entries: impl IntoIterator<Item = PriceEntry>) -> Vec<PriceEntry> {
    type DedupeKey = (String, u32, u32, u32, u32, bool, String, u32, u32);
    let mut index: HashMap<DedupeKey, usize> = HashMap::new();
    let mut deduped: Vec<PriceEntry> = Vec::new();
    for entry in entries {
        let k = &entry.key;
        let key = (
            k.name.clone(),
            k.frame_type.unwrap_or(0),
            k.quality.unwrap_or(0),
            k.links.unwrap_or(0),
            k.level.unwrap_or(0),
            k.corrupted.unwrap_or(false),
            k.variant.clone().unwrap_or_default(),
            k.tier.unwrap_or(0),
            k.ilvl.unwrap_or(0),
        );
        match index.get(&key) {
            Some(&i) => {
                if entry.count > deduped[i].count {
                    deduped[i] = entry;
                }
            }
            None => {
                index.insert(key, deduped.len());
                deduped.push(entry);
            }
        }
    }
    deduped
}
