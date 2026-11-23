//! The fixture stash: the Snapshot the pricing oracle prices, from the
//! captures in this directory. Included with `#[path]` by the pricing golden
//! test (`domain/tests/pricing_golden.rs`) and the poe.ninja drift check
//! (`tools/ninja-drift`), which pass this directory as `root`.

// Fixture loading: a missing or malformed capture panics the test.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::Path;

use domain::ggg::Item;
use domain::pricing::{TabId, TabItems};
use serde::Deserialize;
use serde::de::DeserializeOwned;

/// The league of the GGG and poe.ninja captures.
pub const LEAGUE: &str = "Standard";

/// The fixture `root`/`path`, parsed; panics if it cannot be read or parsed.
pub fn fixture<T: DeserializeOwned>(root: &Path, path: &str) -> T {
    let path = root.join(path);
    let json = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&json).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Profile {
    stash_tab_ids: Vec<String>,
    character: String,
    include_inventory: bool,
    include_equipment: bool,
}

#[derive(Deserialize)]
pub struct StashResponse {
    pub stash: Stash,
}

#[derive(Deserialize)]
pub struct Stash {
    pub id: String,
    pub items: Vec<Item>,
}

#[derive(Deserialize)]
struct CharacterResponse {
    character: Character,
}

#[derive(Deserialize)]
struct Character {
    #[serde(default)]
    inventory: Vec<Item>,
    #[serde(default)]
    equipment: Vec<Item>,
}

/// A selected tab with its child tabs' items folded in (F3); child tabs are
/// captured under `ggg/stash/<parent>/`.
fn stash_tab(root: &Path, id: &str) -> Stash {
    let mut tab = fixture::<StashResponse>(root, &format!("{LEAGUE}/ggg/stash/{id}.json")).stash;
    if let Ok(children) = fs::read_dir(root.join(format!("{LEAGUE}/ggg/stash/{id}"))) {
        let mut children: Vec<_> = children.map(|c| c.unwrap().file_name()).collect();
        children.sort();
        for child in children {
            let child = child.to_str().unwrap();
            let child: StashResponse = fixture(root, &format!("{LEAGUE}/ggg/stash/{id}/{child}"));
            tab.items.extend(child.stash.items);
        }
    }
    tab
}

/// The Snapshot the oracle prices: the Profile's selected tabs, the
/// hand-made tabs and the Character pseudo-tab.
pub struct Snapshot {
    stashes: Vec<Stash>,
    character: Character,
    profile: Profile,
}

impl Snapshot {
    pub fn load(root: &Path) -> Self {
        let profile: Profile = fixture(root, &format!("{LEAGUE}/profile.json"));
        let mut stashes: Vec<Stash> = profile
            .stash_tab_ids
            .iter()
            .map(|id| stash_tab(root, id))
            .collect();
        let mut hand_made: Vec<_> = fs::read_dir(root.join("hand-made/tabs"))
            .unwrap()
            .map(|f| f.unwrap().file_name())
            .collect();
        hand_made.sort();
        for file in hand_made {
            let path = format!("hand-made/tabs/{}", file.to_str().unwrap());
            stashes.push(fixture::<StashResponse>(root, &path).stash);
        }
        let character = fixture::<CharacterResponse>(
            root,
            &format!("{LEAGUE}/ggg/character/{}.json", profile.character),
        )
        .character;
        Self {
            stashes,
            character,
            profile,
        }
    }

    pub fn tabs(&self) -> Vec<TabItems<'_>> {
        let mut tabs: Vec<_> = self
            .stashes
            .iter()
            .map(|s| TabItems {
                id: TabId::Stash(s.id.clone()),
                items: s.items.iter().collect(),
            })
            .collect();
        tabs.push(TabItems::character(
            &self.character.inventory,
            &self.character.equipment,
            self.profile.include_inventory,
            self.profile.include_equipment,
        ));
        tabs
    }
}
