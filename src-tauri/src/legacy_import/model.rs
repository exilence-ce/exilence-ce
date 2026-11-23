//! The legacy values the Legacy import reads, as mobx-persist serialized them
//! (`exilence-ce-legacy/src/store/*Store.ts`, `src/store/domains/*.ts`). Every field is optional:
//! data written up to Exilence Next 1.1.8 / CE 1.2.2 lacks some, and unknown fields are ignored.
//! Only fields the import maps are declared ([legacy import](https://github.com/exilence-ce/migration/blob/main/docs/spec/legacy-import.md#mapping));
//! the GGG `token` is deliberately absent so it is never kept.

use serde::Deserialize;

/// `account` (`AccountStore`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountStore {
    pub accounts: Option<Vec<Account>>,
    /// `uuid` of the selected legacy account.
    pub active_account: Option<String>,
}

/// A legacy account, keyed by GGG account `name`; `uuid` is the legacy backend's id.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub uuid: Option<String>,
    pub name: Option<String>,
    pub account_leagues: Option<Vec<AccountLeague>>,
    pub profiles: Option<Vec<Profile>>,
}

/// The cached league data of an account, including its stash tab list.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountLeague {
    pub league_id: Option<String>,
    pub stashtabs: Option<Vec<StashTab>>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct StashTab {
    pub id: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub uuid: Option<String>,
    pub name: Option<String>,
    pub active_league_id: Option<String>,
    pub active_price_league_id: Option<String>,
    /// `''` or `'None'` when no character is selected.
    pub active_character_name: Option<String>,
    pub active_stash_tab_ids: Option<Vec<String>>,
    /// Newest first, at most 1000.
    pub snapshots: Option<Vec<Snapshot>>,
    pub active: Option<bool>,
    pub include_equipment: Option<bool>,
    pub include_inventory: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub uuid: Option<String>,
    /// ISO-8601 UTC.
    pub created: Option<String>,
    pub stash_tab_snapshots: Option<Vec<StashTabSnapshot>>,
}

/// One tab of a Snapshot; `stash_tab_id` `Character` is the character pseudo-tab.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StashTabSnapshot {
    pub stash_tab_id: Option<String>,
    /// Chaos.
    pub value: Option<f64>,
    /// Only kept for the newest Snapshots; read for tab names only.
    pub priced_items: Option<Vec<PricedItem>>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct PricedItem {
    pub tab: Option<Vec<StashTab>>,
}

/// `customPrice` (`CustomPriceStore`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomPriceStore {
    pub custom_league_prices: Option<Vec<LeaguePrices>>,
}

/// Custom prices of one price league.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LeaguePrices {
    pub league_id: Option<String>,
    pub prices: Option<Vec<CustomPrice>>,
}

/// A poe.ninja price entry; `custom_price` 0 or absent was never applied by legacy.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomPrice {
    pub name: Option<String>,
    pub icon: Option<String>,
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
    pub custom_price: Option<f64>,
}

/// `setting` (`SettingStore`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Setting {
    pub low_confidence_pricing: Option<bool>,
    pub price_threshold: Option<f64>,
    pub total_price_threshold: Option<f64>,
    pub auto_snapshotting: Option<bool>,
    /// Milliseconds.
    pub auto_snapshot_interval: Option<f64>,
    /// `chaos`, `exalt` or `divine`; absent in Exilence Next ≤ 1.1.8 data.
    pub currency: Option<String>,
    /// Exilence Next ≤ 1.1.8's predecessor of `currency`.
    pub show_price_in_exalt: Option<bool>,
    /// Percent.
    pub ui_scale: Option<f64>,
    /// `minimize-to-tray` or `exit`; `local-settings.json` wins.
    pub app_exit_action: Option<String>,
}

/// `uiState` (`UiStateStore`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiState {
    /// `1 day`, `1 week`, `1 month` or `All time`.
    pub chart_time_span: Option<String>,
    pub net_worth_chart_expanded: Option<bool>,
    pub tab_chart_expanded: Option<bool>,
    pub net_worth_items_expanded: Option<bool>,
    pub sidenav_open: Option<bool>,
}

/// `local-settings.json` in the folder root, written by legacy's main process.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalSettings {
    pub app_exit_action: Option<String>,
}

/// `migration` (`MigrationStore`); only `current` = 3 is importable data.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Migration {
    pub current: Option<u32>,
}
