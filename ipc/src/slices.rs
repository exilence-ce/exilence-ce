//! The state slice types. Each is pushed whole whenever it changes. Times are
//! Unix milliseconds, UTC; values are chaos.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::message::Message;

/// `session`: who is signed in and the connection to the backend.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub login: LoginState,
    pub account_name: Option<String>,
    pub platform: Platform,
    /// Not connected to the backend. Only ever set in a Group (D25, D26).
    pub offline: bool,
    /// The same account connected elsewhere; offline until restarted (F13).
    pub superseded: bool,
    /// The session is ready (F3): the session start's GGG part has run,
    /// stash tab list and prices of the active Profile included.
    pub ready: bool,
    /// The active Profile's league has stash tabs, per the latest stash tab
    /// list (F3).
    pub league_has_tabs: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoginState {
    #[default]
    SignedOut,
    SigningIn,
    SignedIn,
}

/// The GGG platform of an account (F1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    #[default]
    Pc,
    Xbox,
    Playstation,
}

/// `status`: what the core is doing.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Status {
    /// The current status message, shown with a spinner.
    pub message: Option<Message>,
    pub snapshot_running: bool,
    /// The operation slot is taken (a Snapshot or another operation runs):
    /// Sign out is disabled (F1).
    pub operation_running: bool,
    /// Tab i of N of the running Snapshot, once its tabs are known.
    pub snapshot_progress: Option<SnapshotProgress>,
    /// The running Snapshot is being saved: Cancel is disabled (F3).
    pub snapshot_saving: bool,
    /// Debug mode is on, from the setting or from launch: the toolbar shows
    /// the debug marker (F20).
    pub debug_mode: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotProgress {
    /// 1-based.
    pub tab: u32,
    pub count: u32,
}

/// `rate_limit`: GGG rate limiting (F7).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RateLimit {
    /// End of the current cooldown, if any; the UI counts down from it.
    pub cooldown_until: Option<i64>,
    /// The allowance hint's numbers: the strictest window of the stash-tab
    /// policy from the latest headers. `None` before any are known.
    pub allowance: Option<Allowance>,
}

/// GGG allows `max` requests per `period` seconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Allowance {
    pub max: u32,
    pub period: u32,
}

/// `profiles`: the signed-in account's Profiles and the price leagues.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Profiles {
    /// In creation order.
    pub profiles: Vec<Profile>,
    pub active_profile_id: Option<String>,
    /// poe.ninja league names (D21).
    pub price_leagues: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    /// GGG league id.
    pub league_id: String,
    /// poe.ninja league name.
    pub price_league_id: String,
    /// GGG stash tab ids, in selection order.
    pub stash_tab_ids: Vec<String>,
    pub character_name: Option<String>,
    pub include_equipment: bool,
    pub include_inventory: bool,
}

/// `prices`: poe.ninja prices of the active price league (F5, F6, F9).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Prices {
    pub price_league_id: Option<String>,
    pub fetched_at: Option<i64>,
    pub availability: PriceAvailability,
    /// Chaos per Exalted Orb, the custom price if set.
    pub exalted_rate: Option<f64>,
    /// Chaos per Divine Orb, the custom price if set.
    pub divine_rate: Option<f64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PriceAvailability {
    #[default]
    NotLoaded,
    Loading,
    Available,
    /// poe.ninja has no prices for the price league.
    Unavailable,
}

/// `net_worth`: the net worth widgets (F8), solo or Group-combined.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NetWorth {
    pub total: f64,
    /// Change since the previous Snapshot.
    pub change: f64,
    /// Totals of the newest 10 Snapshots, oldest first.
    pub sparkline: Vec<f64>,
    /// Chaos per hour.
    pub income: f64,
    pub snapshot_count: u32,
    pub last_snapshot_at: Option<i64>,
    /// Combined over the selected Group players.
    pub group_combined: bool,
}

/// `items`: the item table (F11), solo or Group-merged.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Items {
    /// The latest Snapshot's tabs in its order; rows name them by index.
    pub tabs: Vec<ItemTab>,
    /// The latest Snapshot's items merged across its tabs, in the order
    /// they were first seen.
    pub rows: Vec<ItemRow>,
    /// Differences between the two compared Snapshots: `quantity` and
    /// `total` hold the changes, `tabs` is empty.
    pub comparison_rows: Vec<ItemRow>,
    pub group_combined: bool,
}

/// A tab of the latest Snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemTab {
    /// GGG stash tab id; empty for the Character pseudo-tab.
    pub id: String,
    /// The name as of that Snapshot.
    pub name: String,
}

/// One row of the item table with every F11 column's data.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ItemRow {
    /// Stable within one slice value; `export_csv` refers to rows by it.
    pub key: String,
    pub name: String,
    pub icon: String,
    /// GGG frame type, which gives the rarity colour.
    pub frame_type: u32,
    /// Item level; seed tier for seeds.
    pub ilvl: u32,
    /// Map tier, 0 otherwise.
    pub tier: u32,
    pub corrupted: bool,
    pub links: u32,
    pub quality: u32,
    pub level: u32,
    pub quantity: i64,
    /// Per unit.
    pub price: f64,
    /// Quantity × price.
    pub total: f64,
    /// The tabs holding the item, with the quantity in each.
    pub tabs: Vec<ItemRowTab>,
    /// "Price last 7 days" (D3); `None` shows blank.
    pub trend: Option<PriceTrend>,
    /// The item's poe.ninja page (D3).
    pub ninja_url: Option<String>,
}

impl ItemRow {
    /// The names of the tabs holding the item, comma-separated.
    pub fn tab_names(&self, tabs: &[ItemTab]) -> String {
        self.tabs
            .iter()
            .filter_map(|t| tabs.get(t.tab as usize))
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemRowTab {
    /// Index into [`Items::tabs`].
    pub tab: u32,
    pub quantity: i64,
}

/// poe.ninja's price trend over the last 7 days (D3).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PriceTrend {
    /// Change in percent against the first day, one point per day, oldest
    /// first; `None` for a day without data.
    pub points: Vec<Option<f64>>,
    /// Change in percent over the 7 days.
    pub change: f64,
}

/// `charts`: chart series for the chosen time span (F10).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Charts {
    pub time_span: TimeSpan,
    /// One series per selected player (D15).
    pub net_worth: Vec<Series>,
    /// One series per tab over the newest 50 Snapshots.
    pub tabs: Vec<Series>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Series {
    /// Player or tab name.
    pub name: String,
    /// Oldest first.
    pub points: Vec<Point>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub at: i64,
    pub value: f64,
}

/// `group`: the Group the account is in (F13).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Group {
    /// `None` outside a Group.
    pub name: Option<String>,
    /// The players, the account included, in joining order.
    pub members: Vec<GroupMember>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupMember {
    /// GGG account uuid; `set_player_selected` names the player by it.
    pub account_id: String,
    pub account_name: String,
    /// In the player selection: counts in the Group-combined views.
    pub selected: bool,
}

/// `notifications`: the notification list (F18).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Notifications {
    /// The newest 10, newest first.
    pub newest: Vec<Notification>,
    pub unread: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Notification {
    pub id: u64,
    pub at: i64,
    pub kind: NotificationKind,
    /// The title.
    pub message: Message,
    pub description: Option<Message>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationKind {
    Info,
    Success,
    Warning,
    Error,
}

/// The `toast` event: a transient message (F18). Not a slice.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Toast {
    pub kind: NotificationKind,
    /// The title.
    pub message: Message,
    pub description: Option<Message>,
}

/// `settings`: every persisted setting and remembered UI choice, with the
/// defaults of `client-schema.sql`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub platform: Platform,
    pub auto_snapshot: bool,
    /// 5..=1000.
    pub auto_snapshot_minutes: u32,
    pub low_confidence_pricing: bool,
    /// Chaos, 0..=100.
    pub price_threshold: f64,
    /// Chaos, 0..=5000.
    pub total_price_threshold: f64,
    pub currency: Currency,
    pub chart_time_span: TimeSpan,
    pub net_worth_chart_open: bool,
    pub tab_chart_open: bool,
    pub items_open: bool,
    pub item_table_sort: ItemTableSort,
    /// Column to width in pixels; a column not listed has its default.
    pub item_table_column_widths: BTreeMap<ItemColumn, u32>,
    /// One of [`ITEM_TABLE_PAGE_SIZES`].
    pub item_table_page_size: u32,
    pub item_table_presets: Vec<ColumnPreset>,
    /// The name of the preset whose hidden columns the table hides.
    pub item_table_active_preset: String,
    pub exit_action: ExitAction,
    pub nav_drawer_open: bool,
    pub release_channel: ReleaseChannel,
    /// Percent, 50..=150 in steps of 10.
    pub ui_scale: u32,
    pub hardware_acceleration: bool,
    pub tour_completed: bool,
    pub whats_new_shown: bool,
    pub debug_mode: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            platform: Platform::Pc,
            auto_snapshot: false,
            auto_snapshot_minutes: 20,
            low_confidence_pricing: false,
            price_threshold: 0.0,
            total_price_threshold: 0.0,
            currency: Currency::Chaos,
            chart_time_span: TimeSpan::All,
            net_worth_chart_open: false,
            tab_chart_open: false,
            items_open: true,
            item_table_sort: ItemTableSort::default(),
            item_table_column_widths: BTreeMap::new(),
            item_table_page_size: 25,
            item_table_presets: ColumnPreset::legacy(),
            item_table_active_preset: ColumnPreset::CUSTOM.to_owned(),
            exit_action: ExitAction::MinimizeToTray,
            nav_drawer_open: false,
            release_channel: ReleaseChannel::Latest,
            ui_scale: 100,
            hardware_acceleration: true,
            tour_completed: false,
            whats_new_shown: false,
            debug_mode: false,
        }
    }
}

/// The currency values are shown in (F9).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Currency {
    #[default]
    Chaos,
    Exalted,
    Divine,
}

/// The time span of the net worth chart (F10).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeSpan {
    Day,
    Week,
    Month,
    #[default]
    All,
}

/// The page sizes the item table offers (F11).
pub const ITEM_TABLE_PAGE_SIZES: [u32; 4] = [5, 10, 25, 50];

/// A column of the item table (F11), in display order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemColumn {
    Icon,
    Name,
    IlvlTier,
    Tabs,
    Corrupted,
    Links,
    Quality,
    Level,
    Quantity,
    /// "Price last 7 days" (D3).
    Trend,
    Price,
    Total,
    Cumulative,
}

impl ItemColumn {
    pub const ALL: [Self; 13] = [
        Self::Icon,
        Self::Name,
        Self::IlvlTier,
        Self::Tabs,
        Self::Corrupted,
        Self::Links,
        Self::Quality,
        Self::Level,
        Self::Quantity,
        Self::Trend,
        Self::Price,
        Self::Total,
        Self::Cumulative,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemTableSort {
    pub column: ItemColumn,
    pub descending: bool,
}

/// Legacy sorts by total, descending (`ItemTableContainer.tsx:144`).
impl Default for ItemTableSort {
    fn default() -> Self {
        Self {
            column: ItemColumn::Total,
            descending: true,
        }
    }
}

/// A named set of hidden item-table columns (F11).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnPreset {
    pub name: String,
    pub hidden_columns: Vec<ItemColumn>,
}

impl ColumnPreset {
    /// The preset active by default.
    pub const CUSTOM: &str = "custom";

    /// Legacy's presets (`exilence-ce-legacy/src/components/bulk-sell-column-presets-panel/supportedPresets.ts`),
    /// named by their locale key without the namespace.
    pub fn legacy() -> Vec<Self> {
        use ItemColumn::{Corrupted, IlvlTier, Level, Price, Quality, Quantity};
        let preset = |name: &str, hidden_columns: &[ItemColumn]| Self {
            name: name.to_owned(),
            hidden_columns: hidden_columns.to_vec(),
        };
        let mut presets = vec![
            preset(Self::CUSTOM, &[Quality, Level, Corrupted, IlvlTier]),
            preset("breachstones", &[Corrupted, Quality, IlvlTier]),
            preset("catalysts", &[Quantity, Price, IlvlTier]),
        ];
        presets.extend(
            [
                "delirium_orbs",
                "divination_cards",
                "essences",
                "fossils",
                "incubators",
                "prophecies",
                "scarabs",
                "sets",
            ]
            .map(|name| preset(name, &[IlvlTier])),
        );
        presets
    }
}

/// What closing the main window does (F15).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitAction {
    #[default]
    MinimizeToTray,
    Exit,
}

/// The release channel (F16): **Latest Stable** or **Beta**.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseChannel {
    #[default]
    Latest,
    Beta,
}

/// `update`: the updater (F16).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Update {
    /// The channel this run checks: the setting at launch. A changed setting
    /// takes effect at the next start.
    pub channel: ReleaseChannel,
    pub state: UpdateState,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum UpdateState {
    #[default]
    None,
    Available {
        version: String,
    },
    Downloading {
        version: String,
        /// 0..=100.
        percent: u8,
    },
    Ready {
        version: String,
    },
}

/// `legacy_import`: the legacy import offer (F21), shown by main.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LegacyImport {
    /// The pending offer; `None` once answered or without legacy data.
    pub offer: Option<LegacyImportOffer>,
    /// An import runs.
    pub running: bool,
}

/// The scan summary of a pending offer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyImportOffer {
    /// Every legacy account of both legacy folders, merged by name.
    pub accounts: Vec<LegacyAccount>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyAccount {
    /// Legacy account name, e.g. `Foo` or `Foo#1234`.
    pub name: String,
    /// Named like the signed-in account, with or without its `#digits`
    /// discriminator.
    pub preselected: bool,
    /// In legacy order.
    pub profiles: Vec<LegacyProfile>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyProfile {
    /// As legacy stored it; may be empty.
    pub name: String,
    pub snapshot_count: u32,
    /// The oldest and newest Snapshot; `None` without Snapshots.
    pub first_snapshot_at: Option<i64>,
    pub last_snapshot_at: Option<i64>,
}
