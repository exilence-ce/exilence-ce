//! Commands: name, arguments and output. The arguments serialize to the
//! object Tauri maps onto the handler's parameters; every command fails with
//! [`crate::IpcError`].

use std::collections::BTreeMap;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::event::{Event, SliceName};
use crate::slices::{
    ColumnPreset, Currency, ExitAction, ItemColumn, ItemTableSort, Platform, Profile,
    ReleaseChannel, TimeSpan,
};

/// A command the UI can invoke.
pub trait Command {
    /// The handler's name in the core.
    const NAME: &'static str;
    type Args: Serialize + DeserializeOwned;
    type Output: Serialize + DeserializeOwned;
}

/// Arguments of a command without parameters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoArgs {}

/// `get_state(slices)`: the current value of each named slice, in order.
pub struct GetState;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetStateArgs {
    pub slices: Vec<SliceName>,
}

impl Command for GetState {
    const NAME: &'static str = "get_state";
    type Args = GetStateArgs;
    type Output = Vec<Event>;
}

/// `log(level, message)`: a line from the UI for the core's log (F20).
pub struct Log;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogArgs {
    pub level: LogLevel,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
}

impl Command for Log {
    const NAME: &'static str = "log";
    type Args = LogArgs;
    type Output = ();
}

/// `mark_notifications_read`: clears the unread count (F18).
pub struct MarkNotificationsRead;

impl Command for MarkNotificationsRead {
    const NAME: &'static str = "mark_notifications_read";
    type Args = NoArgs;
    type Output = ();
}

/// `update_settings(patch)`: stores the settings `patch` sets and pushes the
/// `settings` slice (F4, F5, F9–F11, F14, F15, F17, F20).
pub struct UpdateSettings;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct UpdateSettingsArgs {
    pub patch: SettingsPatch,
}

/// The settings to change; `None` keeps a setting. Each field is named as its
/// [`crate::slices::Settings`] field and its key in the `settings` table.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SettingsPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_action: Option<ExitAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nav_drawer_open: Option<bool>,
    /// Takes effect at the next start (F16).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_channel: Option<ReleaseChannel>,
    /// Set by `start_login` (F1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<Platform>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_snapshot: Option<bool>,
    /// 5..=1000 (F4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_snapshot_minutes: Option<u32>,
    /// Applies from the next Snapshot (F5).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub low_confidence_pricing: Option<bool>,
    /// Chaos, 0..=100; applies from the next Snapshot (F5).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price_threshold: Option<f64>,
    /// Chaos, 0..=5000; applies from the next Snapshot (F5).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_price_threshold: Option<f64>,
    /// Exempt Harvest lifeforce and Rogue's Markers from `price_threshold`;
    /// applies from the next Snapshot.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exempt_lifeforce_and_markers: Option<bool>,
    /// Percent, 50..=150 in steps of 10; applies at once (F17).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ui_scale: Option<u32>,
    /// Takes effect at the next start (D7).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hardware_acceleration: Option<bool>,
    /// Applies at once (F20).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debug_mode: Option<bool>,
    /// The currency values are shown in (F9).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub currency: Option<Currency>,
    /// The item table's layout (F11), remembered as the user changes it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_table_sort: Option<ItemTableSort>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_table_column_widths: Option<BTreeMap<ItemColumn, u32>>,
    /// One of [`crate::slices::ITEM_TABLE_PAGE_SIZES`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_table_page_size: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_table_presets: Option<Vec<ColumnPreset>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_table_active_preset: Option<String>,
    /// The net worth chart's time span; the `charts` slice follows (F10).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chart_time_span: Option<TimeSpan>,
    /// The net worth chart section is open (F10).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net_worth_chart_open: Option<bool>,
    /// The per-tab chart section is open (F10).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tab_chart_open: Option<bool>,
    /// The item table section is open (F10).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub items_open: Option<bool>,
}

impl Command for UpdateSettings {
    const NAME: &'static str = "update_settings";
    type Args = UpdateSettingsArgs;
    type Output = ();
}

/// `open_link(target)`: opens `target` in the system browser if its host is
/// allowlisted (F15).
pub struct OpenLink;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenLinkArgs {
    pub target: String,
}

impl Command for OpenLink {
    const NAME: &'static str = "open_link";
    type Args = OpenLinkArgs;
    type Output = ();
}

/// `open_issue_template(template)`: opens the new-issue page of one of the
/// client repository's issue templates in the system browser (F19, D11).
pub struct OpenIssueTemplate;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenIssueTemplateArgs {
    pub template: IssueTemplate,
}

/// An issue template of the client's repository; the bug report asks for
/// the Diagnostics bundle (F19, F20).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueTemplate {
    FeatureRequest,
    BugReport,
}

impl Command for OpenIssueTemplate {
    const NAME: &'static str = "open_issue_template";
    type Args = OpenIssueTemplateArgs;
    type Output = ();
}

/// `get_changelog`: What's New's markdown, the client repository's
/// `CHANGELOG.md`, unrendered (F19, D11).
pub struct GetChangelog;

impl Command for GetChangelog {
    const NAME: &'static str = "get_changelog";
    type Args = NoArgs;
    type Output = String;
}

/// `get_credits`: the Credits' markdown, the client repository's
/// `CREDITS.md`, unrendered (F19, D11).
pub struct GetCredits;

impl Command for GetCredits {
    const NAME: &'static str = "get_credits";
    type Args = NoArgs;
    type Output = String;
}

/// `complete_tour`: the first-run tour ended or was retaken (F19). Returns
/// whether What's New opens now: once, after the first tour.
pub struct CompleteTour;

impl Command for CompleteTour {
    const NAME: &'static str = "complete_tour";
    type Args = NoArgs;
    type Output = bool;
}

/// `start_login(platform)`: remembers the platform, starts a fresh login
/// attempt and opens GGG's consent page in the system browser (F1).
pub struct StartLogin;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartLoginArgs {
    pub platform: Platform,
}

impl Command for StartLogin {
    const NAME: &'static str = "start_login";
    type Args = StartLoginArgs;
    type Output = ();
}

/// `copy_login_link`: puts the pending attempt's authorization URL (a fresh
/// attempt's when none is pending) on the clipboard (F1).
pub struct CopyLoginLink;

impl Command for CopyLoginLink {
    const NAME: &'static str = "copy_login_link";
    type Args = NoArgs;
    type Output = ();
}

/// `sign_out`: forgets this Login and returns to login; the account's local
/// history stays (F1, D19). `Busy` while the operation slot is taken.
pub struct SignOut;

impl Command for SignOut {
    const NAME: &'static str = "sign_out";
    type Args = NoArgs;
    type Output = ();
}

/// `reset_data`: cancels a running Snapshot, signs out (best effort),
/// deletes all local data except logs and restarts (F17). Returns only if
/// that fails.
pub struct ResetData;

impl Command for ResetData {
    const NAME: &'static str = "reset_data";
    type Args = NoArgs;
    type Output = ();
}

/// `install_update`: installs the downloaded update and restarts into it
/// (F16).
pub struct InstallUpdate;

impl Command for InstallUpdate {
    const NAME: &'static str = "install_update";
    type Args = NoArgs;
    type Output = ();
}

/// `take_snapshot`: starts a Snapshot of the active Profile (F3) and returns
/// once it holds the operation slot; it then runs in the background, its
/// progress in the `status` slice. `NotReady` unless the session is ready,
/// the league has tabs and prices of the price league are loaded and not
/// updating; `RateLimited` during a cooldown (F7); `Busy` while the slot is
/// taken.
pub struct TakeSnapshot;

impl Command for TakeSnapshot {
    const NAME: &'static str = "take_snapshot";
    type Args = NoArgs;
    type Output = ();
}

/// `cancel_snapshot`: stops the running Snapshot without saving anything,
/// unless it is already being saved (F3).
pub struct CancelSnapshot;

impl Command for CancelSnapshot {
    const NAME: &'static str = "cancel_snapshot";
    type Args = NoArgs;
    type Output = ();
}

/// `reset_income`: sets the active Profile's income to 0 and starts its
/// window now; income and its reset last until the app quits (F8).
pub struct ResetIncome;

impl Command for ResetIncome {
    const NAME: &'static str = "reset_income";
    type Args = NoArgs;
    type Output = ();
}

/// `list_snapshots`: the active Profile's Snapshots, newest first, for the
/// removal dialog (F12). Empty without an active Profile.
pub struct ListSnapshots;

impl Command for ListSnapshots {
    const NAME: &'static str = "list_snapshots";
    type Args = NoArgs;
    type Output = Vec<SnapshotEntry>;
}

/// One Snapshot of the removal list (F12).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SnapshotEntry {
    pub id: String,
    /// Unix ms.
    pub created_at: i64,
    /// Chaos.
    pub total_value: f64,
    /// Whether its items are kept: false once retention dropped them, and
    /// for imported Snapshots, which never had them (F12, F21).
    pub item_detail: bool,
}

/// `remove_snapshots(snapshot_ids)`: removes the account's Snapshots with
/// these ids here, also when the backend cannot be reached; the backend's
/// copies go now or after a later session start (F12, D5, D26). `Busy`
/// while the operation slot is taken.
pub struct RemoveSnapshots;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoveSnapshotsArgs {
    pub snapshot_ids: Vec<String>,
}

impl Command for RemoveSnapshots {
    const NAME: &'static str = "remove_snapshots";
    type Args = RemoveSnapshotsArgs;
    type Output = ();
}

/// `export_csv(row_keys)`: asks where to save the CSV of the Latest rows
/// with these keys, in this order, and saves it there; nothing when the
/// user cancels (F11).
pub struct ExportCsv;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportCsvArgs {
    pub row_keys: Vec<String>,
}

impl Command for ExportCsv {
    const NAME: &'static str = "export_csv";
    type Args = ExportCsvArgs;
    type Output = ();
}

/// `export_diagnostics`: asks where to save the Diagnostics bundle and saves
/// it there; nothing when the user cancels (F20).
pub struct ExportDiagnostics;

impl Command for ExportDiagnostics {
    const NAME: &'static str = "export_diagnostics";
    type Args = NoArgs;
    type Output = ();
}

/// `open_log_folder`: opens the log directory in the file manager (F20).
pub struct OpenLogFolder;

impl Command for OpenLogFolder {
    const NAME: &'static str = "open_log_folder";
    type Args = NoArgs;
    type Output = ();
}

/// `open_overlay`: opens the Net worth overlay, or brings it to the front
/// when open (F14).
pub struct OpenOverlay;

impl Command for OpenOverlay {
    const NAME: &'static str = "open_overlay";
    type Args = NoArgs;
    type Output = ();
}

/// `close_overlay`: closes the Net worth overlay (F14).
pub struct CloseOverlay;

impl Command for CloseOverlay {
    const NAME: &'static str = "close_overlay";
    type Args = NoArgs;
    type Output = ();
}

/// `get_profile_options(league)`: what the Profile dialog offers for
/// `league`, the first league of the account's characters when `None` (F2).
/// Its GGG requests fail with `RateLimited` during a cooldown (F7). Also
/// loads the price leagues into the `profiles` slice when it has none (D21).
pub struct GetProfileOptions;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetProfileOptionsArgs {
    pub league: Option<String>,
}

impl Command for GetProfileOptions {
    const NAME: &'static str = "get_profile_options";
    type Args = GetProfileOptionsArgs;
    type Output = ProfileOptions;
}

/// The Profile dialog's choices for one league.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileOptions {
    /// The league the tabs and characters belong to; `None` without
    /// characters.
    pub league: Option<String>,
    /// The leagues of the account's characters, in character order.
    pub leagues: Vec<String>,
    /// The league's stash tabs in GGG's order, folder tabs replaced by their
    /// child tabs (F3).
    pub stash_tabs: Vec<StashTab>,
    /// Names of the league's characters.
    pub characters: Vec<String>,
}

/// A stash tab in the tab picker.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StashTab {
    pub id: String,
    pub name: String,
    /// GGG's position of the tab.
    pub index: u32,
    /// RGB hex without `#`, e.g. `7c5436`.
    pub colour: Option<String>,
}

/// A Profile before it exists: every field but the id, which the core mints.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewProfile {
    pub name: String,
    pub league_id: String,
    pub price_league_id: String,
    pub stash_tab_ids: Vec<String>,
    pub character_name: Option<String>,
    pub include_equipment: bool,
    pub include_inventory: bool,
}

impl NewProfile {
    /// The Profile with id `id`.
    pub fn with_id(self, id: String) -> Profile {
        Profile {
            id,
            name: self.name,
            league_id: self.league_id,
            price_league_id: self.price_league_id,
            stash_tab_ids: self.stash_tab_ids,
            character_name: self.character_name,
            include_equipment: self.include_equipment,
            include_inventory: self.include_inventory,
        }
    }
}

/// `create_profile(profile)`: creates the Profile on the backend, then here,
/// and makes it the active one (F2). A blank name fails with a validation
/// error on the name field.
pub struct CreateProfile;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateProfileArgs {
    pub profile: NewProfile,
}

impl Command for CreateProfile {
    const NAME: &'static str = "create_profile";
    type Args = CreateProfileArgs;
    type Output = ();
}

/// `update_profile(profile)`: replaces the Profile with the same id (F2).
pub struct UpdateProfile;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateProfileArgs {
    pub profile: Profile,
}

impl Command for UpdateProfile {
    const NAME: &'static str = "update_profile";
    type Args = UpdateProfileArgs;
    type Output = ();
}

/// `set_active_profile(profile_id)`: switches the active Profile (F2).
pub struct SetActiveProfile;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetActiveProfileArgs {
    pub profile_id: String,
}

impl Command for SetActiveProfile {
    const NAME: &'static str = "set_active_profile";
    type Args = SetActiveProfileArgs;
    type Output = ();
}

/// `remove_profile(profile_id)`: removes a Profile with its Snapshots,
/// switching to another Profile first when it is the active one. Fails when
/// it is the only Profile (F2).
pub struct RemoveProfile;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoveProfileArgs {
    pub profile_id: String,
}

impl Command for RemoveProfile {
    const NAME: &'static str = "remove_profile";
    type Args = RemoveProfileArgs;
    type Output = ();
}

/// `get_price_table(price_league)`: the poe.ninja prices of `price_league`
/// with their custom prices, loaded first when missing or older than 60
/// minutes; older map generations are left out (F6). Also loads the price
/// leagues into the `profiles` slice when it has none (D21).
pub struct GetPriceTable;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetPriceTableArgs {
    pub price_league: String,
}

impl Command for GetPriceTable {
    const NAME: &'static str = "get_price_table";
    type Args = GetPriceTableArgs;
    /// In poe.ninja's order.
    type Output = Vec<PriceTableRow>;
}

/// One poe.ninja price of the price table (F6).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PriceTableRow {
    pub key: PriceKey,
    /// poe.ninja's price in chaos.
    pub price: f64,
    /// The custom price in chaos, if set.
    pub custom_price: Option<f64>,
    /// "Price last 7 days" (D3); `None` shows N/A.
    pub trend: Option<crate::slices::PriceTrend>,
    /// The entry's poe.ninja page (D3).
    pub ninja_url: Option<String>,
}

/// The fields that identify a poe.ninja price entry (legacy `findPrice`),
/// as `domain::pricing::PriceKey`. `None` is a field the entry lacks.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PriceKey {
    pub name: String,
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

/// `set_custom_price(price_league, key, value)`: stores `value` chaos as the
/// custom price of the entry `key` in `price_league`, replacing any; a
/// negative or non-finite value fails with a validation error on the
/// custom-price field (F6).
pub struct SetCustomPrice;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SetCustomPriceArgs {
    pub price_league: String,
    pub key: PriceKey,
    pub value: f64,
}

impl Command for SetCustomPrice {
    const NAME: &'static str = "set_custom_price";
    type Args = SetCustomPriceArgs;
    type Output = ();
}

/// `remove_custom_price(price_league, key)`: removes the custom price of the
/// entry `key` in `price_league`, if any (F6).
pub struct RemoveCustomPrice;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoveCustomPriceArgs {
    pub price_league: String,
    pub key: PriceKey,
}

impl Command for RemoveCustomPrice {
    const NAME: &'static str = "remove_custom_price";
    type Args = RemoveCustomPriceArgs;
    type Output = ();
}

/// `scan_legacy_import`: Settings → Advanced → Import from Exilence CE
/// (F21). Scans the legacy folders now and sets the `legacy_import` offer;
/// an empty scan says no Exilence CE data was found, a failed one (Exilence
/// CE running, unreadable data) shows a toast. `Busy` while an import runs.
pub struct ScanLegacyImport;

impl Command for ScanLegacyImport {
    const NAME: &'static str = "scan_legacy_import";
    type Args = NoArgs;
    type Output = ();
}

/// `run_legacy_import(account_names)`: imports the ticked legacy accounts
/// into the signed-in account under the operation slot (F21): their Profiles
/// on the backend first, then everything locally in one transaction. A
/// failure changes nothing locally and shows a toast; the offer stays open.
pub struct RunLegacyImport;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunLegacyImportArgs {
    /// Names of the ticked legacy accounts, as in the offer.
    pub account_names: Vec<String>,
}

impl Command for RunLegacyImport {
    const NAME: &'static str = "run_legacy_import";
    type Args = RunLegacyImportArgs;
    type Output = ();
}

/// `skip_legacy_import`: records `declined` for the signed-in account and
/// closes the offer; an account without a Profile then gets `profile 1`
/// (F2, F21).
pub struct SkipLegacyImport;

impl Command for SkipLegacyImport {
    const NAME: &'static str = "skip_legacy_import";
    type Args = NoArgs;
    type Output = ();
}

/// `check_group_name(name)`: whether a Group of that name exists, without
/// regard to case; asked when the name field loses focus (F13).
pub struct CheckGroupName;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckGroupNameArgs {
    pub name: String,
}

impl Command for CheckGroupName {
    const NAME: &'static str = "check_group_name";
    type Args = CheckGroupNameArgs;
    type Output = bool;
}

/// A Group's name and its password, empty for none (F13).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupForm {
    pub name: String,
    pub password: String,
}

/// `create_group(group)`: creates the Group, leaving any previous one, and
/// opens its stream (F13). `Validation` on the name (empty, taken) or the
/// password (shorter than 6); `Busy` while the operation slot is taken.
pub struct CreateGroup;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateGroupArgs {
    pub group: GroupForm,
}

impl Command for CreateGroup {
    const NAME: &'static str = "create_group";
    type Args = CreateGroupArgs;
    type Output = ();
}

/// `join_group(group)`: joins the Group, leaving any previous one, and opens
/// its stream (F13). `Validation` on the name (not found) or the password
/// (wrong); `Busy` while the operation slot is taken.
pub struct JoinGroup;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JoinGroupArgs {
    pub group: GroupForm,
}

impl Command for JoinGroup {
    const NAME: &'static str = "join_group";
    type Args = JoinGroupArgs;
    type Output = ();
}

/// `leave_group`: leaves the Group and closes its stream (F13). `Busy` while
/// the operation slot is taken.
pub struct LeaveGroup;

impl Command for LeaveGroup {
    const NAME: &'static str = "leave_group";
    type Args = NoArgs;
    type Output = ();
}

/// `set_player_selected(account_id, selected)`: whether the player counts
/// in the Group-combined views (F13).
pub struct SetPlayerSelected;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetPlayerSelectedArgs {
    pub account_id: String,
    pub selected: bool,
}

impl Command for SetPlayerSelected {
    const NAME: &'static str = "set_player_selected";
    type Args = SetPlayerSelectedArgs;
    type Output = ();
}
