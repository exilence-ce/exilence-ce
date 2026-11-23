//! The parts of GGG's `Item` that pricing reads, as `Get Stash` and
//! `Get Character` send it (API 3.29: mods are `ItemMod` objects).

use serde::Deserialize;

/// One item of a stash tab, a character's inventory or its equipment.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    #[serde(default)]
    pub id: String,
    /// Empty for items without a name, e.g. currency.
    #[serde(default)]
    pub name: String,
    pub type_line: String,
    #[serde(default)]
    pub base_type: String,
    pub frame_type: u32,
    #[serde(default)]
    pub frame_type_id: Option<String>,
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub ilvl: u32,
    pub stack_size: Option<u32>,
    #[serde(default)]
    pub corrupted: bool,
    #[serde(default)]
    pub elder: bool,
    #[serde(default)]
    pub shaper: bool,
    pub descr_text: Option<String>,
    #[serde(default)]
    pub properties: Vec<ItemProperty>,
    #[serde(default)]
    pub sockets: Vec<ItemSocket>,
    #[serde(default)]
    pub implicit_mods: Vec<ItemMod>,
    #[serde(default)]
    pub explicit_mods: Vec<ItemMod>,
}

/// A named property such as `Quality` or `Map Tier`; each value is the
/// displayed text and its display style.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ItemProperty {
    pub name: String,
    #[serde(default)]
    pub values: Vec<(String, u32)>,
}

impl ItemProperty {
    /// The first value's text, as legacy reads it (`values[0][0]`).
    pub fn first_value(&self) -> Option<&str> {
        self.values.first().map(|(text, _)| text.as_str())
    }
}

/// A socket; sockets sharing a group are linked.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ItemSocket {
    pub group: u32,
}

/// An implicit or explicit mod.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ItemMod {
    pub description: String,
}
