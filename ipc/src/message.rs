//! Message keys with their arguments: the only form in which user-facing text
//! crosses IPC (D10). [`crate::locale`] turns them into English.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Declares [`MessageKey`]: each variant with the id of its message in the
/// locale resources, which is also its serde form.
macro_rules! message_keys {
    ($($(#[$doc:meta])* $variant:ident => $id:literal,)*) => {
        /// A message in the locale resources (`ipc/locales/en.ftl`).
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        pub enum MessageKey {
            $($(#[$doc])* #[serde(rename = $id)] $variant,)*
        }

        impl MessageKey {
            /// Every key, in declaration order.
            pub const ALL: &[MessageKey] = &[$(MessageKey::$variant,)*];

            /// The message id in the locale resources.
            pub const fn id(self) -> &'static str {
                match self {
                    $(MessageKey::$variant => $id,)*
                }
            }
        }
    };
}

message_keys! {
    /// The window title.
    AppTitle => "app-title",
    /// Unread notifications; `count`.
    NotificationsUnread => "notifications-unread",
    /// Snapshot progress; `tab`, `count`.
    StatusSnapshotProgress => "status-snapshot-progress",
    /// The running Snapshot refreshes the stash tab list (F3).
    StatusRefreshingStashTabs => "status-refreshing-stash-tabs",
    /// The running Snapshot is being saved (F3).
    StatusSavingSnapshot => "status-saving-snapshot",
    /// The allowance hint while a Snapshot runs; `max` requests per `period`
    /// seconds (F7).
    SnapshotAllowance => "snapshot-allowance",
    /// The allowance hint before GGG's numbers are known (F7).
    SnapshotAllowanceUnknown => "snapshot-allowance-unknown",
    /// The toolbar's Snapshot and Cancel buttons (F3).
    SnapshotTake => "snapshot-take",
    SnapshotCancel => "snapshot-cancel",
    /// Notification: a Snapshot was saved (F3, F18).
    NotificationSnapshotSucceeded => "notification-snapshot-succeeded",
    /// Notification and toast: a Snapshot failed (F3, F18); the description
    /// says why.
    NotificationSnapshotFailed => "notification-snapshot-failed",
    /// None of the Profile's stash tabs is in the league's tab list (F3).
    ErrorNoStashTabsSelected => "error-no-stash-tabs-selected",
    /// [`crate::IpcError::NotReady`].
    ErrorNotReady => "error-not-ready",
    /// The Net worth widgets (F8): total with its change, Snapshot count with
    /// the time since the last Snapshot.
    NetWorthTotal => "net-worth-total",
    NetWorthChangeTooltip => "net-worth-change-tooltip",
    NetWorthSnapshotCount => "net-worth-snapshot-count",
    NetWorthTimeSinceTooltip => "net-worth-time-since-tooltip",
    /// The income widget (F8): title, the window note and its tooltip, the
    /// per-hour suffix and the Reset button.
    NetWorthIncome => "net-worth-income",
    NetWorthIncomeBasedOn => "net-worth-income-based-on",
    NetWorthIncomeTooltip => "net-worth-income-tooltip",
    NetWorthHourSuffix => "net-worth-hour-suffix",
    NetWorthIncomeReset => "net-worth-income-reset",
    /// The toolbar's overlay button and the overlay's close button (F14).
    OverlayOpen => "overlay-open",
    OverlayClose => "overlay-close",
    /// The short currency labels (F9).
    CurrencyChaos => "currency-chaos",
    CurrencyExalted => "currency-exalted",
    CurrencyDivine => "currency-divine",
    /// The currency switch's tooltip: the rates, `exalted` and `divine`
    /// chaos, and the hint (F9).
    CurrencySwitchRates => "currency-switch-rates",
    CurrencySwitchHint => "currency-switch-hint",
    /// The chart sections' titles (F10).
    ChartNetWorth => "chart-net-worth",
    ChartTabs => "chart-tabs",
    /// The net worth chart's time spans (F10).
    ChartTimeSpanDay => "chart-time-span-day",
    ChartTimeSpanWeek => "chart-time-span-week",
    ChartTimeSpanMonth => "chart-time-span-month",
    ChartTimeSpanAll => "chart-time-span-all",
    /// Restores a zoomed chart's full range (F10).
    ChartResetZoom => "chart-reset-zoom",
    /// The toolbar's remove-Snapshots button and the dialog's remove action
    /// (F12).
    SnapshotsRemove => "snapshots-remove",
    /// The remove-Snapshots dialog's title (F12).
    SnapshotsRemoveTitle => "snapshots-remove-title",
    /// One entry of the removal list; `date` (`YYYY-MM-DD HH:mm`) and
    /// `value` (chaos, 2 decimals) (F12).
    SnapshotsRemoveEntry => "snapshots-remove-entry",
    /// The mark of an entry without item detail (F12).
    SnapshotsItemsMissing => "snapshots-items-missing",
    /// Select all, in the remove-Snapshots dialog (F12).
    SelectAll => "select-all",
    /// Toast: removing Snapshots failed (F12); the description says why.
    NotificationSnapshotsRemoveFailed => "notification-snapshots-remove-failed",
    /// The toolbar's warning icon title during a rate-limit cooldown (F7).
    RateLimitExceeded => "rate-limit-exceeded",
    /// The toolbar's rate-limit countdown; `seconds` left (F7).
    RateLimitWait => "rate-limit-wait",
    /// The error state after a UI panic, pointing to Export diagnostics (F20).
    UiCrashed => "ui-crashed",
    /// The notification bell and its list (F18).
    Notifications => "notifications",
    /// The notification list without notifications.
    NotificationsEmpty => "notifications-empty",
    /// Less than a minute ago.
    TimeJustNow => "time-just-now",
    /// `count` minutes ago.
    TimeMinutesAgo => "time-minutes-ago",
    /// `count` hours ago.
    TimeHoursAgo => "time-hours-ago",
    /// `count` days ago.
    TimeDaysAgo => "time-days-ago",
    /// The platform choice on the login screen (F1).
    LoginPlatform => "login-platform",
    PlatformPc => "platform-pc",
    PlatformXbox => "platform-xbox",
    PlatformPlaystation => "platform-playstation",
    /// The Authorize button (F1).
    LoginAuthorize => "login-authorize",
    /// The hint above the copy-link control (F1).
    LoginCopyLinkHint => "login-copy-link-hint",
    /// The copy-link control (F1).
    LoginCopyLink => "login-copy-link",
    /// Shown while the core exchanges the code (F1).
    LoginSigningIn => "login-signing-in",
    /// The Net worth view.
    NetWorth => "net-worth",
    /// The item table's header: "Prices fetched from" poe.ninja, then how
    /// long ago; its hover text (F5).
    PricesFetchedFrom => "prices-fetched-from",
    PricesFetchedInterval => "prices-fetched-interval",
    /// The toolbar warning: poe.ninja has no prices for the price league
    /// (F5).
    PricesUnavailable => "prices-unavailable",
    /// The toolbar warning's icon title; `league` (F5).
    PricesUnavailableTitle => "prices-unavailable-title",
    /// The item table's column headers (F11).
    ItemTableIcon => "item-table-icon",
    ItemTableName => "item-table-name",
    ItemTableIlvlTier => "item-table-ilvl-tier",
    ItemTableTabs => "item-table-tabs",
    ItemTableCorrupted => "item-table-corrupted",
    ItemTableLinks => "item-table-links",
    ItemTableQuality => "item-table-quality",
    ItemTableLevel => "item-table-level",
    ItemTableQuantity => "item-table-quantity",
    ItemTableTrend => "item-table-trend",
    ItemTablePrice => "item-table-price",
    ItemTableTotal => "item-table-total",
    ItemTableCumulative => "item-table-cumulative",
    /// The item table's cells (F11).
    ItemTableYes => "item-table-yes",
    ItemTableNo => "item-table-no",
    ItemTableNotAvailable => "item-table-not-available",
    ItemTableIconAlt => "item-table-icon-alt",
    ItemTableOpenOnNinja => "item-table-open-on-ninja",
    /// The item table without rows (F11).
    ItemTableEmpty => "item-table-empty",
    /// The item table's column menu (F11).
    ItemTableEditColumns => "item-table-edit-columns",
    ItemTableVisibleColumns => "item-table-visible-columns",
    /// The item table's pager; `from`, `to`, `count` (F11).
    ItemTableRowsPerPage => "item-table-rows-per-page",
    ItemTablePage => "item-table-page",
    ItemTableFirstPage => "item-table-first-page",
    ItemTablePreviousPage => "item-table-previous-page",
    ItemTableNextPage => "item-table-next-page",
    ItemTableLastPage => "item-table-last-page",
    /// The item table section's title (F10).
    ItemTableTitle => "item-table-title",
    /// The item table's views: Latest and Comparison tooltips (F11).
    ItemTableLatest => "item-table-latest",
    ItemTableComparison => "item-table-comparison",
    /// The item table's filter, tab filter and subtotal (F11).
    ItemTableSearch => "item-table-search",
    ItemTableClearSearch => "item-table-clear-search",
    ItemTableShowTabFilter => "item-table-show-tab-filter",
    ItemTableHideTabFilter => "item-table-hide-tab-filter",
    ItemTableSelectTabs => "item-table-select-tabs",
    ItemTableFilterTotal => "item-table-filter-total",
    /// The item table's CSV export button (F11).
    ItemTableExport => "item-table-export",
    /// The column menu's preset picker and legacy's presets (F11).
    ItemTableColumnPreset => "item-table-column-preset",
    ItemTablePresetCustom => "item-table-preset-custom",
    ItemTablePresetBreachstones => "item-table-preset-breachstones",
    ItemTablePresetCatalysts => "item-table-preset-catalysts",
    ItemTablePresetDeliriumOrbs => "item-table-preset-delirium-orbs",
    ItemTablePresetDivinationCards => "item-table-preset-divination-cards",
    ItemTablePresetEssences => "item-table-preset-essences",
    ItemTablePresetFossils => "item-table-preset-fossils",
    ItemTablePresetIncubators => "item-table-preset-incubators",
    ItemTablePresetProphecies => "item-table-preset-prophecies",
    ItemTablePresetScarabs => "item-table-preset-scarabs",
    ItemTablePresetSets => "item-table-preset-sets",
    /// Notification: signed in (F1).
    NotificationLoginSucceeded => "notification-login-succeeded",
    /// Notification and toast: a login failed (F1); the description says why.
    NotificationLoginFailed => "notification-login-failed",
    /// Notification: signed out (F1).
    NotificationSignedOut => "notification-signed-out",
    /// Notification and toast: sign-out failed (F1, F18); the description
    /// says why.
    NotificationSignOutFailed => "notification-sign-out-failed",
    /// Notification and toast: the GGG part of a session start failed (F1,
    /// F18); the description says why. Retried after 45 s.
    NotificationSessionStartFailed => "notification-session-start-failed",
    /// A deep link whose `state` is not the pending attempt's (D18).
    ErrorLoginRejected => "error-login-rejected",
    /// The user refused the authorization on pathofexile.com.
    ErrorLoginRefused => "error-login-refused",
    /// The system browser did not open (F1).
    ErrorBrowserNotOpened => "error-browser-not-opened",
    /// The clipboard could not be written (F1).
    ErrorClipboard => "error-clipboard",
    /// The GGG access token expired and could not be renewed now (D6).
    ErrorGggTokenExpired => "error-ggg-token-expired",
    /// [`crate::IpcError::RateLimited`]; `seconds`.
    ErrorRateLimited => "error-rate-limited",
    /// [`crate::IpcError::Busy`].
    ErrorBusy => "error-busy",
    /// [`crate::IpcError::Offline`].
    ErrorOffline => "error-offline",
    /// [`crate::IpcError::SessionExpired`].
    ErrorSessionExpired => "error-session-expired",
    /// [`crate::IpcError::Superseded`].
    ErrorSuperseded => "error-superseded",
    /// [`crate::IpcError::Internal`]; `correlation_id`.
    ErrorInternal => "error-internal",
    /// A backend `ErrorReason` without a message of its own.
    ErrorBackend => "error-backend",
    /// Backend `GGG_REFRESH_FAILED`.
    ErrorGggRefreshFailed => "error-ggg-refresh-failed",
    /// Backend `GROUP_NAME_TAKEN`.
    ErrorGroupNameTaken => "error-group-name-taken",
    /// Backend `GROUP_NOT_FOUND`.
    ErrorGroupNotFound => "error-group-not-found",
    /// Backend `WRONG_PASSWORD`.
    ErrorWrongPassword => "error-wrong-password",
    /// Backend `NOT_IN_GROUP`.
    ErrorNotInGroup => "error-not-in-group",
    /// Backend `GGG_AUTH_FAILED`.
    ErrorGggAuthFailed => "error-ggg-auth-failed",
    /// The app version in the title bar; `version` (F15).
    TitleBarVersion => "title-bar-version",
    /// The ko-fi button in the title bar (F15).
    TitleBarKofi => "title-bar-kofi",
    /// The support button in the title bar (F15).
    TitleBarSupport => "title-bar-support",
    /// Window controls in the title bar (F15).
    TitleBarMinimize => "title-bar-minimize",
    TitleBarMaximize => "title-bar-maximize",
    TitleBarRestore => "title-bar-restore",
    TitleBarClose => "title-bar-close",
    /// The title bar's banner once an update is downloaded (F16).
    TitleBarUpdateReady => "title-bar-update-ready",
    /// Support panel links (F15, F19).
    SupportDiscord => "support-discord",
    SupportWhatsNew => "support-whats-new",
    SupportRetakeTour => "support-retake-tour",
    SupportFeatureRequest => "support-feature-request",
    SupportBugReport => "support-bug-report",
    SupportUs => "support-us",
    SupportCredits => "support-credits",
    /// What's New's title (F19).
    WhatsNewTitle => "whats-new-title",
    /// What's New and Credits while their markdown loads, when it could not
    /// be fetched, and their close button (F19).
    InfoDialogLoading => "info-dialog-loading",
    InfoDialogUnavailable => "info-dialog-unavailable",
    InfoDialogClose => "info-dialog-close",
    /// The first-run tour (F19): its arrows' labels, last button, and each
    /// step's title, text and second paragraph.
    TourBack => "tour-back",
    TourNext => "tour-next",
    TourLetsBegin => "tour-lets-begin",
    TourInitialTitle => "tour-initial-title",
    TourInitialBody => "tour-initial-body",
    TourNetWorthTitle => "tour-net-worth-title",
    TourNetWorthBody => "tour-net-worth-body",
    TourSettingsTitle => "tour-settings-title",
    TourSettingsBody => "tour-settings-body",
    TourProfileTitle => "tour-profile-title",
    TourProfileBody => "tour-profile-body",
    TourProfileBody2 => "tour-profile-body2",
    TourSnapshotTitle => "tour-snapshot-title",
    TourSnapshotBody => "tour-snapshot-body",
    TourSnapshotBody2 => "tour-snapshot-body2",
    TourOverlayTitle => "tour-overlay-title",
    TourOverlayBody => "tour-overlay-body",
    TourGroupTitle => "tour-group-title",
    TourGroupBody => "tour-group-body",
    TourNotificationsTitle => "tour-notifications-title",
    TourNotificationsBody => "tour-notifications-body",
    TourCurrencySwitchTitle => "tour-currency-switch-title",
    TourCurrencySwitchBody => "tour-currency-switch-body",
    TourResetIncomeTitle => "tour-reset-income-title",
    TourResetIncomeBody => "tour-reset-income-body",
    TourSupportPanelTitle => "tour-support-panel-title",
    TourSupportPanelBody => "tour-support-panel-body",
    /// The navigation drawer (F15).
    NavOpen => "nav-open",
    NavClose => "nav-close",
    NavNetWorth => "nav-net-worth",
    NavSettings => "nav-settings",
    /// The account menu in the toolbar and its Sign out item (F1).
    AccountMenu => "account-menu",
    AccountSignOut => "account-sign-out",
    /// The exit action setting and its choices (F15).
    SettingsExitAction => "settings-exit-action",
    SettingsExitActionMinimizeToTray => "settings-exit-action-minimize-to-tray",
    SettingsExitActionExit => "settings-exit-action-exit",
    /// The release channel setting, its choices and its note (F16).
    SettingsReleaseChannel => "settings-release-channel",
    SettingsReleaseChannelLatest => "settings-release-channel-latest",
    SettingsReleaseChannelBeta => "settings-release-channel-beta",
    SettingsReleaseChannelNote => "settings-release-channel-note",
    /// The toolbar's Profile controls (F2).
    ProfileSwitcher => "profile-switcher",
    ProfileEdit => "profile-edit",
    ProfileCreate => "profile-create",
    ProfileRemove => "profile-remove",
    /// The Profile dialog (F2): titles, fields and buttons.
    ProfileDialogCreateTitle => "profile-dialog-create-title",
    ProfileDialogEditTitle => "profile-dialog-edit-title",
    ProfileName => "profile-name",
    ProfileNamePlaceholder => "profile-name-placeholder",
    ProfileGenerateName => "profile-generate-name",
    ProfilePriceLeague => "profile-price-league",
    ProfileLeague => "profile-league",
    ProfileStashTabs => "profile-stash-tabs",
    /// Shown with 5 or more selected tabs (F2).
    ProfileStashTabsWarning => "profile-stash-tabs-warning",
    /// The stash tab picker's input, its empty list and its buttons (legacy
    /// MUI Autocomplete).
    ProfileStashTabsPlaceholder => "profile-stash-tabs-placeholder",
    ProfileStashTabsNone => "profile-stash-tabs-none",
    ProfileStashTabsClear => "profile-stash-tabs-clear",
    ProfileStashTabsOpen => "profile-stash-tabs-open",
    ProfileStashTabsClose => "profile-stash-tabs-close",
    ProfileCharacter => "profile-character",
    /// The character choice's empty option.
    ProfileNoCharacter => "profile-no-character",
    ProfileIncludeEquipment => "profile-include-equipment",
    ProfileIncludeInventory => "profile-include-inventory",
    /// While the dialog's leagues, tabs and characters load.
    ProfileOptionsLoading => "profile-options-loading",
    ProfileDialogCreate => "profile-dialog-create",
    ProfileDialogSave => "profile-dialog-save",
    Cancel => "cancel",
    /// The remove confirmation (F2).
    ProfileRemoveTitle => "profile-remove-title",
    ProfileRemoveBody => "profile-remove-body",
    ProfileRemoveConfirm => "profile-remove-confirm",
    /// The Profile name is blank (F2).
    ErrorProfileNameRequired => "error-profile-name-required",
    /// The chosen league has no characters (F2).
    ErrorNoCharactersInLeague => "error-no-characters-in-league",
    /// poe.ninja's price leagues did not load (D21).
    ErrorPriceLeagues => "error-price-leagues",
    /// Notifications of Profile changes (F2, F18): successes list-only,
    /// failures also as a toast whose description says why.
    NotificationProfileCreated => "notification-profile-created",
    NotificationProfileCreateFailed => "notification-profile-create-failed",
    NotificationProfileUpdated => "notification-profile-updated",
    NotificationProfileUpdateFailed => "notification-profile-update-failed",
    NotificationProfileChanged => "notification-profile-changed",
    NotificationProfileChangeFailed => "notification-profile-change-failed",
    NotificationProfileRemoved => "notification-profile-removed",
    NotificationProfileRemoveFailed => "notification-profile-remove-failed",
    /// The Profile dialog's leagues, tabs or characters did not load.
    NotificationProfileOptionsFailed => "notification-profile-options-failed",
    /// The Settings tabs (F17).
    SettingsTabGeneral => "settings-tab-general",
    SettingsTabUi => "settings-tab-ui",
    SettingsTabPrices => "settings-tab-prices",
    SettingsTabAdvanced => "settings-tab-advanced",
    /// The Settings sections' titles (F17).
    SettingsSectionExitAction => "settings-section-exit-action",
    SettingsSectionSnapshot => "settings-section-snapshot",
    SettingsSectionPricing => "settings-section-pricing",
    SettingsSectionInterface => "settings-section-interface",
    SettingsSectionPrices => "settings-section-prices",
    SettingsSectionReleaseChannel => "settings-section-release-channel",
    SettingsSectionHardwareAcceleration => "settings-section-hardware-acceleration",
    /// The note on settings marked * (F5).
    SettingsRequiresSnapshot => "settings-requires-snapshot",
    /// Auto-snapshot on/off and its interval: label, value, help (F4).
    SettingsAutoSnapshotLabel => "settings-auto-snapshot-label",
    SettingsAutoSnapshot => "settings-auto-snapshot",
    SettingsAutoSnapshotHelp => "settings-auto-snapshot-help",
    SettingsAutoSnapshotMinutes => "settings-auto-snapshot-minutes",
    SettingsAutoSnapshotMinutesHelp => "settings-auto-snapshot-minutes-help",
    /// The interval's unit.
    SettingsUnitMinutes => "settings-unit-minutes",
    /// Pricing options: label, value, help (F5).
    SettingsLowConfidencePricingLabel => "settings-low-confidence-pricing-label",
    SettingsLowConfidencePricing => "settings-low-confidence-pricing",
    SettingsLowConfidencePricingHelp => "settings-low-confidence-pricing-help",
    SettingsPriceThreshold => "settings-price-threshold",
    SettingsPriceThresholdHelp => "settings-price-threshold-help",
    SettingsTotalPriceThreshold => "settings-total-price-threshold",
    SettingsTotalPriceThresholdHelp => "settings-total-price-threshold-help",
    /// UI scale, its help and its reset (F17).
    SettingsUiScale => "settings-ui-scale",
    SettingsUiScaleHelp => "settings-ui-scale-help",
    SettingsUiScaleReset => "settings-ui-scale-reset",
    /// Hardware acceleration: label, value and note (D7).
    SettingsHardwareAccelerationLabel => "settings-hardware-acceleration-label",
    SettingsHardwareAcceleration => "settings-hardware-acceleration",
    SettingsHardwareAccelerationNote => "settings-hardware-acceleration-note",
    /// Reset data, its description and confirmation (F17).
    SettingsResetData => "settings-reset-data",
    SettingsResetDataDescription => "settings-reset-data-description",
    SettingsResetDataQuestion => "settings-reset-data-question",
    SettingsResetDataConfirm => "settings-reset-data-confirm",
    SettingsResetDataCancel => "settings-reset-data-cancel",
    /// Debug mode: label, value and note (F20).
    SettingsDebugModeLabel => "settings-debug-mode-label",
    SettingsDebugMode => "settings-debug-mode",
    SettingsDebugModeNote => "settings-debug-mode-note",
    /// The diagnostics section of Settings → Advanced and its note (F20).
    SettingsDiagnostics => "settings-diagnostics",
    SettingsDiagnosticsNote => "settings-diagnostics-note",
    /// Export diagnostics, in Settings and the support panel (F20).
    ExportDiagnostics => "export-diagnostics",
    /// Open log folder (F20).
    OpenLogFolder => "open-log-folder",
    /// The toolbar's marker while Debug mode is on, and its title (F20).
    DebugMarker => "debug-marker",
    DebugMarkerTitle => "debug-marker-title",
    /// Notification: the last session ended unexpectedly, pointing to Export
    /// diagnostics (F20). List-only.
    NotificationLastSessionCrashed => "notification-last-session-crashed",
    NotificationLastSessionCrashedDescription => "notification-last-session-crashed-description",
    /// Notification: the Diagnostics bundle is saved; `path` (F20).
    NotificationDiagnosticsExported => "notification-diagnostics-exported",
    /// Notification and toast: the Diagnostics bundle could not be saved, or
    /// the log folder not opened (F20); the description says why.
    NotificationDiagnosticsExportFailed => "notification-diagnostics-export-failed",
    NotificationLogFolderFailed => "notification-log-folder-failed",
    /// Notification: the item table's CSV export failed (F11); the
    /// description says why.
    NotificationCsvExportFailed => "notification-csv-export-failed",
    /// A setting outside its range (F4, F5, F17).
    ErrorSettingOutOfRange => "error-setting-out-of-range",
    /// The Custom prices tab (F6): price league, name filter, the text
    /// until prices are available, the price table's columns and actions.
    CustomPricesPriceLeague => "custom-prices-price-league",
    CustomPricesFilter => "custom-prices-filter",
    CustomPricesClearFilter => "custom-prices-clear-filter",
    CustomPricesWaiting => "custom-prices-waiting",
    CustomPricesEmpty => "custom-prices-empty",
    CustomPricesIcon => "custom-prices-icon",
    CustomPricesName => "custom-prices-name",
    CustomPricesIlvlTier => "custom-prices-ilvl-tier",
    CustomPricesVariant => "custom-prices-variant",
    CustomPricesCorrupted => "custom-prices-corrupted",
    CustomPricesLinks => "custom-prices-links",
    CustomPricesQuality => "custom-prices-quality",
    CustomPricesLevel => "custom-prices-level",
    CustomPricesPrice => "custom-prices-price",
    CustomPricesCustomPrice => "custom-prices-custom-price",
    CustomPricesNoData => "custom-prices-no-data",
    CustomPricesNotSet => "custom-prices-not-set",
    CustomPricesSet => "custom-prices-set",
    CustomPricesRemove => "custom-prices-remove",
    /// The custom-price dialog (F6).
    CustomPriceDialogTitle => "custom-price-dialog-title",
    CustomPriceDialogPlaceholder => "custom-price-dialog-placeholder",
    CustomPriceDialogClose => "custom-price-dialog-close",
    CustomPriceDialogSave => "custom-price-dialog-save",
    /// The custom price is empty (F6).
    ErrorCustomPriceRequired => "error-custom-price-required",
    /// The custom price is negative or not a number (F6).
    ErrorCustomPriceInvalid => "error-custom-price-invalid",
    /// The status while the legacy import runs (F21).
    StatusImportingLegacy => "status-importing-legacy",
    /// The import dialog's title, and the Settings → Advanced section and
    /// button that run it (F21).
    LegacyImportTitle => "legacy-import-title",
    /// The dialog's text above the legacy accounts.
    LegacyImportIntro => "legacy-import-intro",
    /// The dialog's note: everything ticked goes into the signed-in account
    /// `account`; values already set here are kept.
    LegacyImportNote => "legacy-import-note",
    /// A legacy Profile's Snapshot count; `count`.
    LegacyImportSnapshotCount => "legacy-import-snapshot-count",
    /// The date range of a legacy Profile's Snapshots; `from`, `to`.
    LegacyImportDateRange => "legacy-import-date-range",
    /// A legacy Profile without a name.
    LegacyImportUnnamedProfile => "legacy-import-unnamed-profile",
    /// A legacy account without Profiles.
    LegacyImportNoProfiles => "legacy-import-no-profiles",
    /// The dialog's buttons.
    LegacyImportConfirm => "legacy-import-confirm",
    LegacyImportSkip => "legacy-import-skip",
    /// The Settings → Advanced section's note.
    SettingsLegacyImportNote => "settings-legacy-import-note",
    /// Notification: the import finished; `profiles` created, `snapshots`
    /// added (F21). List-only.
    NotificationLegacyImportSucceeded => "notification-legacy-import-succeeded",
    /// Notification and toast: the import or its scan failed (F21); the
    /// description says why.
    NotificationLegacyImportFailed => "notification-legacy-import-failed",
    /// Toast: the scan found no Exilence CE data (F21).
    LegacyImportNoData => "legacy-import-no-data",
    /// Exilence CE holds its data open (F21).
    ErrorLegacyRunning => "error-legacy-running",
    /// The Exilence CE data could not be decoded (F21).
    ErrorLegacyUnreadable => "error-legacy-unreadable",
    /// The tray menu (F15).
    TrayShow => "tray-show",
    TrayCheckForUpdates => "tray-check-for-updates",
    TrayDonate => "tray-donate",
    TrayDiscordHelp => "tray-discord-help",
    TrayQuit => "tray-quit",
    /// The tray's update sublabel (D13): no update.
    TrayUpdateNone => "tray-update-none",
    /// The tray's update sublabel (D13): `version` is available.
    TrayUpdateAvailable => "tray-update-available",
    /// The tray's update sublabel (D13): downloading `version`, `percent`.
    TrayUpdateDownloading => "tray-update-downloading",
    /// The tray's update sublabel (D13): `version` is downloaded.
    TrayUpdateReady => "tray-update-ready",
    /// The toolbar's Group button and the Group drawer (F13).
    GroupOpen => "group-open",
    GroupClose => "group-close",
    GroupCreate => "group-create",
    GroupJoin => "group-join",
    GroupLeave => "group-leave",
    /// The Group drawer's player list (F13).
    GroupPlayers => "group-players",
    /// The create and join dialogs (F13).
    GroupCreateTitle => "group-create-title",
    GroupJoinTitle => "group-join-title",
    GroupName => "group-name",
    GroupNamePlaceholder => "group-name-placeholder",
    GroupGenerateName => "group-generate-name",
    GroupPassword => "group-password",
    GroupPasswordPlaceholder => "group-password-placeholder",
    GroupPasswordHelper => "group-password-helper",
    /// The leave confirmation (F13).
    GroupLeaveTitle => "group-leave-title",
    GroupLeaveBody => "group-leave-body",
    /// The Group name is empty (F13).
    ErrorGroupNameRequired => "error-group-name-required",
    /// The Group password is shorter than 6 characters (F13).
    ErrorGroupPasswordTooShort => "error-group-password-too-short",
    /// Notification: created or joined a Group (F13).
    NotificationGroupJoined => "notification-group-joined",
    /// Notification and toast: creating or joining a Group failed (F13,
    /// F18); the description says why.
    NotificationGroupJoinFailed => "notification-group-join-failed",
    /// Notification: left the Group (F13).
    NotificationGroupLeft => "notification-group-left",
    /// Notification and toast: leaving the Group failed (F13, F18); the
    /// description says why.
    NotificationGroupLeaveFailed => "notification-group-leave-failed",
}

/// An argument of a [`Message`]. Text arguments carry data (names of tabs,
/// accounts, Groups), never English prose.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageArg {
    Number(f64),
    Text(String),
}

impl From<f64> for MessageArg {
    fn from(value: f64) -> Self {
        Self::Number(value)
    }
}

impl From<u32> for MessageArg {
    fn from(value: u32) -> Self {
        Self::Number(value.into())
    }
}

impl From<u64> for MessageArg {
    /// Exact up to 2^53, far beyond any count or duration the app shows.
    fn from(value: u64) -> Self {
        Self::Number(value as f64)
    }
}

impl From<String> for MessageArg {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

impl From<&str> for MessageArg {
    fn from(value: &str) -> Self {
        Self::Text(value.to_owned())
    }
}

/// User-facing text as a key with named arguments.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub key: MessageKey,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub args: BTreeMap<String, MessageArg>,
}

impl Message {
    /// A message without arguments.
    pub fn new(key: MessageKey) -> Self {
        Self {
            key,
            args: BTreeMap::new(),
        }
    }

    /// Adds the argument `name`.
    pub fn arg(mut self, name: &str, value: impl Into<MessageArg>) -> Self {
        self.args.insert(name.to_owned(), value.into());
        self
    }
}

impl From<MessageKey> for Message {
    fn from(key: MessageKey) -> Self {
        Self::new(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Text crosses IPC as a key with arguments, never resolved (D10).
    #[test]
    fn d10_messages_cross_ipc_as_keys() {
        let message = Message::new(MessageKey::NotificationsUnread).arg("count", 3u32);
        let json = serde_json::to_string(&message).unwrap();
        assert_eq!(
            json,
            r#"{"key":"notifications-unread","args":{"count":3.0}}"#
        );
        assert_eq!(serde_json::from_str::<Message>(&json).unwrap(), message);
        assert_eq!(
            serde_json::to_string(&Message::new(MessageKey::AppTitle)).unwrap(),
            r#"{"key":"app-title"}"#
        );
    }
}
