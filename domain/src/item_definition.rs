//! What an item is, independent of stack size and price: the content of the
//! wire `ItemDefinition` without its id. Attributes come from the GGG item
//! (D1), derived as legacy `mapItemsToPricedItems` does
//! (`exilence-ce-legacy/src/utils/item.utils.ts:52-121`), except where D29–D31 fix it.
//!
//! `domain` does not depend on `wire` (it pulls in tonic and tokio), so the
//! app maps this to `wire::v1::ItemDefinition` and hashes it there.

use crate::ggg::{Item, ItemMod};

/// Fields mirror `wire::v1::ItemDefinition` (minus `id`); identical items
/// have equal definitions, so one row per definition merges them (F3).
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct ItemDefinition {
    /// Legacy-derived name: base type for non-unique maps, the name alone for
    /// uniques, name and type line otherwise.
    pub name: String,
    pub type_line: String,
    pub frame_type: u32,
    pub icon: String,
    /// Seed tier for seeds, item level otherwise.
    pub ilvl: u32,
    /// Map tier, 0 otherwise.
    pub tier: u32,
    pub links: u32,
    pub sockets: u32,
    pub quality: u32,
    pub level: u32,
    pub variant: String,
    pub corrupted: bool,
    pub elder: bool,
    pub shaper: bool,
    pub blighted: bool,
    pub beast: bool,
}

/// GGG frame types legacy pricing tells apart.
pub mod frame_type {
    pub const NORMAL: u32 = 0;
    pub const MAGIC: u32 = 1;
    pub const RARE: u32 = 2;
    pub const UNIQUE: u32 = 3;
    pub const GEM: u32 = 4;
    pub const CURRENCY: u32 = 5;
    pub const DIVINATION_CARD: u32 = 6;
    pub const PROPHECY: u32 = 8;
    pub const RELIC: u32 = 9;
    pub const ALLFLAME_EMBER: u32 = 11;
    pub const VISCERA: u32 = 15;
}

const BEAST_DESCRIPTION: &str = "Right-click to add this to your bestiary.";

impl ItemDefinition {
    /// Derives the definition of a GGG item.
    pub fn from_item(item: &Item) -> Self {
        let unique = item.frame_type == frame_type::UNIQUE;
        let viscera = item.frame_type == frame_type::VISCERA
            || item
                .frame_type_id
                .as_deref()
                .is_some_and(|id| id.eq_ignore_ascii_case("viscera"));
        let seed = item.frame_type == frame_type::CURRENCY && item.type_line.contains(" Seed");
        let mut tier: Option<u32> = None;
        let mut quality: Option<u32> = None;
        let mut level: Option<u32> = None;
        let mut seed_ilvl: Option<u32> = None;
        for prop in &item.properties {
            if tier.is_none() && prop.name == "Map Tier" {
                tier = Some(
                    prop.first_value()
                        .map_or(0, |text| text.trim().parse().unwrap_or(0)),
                );
            } else if quality.is_none() && prop.name == "Quality" {
                quality = Some(prop.first_value().map_or(0, parse_leading_int));
            } else if level.is_none() && prop.name == "Level" {
                level = Some(prop.first_value().map_or(0, parse_leading_int));
            } else if seed && seed_ilvl.is_none() && prop.name.contains("Spawns a Level") {
                seed_ilvl = Some(prop.first_value().map_or(0, parse_leading_int));
            }
        }
        let tier = tier.unwrap_or(0);
        let name = if viscera {
            let raw = if item.name.is_empty() {
                &item.type_line
            } else {
                &item.name
            };
            item_name(raw, None)
        } else if tier != 0 && !unique {
            item.base_type.clone()
        } else {
            // D29: a Filled Coffin keeps this name (its type line); legacy
            // named it after its first implicit mod.
            item_name(&item.name, (!unique).then_some(item.type_line.as_str()))
        };
        let ilvl = if seed {
            seed_ilvl.unwrap_or(0)
        } else {
            item.ilvl
        };
        let variant = if viscera {
            let joined = item
                .implicit_mods
                .iter()
                .map(|m| m.description.as_str())
                .collect::<Vec<_>>()
                .join(" / ");
            viscera_variant(&joined)
        } else {
            variant(item)
        };
        Self {
            ilvl,
            tier,
            links: links(item),
            sockets: item.sockets.len() as u32,
            quality: quality.unwrap_or(0),
            level: level.unwrap_or(0),
            variant,
            corrupted: item.corrupted,
            elder: item.elder || mentions(&item.implicit_mods, "Elder"),
            shaper: item.shaper || mentions(&item.implicit_mods, "Shaper"),
            blighted: item.type_line.contains("Blighted "),
            beast: item.descr_text.as_deref() == Some(BEAST_DESCRIPTION),
            name,
            type_line: item.type_line.clone(),
            frame_type: if viscera {
                frame_type::VISCERA
            } else {
                item.frame_type
            },
            icon: item.icon.clone(),
        }
    }
}

/// Normalizes a Viscera mod string to match poe.ninja's variant format:
/// numeric rolls and `(a-b)` ranges become `#`, ` / ` separators become a
/// space, and whitespace is collapsed.
pub fn viscera_variant(mods: &str) -> String {
    let mut normalized = String::with_capacity(mods.len());
    let mut rest = mods;
    while let Some(c) = rest.chars().next() {
        if c == '('
            && let Some(after) = strip_numeric_range(rest)
        {
            normalized.push('#');
            rest = after;
        } else if c.is_ascii_digit() {
            normalized.push('#');
            rest = strip_unsigned_number(rest);
        } else {
            normalized.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    let tokens: Vec<&str> = normalized.split_whitespace().collect();
    let mut out = String::with_capacity(normalized.len());
    for (i, &token) in tokens.iter().enumerate() {
        if token == "/" && i > 0 && i + 1 < tokens.len() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(token);
    }
    out
}

fn strip_unsigned_number(s: &str) -> &str {
    let int_end = s.bytes().take_while(u8::is_ascii_digit).count();
    let after_int = &s[int_end..];
    if let Some(after_dot) = after_int.strip_prefix('.') {
        let frac_end = after_dot.bytes().take_while(u8::is_ascii_digit).count();
        if frac_end > 0 {
            return &after_dot[frac_end..];
        }
    }
    after_int
}

fn strip_signed_number(s: &str) -> Option<&str> {
    let body = s.strip_prefix('-').unwrap_or(s);
    if !body.as_bytes().first().is_some_and(u8::is_ascii_digit) {
        return None;
    }
    Some(strip_unsigned_number(body))
}

fn strip_numeric_range(s: &str) -> Option<&str> {
    let rest = s.strip_prefix('(')?;
    let rest = strip_signed_number(rest)?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('-').or_else(|| rest.strip_prefix('–'))?;
    let rest = rest.trim_start();
    let rest = strip_signed_number(rest)?;
    rest.strip_prefix(')')
}

/// GGG's set markup, which legacy strips from the first place it appears.
const SET_MARKUP: &str = "<<set:MS>><<set:M>><<set:S>>";

/// Legacy `getItemName`: the name, followed by the type line when given,
/// without GGG's set markup.
fn item_name(name: &str, type_line: Option<&str>) -> String {
    let type_line = type_line.filter(|t| !t.is_empty());
    let mut joined = String::with_capacity(name.len() + type_line.map_or(0, |t| 1 + t.len()));
    joined.push_str(name);
    if let Some(type_line) = type_line {
        joined.push(' ');
        joined.push_str(type_line);
    }
    if let Some(pos) = joined.find(SET_MARKUP) {
        joined.replace_range(pos..pos + SET_MARKUP.len(), "");
    }
    let trimmed = joined.trim();
    if trimmed.len() == joined.len() {
        joined
    } else {
        trimmed.to_owned()
    }
}

/// A property's leading integer as JavaScript's `parseInt` reads it
/// (`+20%` is 20, `20 (Max)` is 20); 0 when absent or unreadable.
fn parse_leading_int(text: &str) -> u32 {
    let text = text.trim_start();
    let text = text.strip_prefix('+').unwrap_or(text);
    let end = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    text[..end].parse().unwrap_or(0)
}

/// The size of the largest linked socket group (legacy `getLinks`).
fn links(item: &Item) -> u32 {
    let mut counts: Vec<(u32, u32)> = Vec::new();
    for socket in &item.sockets {
        match counts.iter_mut().find(|(group, _)| *group == socket.group) {
            Some((_, count)) => *count += 1,
            None => counts.push((socket.group, 1)),
        }
    }
    counts
        .into_iter()
        .map(|(_, count)| count)
        .max()
        .unwrap_or(0)
}

fn mentions(mods: &[ItemMod], text: &str) -> bool {
    mods.iter().any(|m| m.description.contains(text))
}

/// D31: each damage type an Impresence's variant can be, with the text its
/// explicit mods name it by.
const IMPRESENCE_DAMAGE: [(&str, &str); 5] = [
    ("Lightning", "Lightning Damage"),
    ("Fire", "Fire Damage"),
    ("Cold", "Cold Damage"),
    ("Physical", "Physical Damage"),
    ("Chaos", "Chaos Damage"),
];

/// Legacy `getItemVariant`, fixed for D30 and D31. Legacy's abyssal-socket
/// variants compare the name joined with the type line against a bare
/// unique name, so they never apply and are left out.
fn variant(item: &Item) -> String {
    // D30: a watchstone's variant is its uses remaining, the mod's first word.
    if let Some(uses) = item
        .explicit_mods
        .iter()
        .find(|m| m.description.contains("uses remaining"))
    {
        return uses
            .description
            .split(' ')
            .next()
            .unwrap_or_default()
            .to_owned();
    }
    // D31: an Impresence's variant is the damage type its explicit mods name.
    if item.name.contains("Impresence") && item_name(&item.name, None) == "Impresence" {
        for (damage, needle) in IMPRESENCE_DAMAGE {
            if mentions(&item.explicit_mods, needle) {
                return damage.to_owned();
            }
        }
    }
    String::new()
}
