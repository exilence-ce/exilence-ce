//! How the legacy values map onto the local schema, without I/O: merging both legacy folders,
//! the dialog's scan summary with its preselection, and the rows to write
//! ([legacy import](https://github.com/exilence-ce/migration/blob/main/docs/spec/legacy-import.md#merge-rules)).

use std::collections::HashMap;
use std::time::UNIX_EPOCH;

use ipc::slices::{LegacyAccount, LegacyImportOffer, LegacyProfile};
use serde_json::{Value, json};

use super::folders::LegacyApp;
use super::model;
use super::reader::LegacyData;
use crate::settings::{AUTO_SNAPSHOT_MINUTES, UI_SCALE, UI_SCALE_STEP};

/// The only `migration.current` whose data is importable.
const MIGRATION_CURRENT: u32 = 3;
/// Legacy's id of the Character pseudo-tab.
const LEGACY_CHARACTER_TAB: &str = "Character";
/// The name of the Character pseudo-tab.
pub const CHARACTER_TAB: &str = "Character";
/// A tab whose name no legacy value holds.
pub const UNKNOWN_TAB: &str = "Unknown tab";

/// Everything the import can bring from the legacy folders, merged.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Scan {
    /// Legacy accounts merged by name, in legacy order: Exilence Next's folder first, the
    /// older one.
    pub accounts: Vec<Account>,
    /// `exilence-ce-app`'s first: the first of the same entry is written.
    pub custom_prices: Vec<CustomPrice>,
    /// `settings` key → value, `exilence-ce-app`'s winning over `exilence-next-app`'s.
    pub settings: Vec<(&'static str, Value)>,
}

impl Scan {
    /// Whether there is nothing to offer: no legacy account and no custom price.
    pub fn is_empty(&self) -> bool {
        self.accounts.is_empty() && self.custom_prices.is_empty()
    }
}

/// A legacy account, both folders merged.
#[derive(Debug, Clone, PartialEq)]
pub struct Account {
    pub name: String,
    /// In legacy order; one per uuid.
    pub profiles: Vec<Profile>,
    /// The active Profile per the folder whose Snapshots of this account are newest.
    pub active_profile_id: Option<String>,
    /// Tab id → name from the cached stash tab lists (`accountLeagues[].stashtabs`).
    pub tab_list: HashMap<String, String>,
}

impl Account {
    /// The newest Snapshot of any of its Profiles.
    pub fn newest(&self) -> Option<i64> {
        self.profiles.iter().filter_map(Profile::newest).max()
    }
}

/// A legacy Profile with its fields mapped, both folders merged.
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub id: String,
    /// As legacy stored it; may be empty ([`profile_name`]).
    pub name: String,
    pub league_id: Option<String>,
    pub price_league_id: Option<String>,
    pub stash_tab_ids: Vec<String>,
    /// `''` and `'None'` already `None`.
    pub character_name: Option<String>,
    pub include_equipment: bool,
    pub include_inventory: bool,
    pub active: bool,
    /// Newest first, one per uuid.
    pub snapshots: Vec<Snapshot>,
}

impl Profile {
    pub fn newest(&self) -> Option<i64> {
        self.snapshots.first().map(|s| s.created_at)
    }
}

/// A legacy Snapshot, totals only.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub id: String,
    /// Unix ms.
    pub created_at: i64,
    /// In legacy order.
    pub tabs: Vec<Tab>,
}

impl Snapshot {
    /// Σ of its tab values, the Character pseudo-tab included.
    pub fn total_value(&self) -> f64 {
        self.tabs.iter().map(|t| t.value).sum()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tab {
    /// `''` is the Character pseudo-tab.
    pub tab_id: String,
    /// Chaos.
    pub value: f64,
    /// The name its priced items carry (`pricedItems[].tab[0].name`); only the newest legacy
    /// Snapshots keep items.
    pub item_tab_name: Option<String>,
}

/// A `custom_prices` row.
#[derive(Debug, Clone, PartialEq)]
pub struct CustomPrice {
    pub price_league_id: String,
    pub name: String,
    pub icon: String,
    pub frame_type: Option<i64>,
    pub variant: Option<String>,
    pub links: Option<i64>,
    pub quality: Option<i64>,
    pub level: Option<i64>,
    pub ilvl: Option<i64>,
    pub tier: Option<i64>,
    pub corrupted: Option<bool>,
    pub elder: Option<bool>,
    pub shaper: Option<bool>,
    pub value: f64,
}

/// Merges what the legacy folders hold. Folders whose `migration.current` is not 3 hold
/// pre-1.0 data legacy itself wipes and are left out.
pub fn scan(folders: Vec<(LegacyApp, LegacyData)>) -> Scan {
    let mut folders: Vec<(LegacyApp, LegacyData)> = folders
        .into_iter()
        .filter(|(_, data)| {
            data.migration.as_ref().and_then(|m| m.current) == Some(MIGRATION_CURRENT)
        })
        .collect();
    // Per-device values: the newer app's first.
    folders.sort_by_key(|(app, _)| *app != LegacyApp::Ce);
    let mut scan = Scan::default();
    for (_, data) in &folders {
        scan.custom_prices.extend(custom_prices(data));
        for (key, value) in settings(data) {
            if !scan.settings.iter().any(|(k, _)| *k == key) {
                scan.settings.push((key, value));
            }
        }
    }
    // History: the older app's first, so Profiles keep their creation order.
    for (_, data) in folders.iter().rev() {
        let accounts = data
            .account
            .iter()
            .flat_map(|a| a.accounts.iter().flatten());
        for legacy in accounts {
            let Some(name) = legacy.name.clone().filter(|n| !n.is_empty()) else {
                continue;
            };
            let account = account(name, legacy);
            match scan.accounts.iter_mut().find(|a| a.name == account.name) {
                Some(merged) => merge_account(merged, account),
                None => scan.accounts.push(account),
            }
        }
    }
    scan
}

fn account(name: String, legacy: &model::Account) -> Account {
    let profiles: Vec<Profile> = legacy.profiles.iter().flatten().filter_map(profile).fold(
        Vec::new(),
        |mut profiles, profile| {
            add_profile(&mut profiles, profile);
            profiles
        },
    );
    let tab_list = legacy
        .account_leagues
        .iter()
        .flatten()
        .flat_map(|league| league.stashtabs.iter().flatten())
        .filter_map(|tab| Some((tab.id.clone()?, tab.name.clone()?)))
        .collect();
    Account {
        name,
        active_profile_id: profiles.iter().find(|p| p.active).map(|p| p.id.clone()),
        profiles,
        tab_list,
    }
}

/// Merges `newer` (the later folder's copy) into `merged`: Profiles by uuid; the active Profile
/// and cached tab names of the folder whose Snapshots are newer.
fn merge_account(merged: &mut Account, newer: Account) {
    if newer.newest() >= merged.newest() && newer.active_profile_id.is_some() {
        merged.active_profile_id = newer.active_profile_id.clone();
    }
    merged.tab_list.extend(newer.tab_list);
    for profile in newer.profiles {
        add_profile(&mut merged.profiles, profile);
    }
}

/// Adds `other` after `profiles`, or merges it into the one with its uuid: fields from the copy
/// whose newest Snapshot is newer (`other` on a tie), Snapshots unioned by uuid.
fn add_profile(profiles: &mut Vec<Profile>, mut other: Profile) {
    let Some(merged) = profiles.iter_mut().find(|p| p.id == other.id) else {
        profiles.push(other);
        return;
    };
    if other.newest() >= merged.newest() {
        std::mem::swap(merged, &mut other);
    }
    let mut seen: std::collections::HashSet<String> =
        merged.snapshots.iter().map(|s| s.id.clone()).collect();
    for snapshot in other.snapshots {
        if seen.insert(snapshot.id.clone()) {
            merged.snapshots.push(snapshot);
        }
    }
    sort_newest_first(&mut merged.snapshots);
}

fn sort_newest_first(snapshots: &mut [Snapshot]) {
    snapshots.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| b.id.cmp(&a.id))
    });
}

/// A legacy Profile; `None` without a uuid, which is its identity.
fn profile(legacy: &model::Profile) -> Option<Profile> {
    let id = legacy.uuid.clone().filter(|id| !id.is_empty())?;
    let mut snapshots: Vec<Snapshot> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for snapshot in legacy.snapshots.iter().flatten().filter_map(snapshot) {
        if seen.insert(snapshot.id.clone()) {
            snapshots.push(snapshot);
        }
    }
    sort_newest_first(&mut snapshots);
    Some(Profile {
        id,
        name: legacy.name.clone().unwrap_or_default(),
        league_id: legacy.active_league_id.clone().filter(|l| !l.is_empty()),
        price_league_id: legacy
            .active_price_league_id
            .clone()
            .filter(|l| !l.is_empty()),
        stash_tab_ids: legacy.active_stash_tab_ids.clone().unwrap_or_default(),
        character_name: legacy
            .active_character_name
            .clone()
            .filter(|c| !c.is_empty() && c != "None"),
        include_equipment: legacy.include_equipment.unwrap_or(false),
        include_inventory: legacy.include_inventory.unwrap_or(false),
        active: legacy.active.unwrap_or(false),
        snapshots,
    })
}

/// A legacy Snapshot; `None` without a uuid or a readable creation time.
fn snapshot(legacy: &model::Snapshot) -> Option<Snapshot> {
    let id = legacy.uuid.clone().filter(|id| !id.is_empty())?;
    let created_at = unix_ms(legacy.created.as_deref()?)?;
    let tabs = legacy
        .stash_tab_snapshots
        .iter()
        .flatten()
        .filter_map(|tab| {
            let id = tab.stash_tab_id.as_deref()?;
            Some(Tab {
                tab_id: if id == LEGACY_CHARACTER_TAB { "" } else { id }.to_owned(),
                value: tab.value.unwrap_or(0.0),
                item_tab_name: tab
                    .priced_items
                    .iter()
                    .flatten()
                    .find_map(|item| item.tab.as_ref()?.first()?.name.clone())
                    .filter(|name| !name.is_empty()),
            })
        })
        .collect();
    Some(Snapshot {
        id,
        created_at,
        tabs,
    })
}

/// Legacy's `created`, ISO-8601 UTC, as Unix ms.
fn unix_ms(iso: &str) -> Option<i64> {
    let at = humantime::parse_rfc3339(iso).ok()?;
    i64::try_from(at.duration_since(UNIX_EPOCH).ok()?.as_millis()).ok()
}

/// The custom prices legacy applied: those with a custom price above 0.
fn custom_prices(data: &LegacyData) -> Vec<CustomPrice> {
    let leagues = data
        .custom_price
        .iter()
        .flat_map(|c| c.custom_league_prices.iter().flatten());
    let mut rows = Vec::new();
    for league in leagues {
        let Some(price_league_id) = league.league_id.clone() else {
            continue;
        };
        for price in league.prices.iter().flatten() {
            let (Some(name), Some(icon), Some(value)) =
                (price.name.clone(), price.icon.clone(), price.custom_price)
            else {
                continue;
            };
            if !(value.is_finite() && value > 0.0) {
                continue;
            }
            rows.push(CustomPrice {
                price_league_id: price_league_id.clone(),
                name,
                icon,
                frame_type: price.frame_type,
                variant: price.variant.clone(),
                links: price.links,
                quality: price.quality,
                level: price.level,
                ilvl: price.ilvl,
                tier: price.tier,
                corrupted: price.corrupted,
                elder: price.elder,
                shaper: price.shaper,
                value,
            });
        }
    }
    rows
}

/// The settings of one legacy folder as `settings` key → value
/// ([mapping](https://github.com/exilence-ce/migration/blob/main/docs/spec/legacy-import.md#settings)).
fn settings(data: &LegacyData) -> Vec<(&'static str, Value)> {
    let mut entries = Vec::new();
    let mut add = |key: &'static str, value: Option<Value>| {
        if let Some(value) = value {
            entries.push((key, value));
        }
    };
    if let Some(s) = &data.setting {
        add(
            "low_confidence_pricing",
            s.low_confidence_pricing.map(Value::from),
        );
        add("price_threshold", s.price_threshold.map(Value::from));
        add(
            "total_price_threshold",
            s.total_price_threshold.map(Value::from),
        );
        add("auto_snapshot", s.auto_snapshotting.map(Value::from));
        add(
            "auto_snapshot_minutes",
            s.auto_snapshot_interval
                .filter(|ms| ms.is_finite())
                .map(|ms| json!(auto_snapshot_minutes(ms))),
        );
        let currency = match s.currency.as_deref() {
            Some("chaos") => Some("chaos"),
            Some("exalt") => Some("exalted"),
            Some("divine") => Some("divine"),
            Some(_) => None,
            None => s
                .show_price_in_exalt
                .map(|exalt| if exalt { "exalted" } else { "chaos" }),
        };
        add("currency", currency.map(Value::from));
        add(
            "ui_scale",
            s.ui_scale
                .filter(|s| s.is_finite())
                .map(|s| json!(ui_scale(s))),
        );
    }
    let exit_action = data
        .local_settings
        .as_ref()
        .and_then(|l| l.app_exit_action.as_deref())
        .or_else(|| data.setting.as_ref()?.app_exit_action.as_deref());
    add(
        "exit_action",
        match exit_action {
            Some("minimize-to-tray") => Some(json!("minimize_to_tray")),
            Some("exit") => Some(json!("exit")),
            _ => None,
        },
    );
    if let Some(ui) = &data.ui_state {
        let span = match ui.chart_time_span.as_deref() {
            Some("1 day") => Some("day"),
            Some("1 week") => Some("week"),
            Some("1 month") => Some("month"),
            Some("All time") => Some("all"),
            _ => None,
        };
        add("chart_time_span", span.map(Value::from));
        add(
            "net_worth_chart_open",
            ui.net_worth_chart_expanded.map(Value::from),
        );
        add("tab_chart_open", ui.tab_chart_expanded.map(Value::from));
        add("items_open", ui.net_worth_items_expanded.map(Value::from));
        add("nav_drawer_open", ui.sidenav_open.map(Value::from));
    }
    entries
}

/// Legacy's interval in ms as whole minutes in 5..=1000.
fn auto_snapshot_minutes(ms: f64) -> u32 {
    let (min, max) = (*AUTO_SNAPSHOT_MINUTES.start(), *AUTO_SNAPSHOT_MINUTES.end());
    (ms / 60_000.0)
        .round()
        .clamp(f64::from(min), f64::from(max)) as u32
}

/// Legacy's UI scale in percent, rounded to a step of 10 in 50..=150.
fn ui_scale(percent: f64) -> u32 {
    let step = f64::from(UI_SCALE_STEP);
    let (min, max) = (*UI_SCALE.start(), *UI_SCALE.end());
    ((percent / step).round() * step).clamp(f64::from(min), f64::from(max)) as u32
}

/// Whether the legacy account `legacy_name` is preselected for the signed-in account
/// `signed_in`: the same name, or the signed-in name without its `#digits` discriminator.
pub fn preselected(legacy_name: &str, signed_in: &str) -> bool {
    legacy_name == signed_in || without_discriminator(signed_in) == Some(legacy_name)
}

fn without_discriminator(name: &str) -> Option<&str> {
    let (base, digits) = name.rsplit_once('#')?;
    (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())).then_some(base)
}

/// The dialog's summary of `scan` for the signed-in account `signed_in`.
pub fn summary(scan: &Scan, signed_in: &str) -> LegacyImportOffer {
    LegacyImportOffer {
        accounts: scan
            .accounts
            .iter()
            .map(|account| LegacyAccount {
                name: account.name.clone(),
                preselected: preselected(&account.name, signed_in),
                profiles: account
                    .profiles
                    .iter()
                    .map(|p| LegacyProfile {
                        name: p.name.clone(),
                        snapshot_count: u32::try_from(p.snapshots.len()).unwrap_or(u32::MAX),
                        first_snapshot_at: p.snapshots.last().map(|s| s.created_at),
                        last_snapshot_at: p.newest(),
                    })
                    .collect(),
            })
            .collect(),
    }
}

/// What the ticked legacy accounts bring.
#[derive(Debug, Clone, PartialEq)]
pub struct Selection {
    /// Merged by uuid across the ticked accounts, in legacy order.
    pub profiles: Vec<Profile>,
    /// The legacy active Profile of the ticked account with the newest Snapshot.
    pub active_profile_id: Option<String>,
    /// Tab id → name ([`tab_name`]).
    tab_names: HashMap<String, String>,
}

impl Selection {
    /// The name of tab `tab_id`: from the newest Snapshot whose items name it, else the cached
    /// tab list, else "Unknown tab"; the Character pseudo-tab is "Character".
    pub fn tab_name(&self, tab_id: &str) -> &str {
        if tab_id.is_empty() {
            return CHARACTER_TAB;
        }
        self.tab_names
            .get(tab_id)
            .map_or(UNKNOWN_TAB, String::as_str)
    }
}

/// The legacy accounts of `scan` named in `names`, in the scan's order.
pub fn select(scan: &Scan, names: &[String]) -> Selection {
    let accounts: Vec<&Account> = scan
        .accounts
        .iter()
        .filter(|a| names.contains(&a.name))
        .collect();
    let mut profiles = Vec::new();
    for profile in accounts.iter().flat_map(|a| &a.profiles) {
        add_profile(&mut profiles, profile.clone());
    }
    // The newest first; without Snapshots, the first ticked account with an active Profile.
    let active_profile_id = accounts
        .iter()
        .filter(|a| a.active_profile_id.is_some())
        .min_by_key(|a| std::cmp::Reverse(a.newest()))
        .and_then(|a| a.active_profile_id.clone());
    let mut tab_names: HashMap<String, String> = HashMap::new();
    for account in &accounts {
        for (id, name) in &account.tab_list {
            tab_names.entry(id.clone()).or_insert_with(|| name.clone());
        }
    }
    let mut snapshots: Vec<&Snapshot> = profiles.iter().flat_map(|p| &p.snapshots).collect();
    snapshots.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| b.id.cmp(&a.id))
    });
    let mut named: HashMap<&str, &str> = HashMap::new();
    for tab in snapshots.iter().flat_map(|s| &s.tabs) {
        if let Some(name) = &tab.item_tab_name {
            named.entry(tab.tab_id.as_str()).or_insert(name.as_str());
        }
    }
    for (id, name) in named {
        tab_names.insert(id.to_owned(), name.to_owned());
    }
    Selection {
        profiles,
        active_profile_id,
        tab_names,
    }
}

/// An imported Profile's name: legacy's, trimmed; an empty one becomes `profile N`, N its
/// 1-based position.
pub fn profile_name(name: &str, position: i64) -> String {
    let name = name.trim();
    if name.is_empty() {
        format!("profile {}", position + 1)
    } else {
        name.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tab(id: &str, value: f64, name: Option<&str>) -> Tab {
        Tab {
            tab_id: id.to_owned(),
            value,
            item_tab_name: name.map(str::to_owned),
        }
    }

    fn snap(id: &str, created_at: i64, tabs: Vec<Tab>) -> Snapshot {
        Snapshot {
            id: id.to_owned(),
            created_at,
            tabs,
        }
    }

    fn prof(id: &str, name: &str, snapshots: Vec<Snapshot>) -> Profile {
        Profile {
            id: id.to_owned(),
            name: name.to_owned(),
            league_id: Some("Settlers".to_owned()),
            price_league_id: Some("Settlers".to_owned()),
            stash_tab_ids: Vec::new(),
            character_name: None,
            include_equipment: false,
            include_inventory: false,
            active: false,
            snapshots,
        }
    }

    #[test]
    fn f21_preselects_the_signed_in_name_with_or_without_its_discriminator() {
        assert!(preselected("Foo#1234", "Foo#1234"));
        assert!(preselected("Foo", "Foo#1234"));
        assert!(preselected("Foo", "Foo"));
        assert!(!preselected("Foo#1234", "Foo"));
        assert!(!preselected("Foo#12", "Foo#1234"));
        assert!(!preselected("Bar", "Foo#1234"));
        assert!(preselected("Foo#", "Foo#"), "the same name");
        assert!(!preselected("Foo", "Foo#"), "no digits, no discriminator");
    }

    #[test]
    fn f21_profiles_merge_by_uuid_with_fields_of_the_newer_copy() {
        let mut profiles = vec![prof("p", "old", vec![snap("a", 1, Vec::new())])];
        add_profile(
            &mut profiles,
            prof(
                "p",
                "new",
                vec![snap("b", 3, Vec::new()), snap("a", 1, Vec::new())],
            ),
        );
        add_profile(
            &mut profiles,
            prof("p", "stale", vec![snap("c", 2, Vec::new())]),
        );
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].name, "new");
        let ids: Vec<&str> = profiles[0]
            .snapshots
            .iter()
            .map(|s| s.id.as_str())
            .collect();
        assert_eq!(ids, ["b", "c", "a"], "unioned, newest first");
    }

    #[test]
    fn f21_tab_names_come_from_the_newest_snapshot_then_the_tab_list() {
        let account = Account {
            name: "Foo".to_owned(),
            profiles: vec![prof(
                "p",
                "",
                vec![
                    snap("new", 3, vec![tab("t1", 1.0, None), tab("", 2.0, None)]),
                    snap("mid", 2, vec![tab("t1", 1.0, Some("Renamed"))]),
                    snap("old", 1, vec![tab("t1", 1.0, Some("Original"))]),
                ],
            )],
            active_profile_id: None,
            tab_list: HashMap::from([
                ("t1".to_owned(), "Listed".to_owned()),
                ("t2".to_owned(), "Only listed".to_owned()),
            ]),
        };
        let scan = Scan {
            accounts: vec![account],
            ..Scan::default()
        };
        let selection = select(&scan, &["Foo".to_owned()]);
        assert_eq!(selection.tab_name("t1"), "Renamed");
        assert_eq!(selection.tab_name("t2"), "Only listed");
        assert_eq!(selection.tab_name("t3"), UNKNOWN_TAB);
        assert_eq!(selection.tab_name(""), CHARACTER_TAB);
        assert_eq!(scan.accounts[0].profiles[0].snapshots[0].total_value(), 3.0);
    }

    #[test]
    fn f21_the_active_profile_is_that_of_the_ticked_account_with_the_newest_snapshot() {
        let account = |name: &str, profile: &str, newest: i64| Account {
            name: name.to_owned(),
            profiles: vec![prof(profile, "", vec![snap(profile, newest, Vec::new())])],
            active_profile_id: Some(profile.to_owned()),
            tab_list: HashMap::new(),
        };
        let scan = Scan {
            accounts: vec![account("Foo", "old", 1), account("Foo#1", "new", 2)],
            ..Scan::default()
        };
        let both = select(&scan, &["Foo".to_owned(), "Foo#1".to_owned()]);
        assert_eq!(both.active_profile_id.as_deref(), Some("new"));
        let one = select(&scan, &["Foo".to_owned()]);
        assert_eq!(one.active_profile_id.as_deref(), Some("old"));
        assert!(select(&scan, &[]).profiles.is_empty());
    }

    #[test]
    fn f21_settings_convert_to_their_ng_values() {
        assert_eq!(auto_snapshot_minutes(120_000.0), 5);
        assert_eq!(auto_snapshot_minutes(1_800_000.0), 30);
        assert_eq!(auto_snapshot_minutes(1e12), 1000);
        assert_eq!(ui_scale(87.0), 90);
        assert_eq!(ui_scale(104.0), 100);
        assert_eq!(ui_scale(10.0), 50);
        assert_eq!(ui_scale(300.0), 150);
        let data = LegacyData {
            setting: Some(model::Setting {
                low_confidence_pricing: None,
                price_threshold: None,
                total_price_threshold: None,
                auto_snapshotting: None,
                auto_snapshot_interval: None,
                currency: None,
                show_price_in_exalt: Some(true),
                ui_scale: None,
                app_exit_action: Some("minimize-to-tray".to_owned()),
            }),
            ..LegacyData::default()
        };
        assert_eq!(
            settings(&data),
            [
                ("currency", json!("exalted")),
                ("exit_action", json!("minimize_to_tray"))
            ]
        );
    }

    #[test]
    fn f21_empty_profile_names_become_profile_n() {
        assert_eq!(profile_name("", 1), "profile 2");
        assert_eq!(profile_name("  ", 0), "profile 1");
        assert_eq!(profile_name(" Main ", 4), "Main");
    }
}
