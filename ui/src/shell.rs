//! The desktop shell of the main window (F15): the custom title bar with its
//! support panel (F19), and the navigation drawer.

use ipc::command::{
    ExportDiagnostics, InstallUpdate, IssueTemplate, NoArgs, OpenIssueTemplate,
    OpenIssueTemplateArgs, OpenLink, OpenLinkArgs, SettingsPatch, SignOut, UpdateSettings,
    UpdateSettingsArgs,
};
use ipc::slices::{LoginState, UpdateState};
use ipc::{MessageKey, links};
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::A;

use crate::LOCALE;
use crate::state::State;
use crate::tauri::{self, WindowControl};

/// Material icon paths (24×24).
pub mod icon {
    pub const MINIMIZE: &str = "M6 19h12v2H6z";
    pub const MAXIMIZE: &str = "M19 5v14H5V5h14m0-2H5c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h14c1.1 0 2-.9 2-2V5c0-1.1-.9-2-2-2z";
    pub const RESTORE: &str = "M3 5H1v16c0 1.1.9 2 2 2h16v-2H3V5zm18-4H7c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h14c1.1 0 2-.9 2-2V3c0-1.1-.9-2-2-2zm0 16H7V3h14v14z";
    pub const CLOSE: &str = "M19 6.41 17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z";
    pub const HELP: &str = "M11 18h2v-2h-2v2zm1-16C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm0 18c-4.41 0-8-3.59-8-8s3.59-8 8-8 8 3.59 8 8-3.59 8-8 8zm0-14c-2.21 0-4 1.79-4 4h2c0-1.1.9-2 2-2s2 .9 2 2c0 2-3 1.75-3 5h2c0-2.25 3-2.5 3-5 0-2.21-1.79-4-4-4z";
    pub const REDIRECT: &str = "M9 5v2h6.59L4 18.59 5.41 20 17 8.41V15h2V5z";
    pub const FAVORITE: &str = "M12 21.35l-1.45-1.32C5.4 15.36 2 12.28 2 8.5 2 5.42 4.42 3 7.5 3c1.74 0 3.41.81 4.5 2.09C13.09 3.81 14.76 3 16.5 3 19.58 3 22 5.42 22 8.5c0 3.78-3.4 6.86-8.55 11.54L12 21.35z";
    pub const CHEVRON_LEFT: &str = "M15.41 7.41 14 6l-6 6 6 6 1.41-1.41L10.83 12z";
    pub const CHEVRON_RIGHT: &str = "M10 6 8.59 7.41 13.17 12l-4.58 4.59L10 18l6-6z";
    pub const NET_WORTH: &str = "M11.8 10.9c-2.27-.59-3-1.2-3-2.15 0-1.09 1.01-1.85 2.7-1.85 1.78 0 2.44.85 2.5 2.1h2.21c-.07-1.72-1.12-3.3-3.21-3.81V3h-3v2.16c-1.94.42-3.5 1.68-3.5 3.61 0 2.31 1.91 3.46 4.7 4.13 2.5.6 3 1.48 3 2.41 0 .69-.49 1.79-2.7 1.79-2.06 0-2.87-.92-2.98-2.1h-2.2c.12 2.19 1.76 3.42 3.68 3.83V21h3v-2.15c1.95-.37 3.5-1.5 3.5-3.55 0-2.84-2.43-3.81-4.7-4.4z";
    pub const BELL: &str = "M12 22c1.1 0 2-.9 2-2h-4c0 1.1.89 2 2 2zm6-6v-5c0-3.07-1.64-5.64-4.5-6.32V4c0-.83-.67-1.5-1.5-1.5s-1.5.67-1.5 1.5v.68C7.63 5.36 6 7.92 6 11v5l-2 2v1h16v-1l-2-2z";
    pub const INFO: &str = "M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm1 15h-2v-6h2v6zm0-8h-2V7h2v2z";
    pub const SUCCESS: &str = "M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm-2 15-5-5 1.41-1.41L10 14.17l7.59-7.59L19 8l-9 9z";
    pub const WARNING: &str = "M1 21h22L12 2 1 21zm12-3h-2v-2h2v2zm0-4h-2v-4h2v4z";
    pub const ERROR: &str = "M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm1 15h-2v-2h2v2zm0-4h-2V7h2v6z";
    pub const SETTINGS: &str = "M19.14 12.94c.04-.3.06-.61.06-.94 0-.32-.02-.64-.07-.94l2.03-1.58c.18-.14.23-.41.12-.61l-1.92-3.32c-.12-.22-.37-.29-.59-.22l-2.39.96c-.5-.38-1.03-.7-1.62-.94l-.36-2.54c-.04-.24-.24-.41-.48-.41h-3.84c-.24 0-.43.17-.47.41l-.36 2.54c-.59.24-1.13.57-1.62.94l-2.39-.96c-.22-.08-.47 0-.59.22L2.74 8.87c-.12.21-.08.47.12.61l2.03 1.58c-.05.3-.09.63-.09.94s.02.64.07.94l-2.03 1.58c-.18.14-.23.41-.12.61l1.92 3.32c.12.22.37.29.59.22l2.39-.96c.5.38 1.03.7 1.62.94l.36 2.54c.05.24.24.41.48.41h3.84c.24 0 .44-.17.47-.41l.36-2.54c.59-.24 1.13-.56 1.62-.94l2.39.96c.22.08.47 0 .59-.22l1.92-3.32c.12-.22.07-.47-.12-.61l-2.01-1.58zM12 15.6c-1.98 0-3.6-1.62-3.6-3.6s1.62-3.6 3.6-3.6 3.6 1.62 3.6 3.6-1.62 3.6-3.6 3.6z";
    pub const ADD: &str = "M19 13h-6v6h-2v-6H5v-2h6V5h2v6h6v2z";
    pub const DELETE: &str =
        "M6 19c0 1.1.9 2 2 2h8c1.1 0 2-.9 2-2V7H6v12zM19 4h-3.5l-1-1h-5l-1 1H5v2h14V4z";
    pub const EDIT: &str = "M3 17.25V21h3.75L17.81 9.94l-3.75-3.75L3 17.25zM20.71 7.04c.39-.39.39-1.02 0-1.41l-2.34-2.34c-.39-.39-1.02-.39-1.41 0l-1.83 1.83 3.75 3.75 1.83-1.83z";
    pub const DICE: &str = "M19 3H5c-1.11 0-2 .9-2 2v14c0 1.1.89 2 2 2h14c1.1 0 2-.9 2-2V5c0-1.1-.9-2-2-2zM7.5 18c-.83 0-1.5-.67-1.5-1.5S6.67 15 7.5 15s1.5.67 1.5 1.5S8.33 18 7.5 18zm0-9C6.67 9 6 8.33 6 7.5S6.67 6 7.5 6 9 6.67 9 7.5 8.33 9 7.5 9zm4.5 4.5c-.83 0-1.5-.67-1.5-1.5s.67-1.5 1.5-1.5 1.5.67 1.5 1.5-.67 1.5-1.5 1.5zm4.5 4.5c-.83 0-1.5-.67-1.5-1.5s.67-1.5 1.5-1.5 1.5.67 1.5 1.5-.67 1.5-1.5 1.5zm0-9c-.83 0-1.5-.67-1.5-1.5S15.67 6 16.5 6s1.5.67 1.5 1.5S17.33 9 16.5 9z";
    /// Material "Update": the Snapshot button (F3).
    pub const UPDATE: &str = "M21 10.12h-6.78l2.74-2.82c-2.73-2.7-7.15-2.8-9.88-.1-2.73 2.71-2.73 7.08 0 9.79s7.15 2.71 9.88 0C18.32 15.65 19 14.08 19 12.1h2c0 1.98-.88 4.55-2.64 6.29-3.51 3.48-9.21 3.48-12.72 0-3.5-3.47-3.53-9.11-.02-12.58s9.14-3.47 12.65 0L21 3v7.12zM12.5 8v4.25l3.5 2.08-.72 1.21L11 13V8h1.5z";
    /// Material "Cancel": the Snapshot's Cancel button (F3).
    pub const CANCEL: &str = "M12 2C6.47 2 2 6.47 2 12s4.47 10 10 10 10-4.47 10-10S17.53 2 12 2zm5 13.59L15.59 17 12 13.41 8.41 17 7 15.59 10.59 12 7 8.41 8.41 7 12 10.59 15.59 7 17 8.41 13.41 12 17 15.59z";
    /// Material "ShowChart": the net worth chart section (F10).
    pub const SHOW_CHART: &str = "M3.5 18.49l6-6.01 4 4L22 6.92l-1.41-1.41-7.09 7.97-4-4L2 16.99z";
    /// Material "StackedLineChart": the per-tab chart section (F10).
    pub const STACKED_LINE_CHART: &str = "M2 19.99l7.5-7.51 4 4 7.09-7.97L22 9.92l-8.5 9.56-4-4-6 6.01-1.5-1.5zm1.5-4.5l6-6.01 4 4L22 3.92l-1.41-1.41-7.09 7.97-4-4L2 13.99l1.5 1.5z";
    /// Material "ExpandMore": a collapsible section's chevron.
    pub const EXPAND_MORE: &str = "M16.59 8.59 12 13.17 7.41 8.59 6 10l6 6 6-6z";
    /// Material "ArrowDropDown": an outlined select's or picker's caret.
    pub const ARROW_DROP_DOWN: &str = "M7 10l5 5 5-5z";
    /// Material "Save": the Profile dialog's submit button (F2).
    pub const SAVE: &str = "M17 3H5c-1.11 0-2 .9-2 2v14c0 1.1.89 2 2 2h14c1.1 0 2-.9 2-2V7l-4-4zm-5 16c-1.66 0-3-1.34-3-3s1.34-3 3-3 3 1.34 3 3-1.34 3-3 3zm3-10H5V5h10v4z";
    /// Material "CheckBoxOutlineBlank" and "CheckBox": an unchecked and a
    /// checked checkbox.
    pub const CHECKBOX_BLANK: &str = "M19 5v14H5V5h14m0-2H5c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h14c1.1 0 2-.9 2-2V5c0-1.1-.9-2-2-2z";
    pub const CHECKBOX: &str = "M19 3H5c-1.11 0-2 .9-2 2v14c0 1.1.89 2 2 2h14c1.11 0 2-.9 2-2V5c0-1.1-.89-2-2-2zm-9 14l-5-5 1.41-1.41L10 14.17l7.59-7.59L19 8l-9 9z";
    /// Material "AccountCircle": the toolbar's account menu (F1).
    pub const ACCOUNT_CIRCLE: &str = "M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm0 4c1.93 0 3.5 1.57 3.5 3.5S13.93 13 12 13s-3.5-1.57-3.5-3.5S10.07 6 12 6zm0 14c-2.03 0-4.43-.82-6.14-2.88C7.55 15.8 9.68 15 12 15s4.45.8 6.14 2.12C16.43 19.18 14.03 20 12 20z";
    /// Material "ChangeCircle": the currency switch (F9).
    pub const CHANGE_CIRCLE: &str = "M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm.06 17v-2.01H12c-1.28 0-2.56-.49-3.54-1.46-1.71-1.71-1.92-4.35-.64-6.29l1.1 1.1c-.71 1.33-.53 3.01.59 4.13.7.7 1.62 1.03 2.54 1.01v-2.14l2.83 2.83L12.06 19zm4.11-4.24-1.1-1.1c.71-1.33.53-3.01-.59-4.13C13.79 8.84 12.9 8.5 12 8.5h-.06v2.15L9.11 7.83 11.94 5v2.02c1.3-.02 2.61.45 3.6 1.45 1.7 1.7 1.91 4.35.63 6.29z";
    /// Material "Group": the Group button (F13).
    pub const GROUP: &str = "M16 11c1.66 0 2.99-1.34 2.99-3S17.66 5 16 5c-1.66 0-3 1.34-3 3s1.34 3 3 3zm-8 0c1.66 0 2.99-1.34 2.99-3S9.66 5 8 5C6.34 5 5 6.34 5 8s1.34 3 3 3zm0 2c-2.33 0-7 1.17-7 3.5V19h14v-2.5c0-2.33-4.67-3.5-7-3.5zm8 0c-.29 0-.62.02-.97.05 1.16.84 1.97 1.97 1.97 3.45V19h6v-2.5c0-2.33-4.67-3.5-7-3.5z";
    /// Material "AddToPhotos": the toolbar's overlay button (F14).
    pub const ADD_TO_PHOTOS: &str = "M4 6H2v14c0 1.1.9 2 2 2h14v-2H4V6zm16-4H8c-1.1 0-2 .9-2 2v12c0 1.1.9 2 2 2h12c1.1 0 2-.9 2-2V4c0-1.1-.9-2-2-2zm-1 9h-4v4h-2v-4H9V9h4V5h2v4h4v2z";
    /// Material "TrendingUp": the income widget (F8).
    pub const TRENDING_UP: &str =
        "m16 6 2.29 2.29-4.88 4.88-4-4L2 16.59 3.41 18l6-6 4 4 6.3-6.29L22 12V6z";
    /// Material "Timeline": an item's poe.ninja page (F11).
    pub const TIMELINE: &str = "M23 8c0 1.1-.9 2-2 2-.18 0-.35-.02-.51-.07l-3.56 3.55c.05.16.07.34.07.52 0 1.1-.9 2-2 2s-2-.9-2-2c0-.18.02-.36.07-.52l-2.55-2.55c-.16.05-.34.07-.52.07s-.36-.02-.52-.07l-4.55 4.56c.05.16.07.33.07.51 0 1.1-.9 2-2 2s-2-.9-2-2 .9-2 2-2c.18 0 .35.02.51.07l4.56-4.55C8.02 9.36 8 9.18 8 9c0-1.1.9-2 2-2s2 .9 2 2c0 .18-.02.36-.07.52l2.55 2.55c.16-.05.34-.07.52-.07s.36.02.52.07l3.55-3.56C19.02 8.35 19 8.18 19 8c0-1.1.9-2 2-2s2 .9 2 2z";
    /// Material "ViewColumn": the item table's column menu (F11).
    pub const VIEW_COLUMN: &str =
        "M14.67 5v14H9.33V5h5.34zm1 14H21V5h-5.33v14zm-7.34 0V5H3v14h5.33z";
    pub const FIRST_PAGE: &str = "M18.41 16.59 13.82 12l4.59-4.59L17 6l-6 6 6 6zM6 6h2v12H6z";
    pub const LAST_PAGE: &str = "M5.59 7.41 10.18 12l-4.59 4.59L7 18l6-6-6-6zM16 6h2v12h-2z";
    pub const ARROW_UPWARD: &str = "M4 12l1.41 1.41L11 7.83V20h2V7.83l5.58 5.59L20 12l-8-8-8 8z";
    pub const ARROW_DOWNWARD: &str =
        "M20 12l-1.41-1.41L13 16.17V4h-2v12.17l-5.58-5.59L4 12l8 8 8-8z";
    /// Material "DeleteSweep": the toolbar's Remove Snapshots button (F12).
    pub const DELETE_SWEEP: &str = "M15 16h4v2h-4zm0-8h7v2h-7zm0 4h6v2h-6zM3 18c0 1.1.9 2 2 2h6c1.1 0 2-.9 2-2V8H3v10zM14 5h-3l-1-1H6L5 5H2v2h12z";
    /// Material "Compare": the item table's Comparison view (F11).
    pub const COMPARE: &str = "M10 3H5c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h5v2h2V1h-2v2zm0 15H5l5-6v6zm9-15h-5v2h5v13l-5-6v9h5c1.1 0 2-.9 2-2V5c0-1.1-.9-2-2-2z";
    /// Material "Search": the item table's filter (F11).
    pub const SEARCH: &str = "M15.5 14h-.79l-.28-.27C15.41 12.59 16 11.11 16 9.5 16 5.91 13.09 3 9.5 3S3 5.91 3 9.5 5.91 16 9.5 16c1.61 0 3.09-.59 4.23-1.57l.27.28v.79l5 4.99L20.49 19l-4.99-5zm-6 0C7.01 14 5 11.99 5 9.5S7.01 5 9.5 5 14 7.01 14 9.5 11.99 14 9.5 14z";
    /// Material "ZoomIn": the UI scale setting (F17).
    pub const ZOOM_IN: &str = "M15.5 14h-.79l-.28-.27C15.41 12.59 16 11.11 16 9.5 16 5.91 13.09 3 9.5 3S3 5.91 3 9.5 5.91 16 9.5 16c1.61 0 3.09-.59 4.23-1.57l.27.28v.79l5 4.99L20.49 19l-4.99-5zm-6 0C7.01 14 5 11.99 5 9.5S7.01 5 9.5 5 14 7.01 14 9.5 11.99 14 9.5 14zM12 10h-2v2H9v-2H7V9h2V7h1v2h2v1z";
    /// Material "MonetizationOn": the Net worth widget (F8).
    pub const MONETIZATION_ON: &str = "M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zm1.41 16.09V20h-2.67v-1.93c-1.71-.36-3.16-1.46-3.27-3.4h1.96c.1 1.05.82 1.87 2.65 1.87 1.96 0 2.4-.98 2.4-1.59 0-.83-.44-1.61-2.67-2.14-2.48-.6-4.18-1.62-4.18-3.67 0-1.72 1.39-2.84 3.11-3.21V4h2.67v1.95c1.86.45 2.79 1.86 2.85 3.39H14.3c-.05-1.11-.64-1.87-2.22-1.87-1.5 0-2.4.68-2.4 1.64 0 .84.65 1.39 2.67 1.91s4.18 1.39 4.18 3.91c-.01 1.83-1.38 2.83-3.12 3.16z";
    /// Material "FilterList": the item table's tab filter (F11).
    pub const FILTER_LIST: &str = "M10 18h4v-2h-4v2zM3 6v2h18V6H3zm3 7h12v-2H6v2z";
    /// Material "GetApp": the item table's CSV export (F11).
    pub const GET_APP: &str = "M19 9h-4V3H9v6H5l7 7 7-7zM5 18v2h14v-2H5z";
    /// Material "StarRounded": the support panel's What's new (F19).
    pub const STAR: &str = "M12 17.27l4.15 2.51c.76.46 1.69-.22 1.49-1.08l-1.1-4.72 3.67-3.18c.67-.58.31-1.68-.57-1.75l-4.83-.41-1.89-4.46c-.34-.81-1.5-.81-1.84 0L9.19 8.63l-4.83.41c-.88.07-1.24 1.17-.57 1.75l3.67 3.18-1.1 4.72c-.2.86.73 1.54 1.49 1.08l4.15-2.5z";
    /// Material "History": the support panel's Re-take tour (F19).
    pub const HISTORY: &str = "M13 3c-4.97 0-9 4.03-9 9H1l3.89 3.89.07.14L9 12H6c0-3.87 3.13-7 7-7s7 3.13 7 7-3.13 7-7 7c-1.93 0-3.68-.79-4.94-2.06l-1.42 1.42C8.27 19.99 10.51 21 13 21c4.97 0 9-4.03 9-9s-4.03-9-9-9zm-1 5v5l4.28 2.54.72-1.21-3.5-2.08V8H12z";
    /// Material "CardGiftcard": the support panel's Credits (F19).
    pub const CARD_GIFTCARD: &str = "M20 6h-2.18c.11-.31.18-.65.18-1 0-1.66-1.34-3-3-3-1.05 0-1.96.54-2.5 1.35l-.5.67-.5-.68C10.96 2.54 10.05 2 9 2 7.34 2 6 3.34 6 5c0 .35.07.69.18 1H4c-1.11 0-1.99.89-1.99 2L2 19c0 1.11.89 2 2 2h16c1.11 0 2-.89 2-2V8c0-1.11-.89-2-2-2zm-5-2c.55 0 1 .45 1 1s-.45 1-1 1-1-.45-1-1 .45-1 1-1zM9 4c.55 0 1 .45 1 1s-.45 1-1 1-1-.45-1-1 .45-1 1-1zm11 15H4v-2h16v2zm0-5H4V8h5.08L7 10.83 8.62 12 11 8.76l1-1.36 1 1.36L15.38 12 17 10.83 14.92 8H20v6z";
}

#[component]
pub fn Icon(path: &'static str) -> impl IntoView {
    view! {
        <svg class="icon" viewBox="0 0 24 24" aria-hidden="true">
            <path d=path />
        </svg>
    }
}

/// Opens an allowlisted external link through the core.
pub fn open_link(target: &str) {
    tauri::send::<OpenLink>(OpenLinkArgs {
        target: target.to_owned(),
    });
}

/// What the support panel opens in the window (F19): the tour, What's New
/// and Credits.
#[derive(Clone, Copy, Default)]
pub struct Onboarding {
    pub tour: RwSignal<bool>,
    pub whats_new: RwSignal<bool>,
    pub credits: RwSignal<bool>,
}

/// The custom title bar: title, version, the update banner, ko-fi, support
/// and the window controls. Empty space drags the window; a double click
/// there toggles maximize.
#[component]
pub fn TitleBar(state: State, onboarding: Onboarding) -> impl IntoView {
    let maximized = RwSignal::new(false);
    spawn_local(async move {
        if let Err(error) = tauri::watch_maximized(move |m| maximized.set(m)).await {
            tauri::log(
                ipc::command::LogLevel::Error,
                format!("watch maximized: {error}"),
            );
        }
    });
    let support_open = RwSignal::new(false);
    let version =
        ipc::Message::new(MessageKey::TitleBarVersion).arg("version", env!("CARGO_PKG_VERSION"));
    let maximize_label = move || {
        LOCALE.key(if maximized.get() {
            MessageKey::TitleBarRestore
        } else {
            MessageKey::TitleBarMaximize
        })
    };
    let update_ready = move || {
        state
            .update
            .with(|u| matches!(u.state, UpdateState::Ready { .. }))
    };
    let install_update = |_| tauri::send::<InstallUpdate>(NoArgs {});
    view! {
        <header class="title-bar" data-tauri-drag-region="deep">
            <span class="title">{LOCALE.key(MessageKey::AppTitle)}</span>
            <span class="version">{LOCALE.text(&version)}</span>
            // Once an update is downloaded (F16): installs it and restarts.
            <Show when=update_ready>
                <button class="update-banner" on:click=install_update>
                    {LOCALE.key(MessageKey::TitleBarUpdateReady)}
                </button>
            </Show>
            <span class="spacer"></span>
            <button
                class="kofi"
                title=LOCALE.key(MessageKey::TitleBarKofi)
                on:click=|_| open_link(links::KOFI)
            >
                <img src="/kofi_button_stroke.webp" alt=LOCALE.key(MessageKey::TitleBarKofi) />
            </button>
            <button
                class="control support"
                data-tour="support-panel"
                class:active=move || support_open.get()
                title=LOCALE.key(MessageKey::TitleBarSupport)
                on:click=move |_| support_open.update(|open| *open = !*open)
            >
                <Icon path=icon::HELP />
            </button>
            <button
                class="control"
                title=LOCALE.key(MessageKey::TitleBarMinimize)
                on:click=|_| tauri::window_control(WindowControl::Minimize)
            >
                <Icon path=icon::MINIMIZE />
            </button>
            <button
                class="control"
                title=maximize_label
                on:click=|_| tauri::window_control(WindowControl::ToggleMaximize)
            >
                {move || {
                    if maximized.get() {
                        view! { <Icon path=icon::RESTORE /> }
                    } else {
                        view! { <Icon path=icon::MAXIMIZE /> }
                    }
                }}
            </button>
            <button
                class="control close"
                title=LOCALE.key(MessageKey::TitleBarClose)
                on:click=|_| tauri::window_control(WindowControl::Close)
            >
                <Icon path=icon::CLOSE />
            </button>
        </header>
        <Show when=move || support_open.get()>
            <SupportPanel state open=support_open onboarding />
        </Show>
    }
}

/// The support button's panel: Discord, What's new, Re-take tour (signed in
/// only), Feature request, Bug report, Export diagnostics (F20), Support us
/// and Credits (F19). GitHub links go to the client repository (D11). A
/// click outside closes it.
#[component]
fn SupportPanel(state: State, open: RwSignal<bool>, onboarding: Onboarding) -> impl IntoView {
    let follow = move |target: &'static str| {
        move |_| {
            open_link(target);
            open.set(false);
        }
    };
    let show = move |dialog: RwSignal<bool>| {
        move |_| {
            dialog.set(true);
            open.set(false);
        }
    };
    let issue = move |template: IssueTemplate| {
        move |_| {
            tauri::send::<OpenIssueTemplate>(OpenIssueTemplateArgs { template });
            open.set(false);
        }
    };
    let signed_in = move || state.session.with(|s| s.login == LoginState::SignedIn);
    view! {
        <div class="backdrop" on:click=move |_| open.set(false)></div>
        <ul class="support-panel">
            <li>
                <button on:click=follow(links::DISCORD)>
                    <span>{LOCALE.key(MessageKey::SupportDiscord)}</span>
                    <Icon path=icon::REDIRECT />
                </button>
            </li>
            <li>
                <button on:click=show(onboarding.whats_new)>
                    <span>{LOCALE.key(MessageKey::SupportWhatsNew)}</span>
                    <Icon path=icon::STAR />
                </button>
            </li>
            <Show when=signed_in>
                <li>
                    <button on:click=show(onboarding.tour)>
                        <span>{LOCALE.key(MessageKey::SupportRetakeTour)}</span>
                        <Icon path=icon::HISTORY />
                    </button>
                </li>
            </Show>
            <li class="separator" role="separator"></li>
            <li>
                <button on:click=issue(IssueTemplate::FeatureRequest)>
                    <span>{LOCALE.key(MessageKey::SupportFeatureRequest)}</span>
                </button>
            </li>
            <li>
                <button on:click=issue(IssueTemplate::BugReport)>
                    <span>{LOCALE.key(MessageKey::SupportBugReport)}</span>
                </button>
            </li>
            <li>
                <button on:click=move |_| {
                    tauri::send::<ExportDiagnostics>(NoArgs {});
                    open.set(false);
                }>
                    <span>{LOCALE.key(MessageKey::ExportDiagnostics)}</span>
                </button>
            </li>
            <li class="separator" role="separator"></li>
            <li>
                <button on:click=follow(links::KOFI)>
                    <span>{LOCALE.key(MessageKey::SupportUs)}</span>
                    <Icon path=icon::FAVORITE />
                </button>
            </li>
            <li>
                <button on:click=show(onboarding.credits)>
                    <span>{LOCALE.key(MessageKey::SupportCredits)}</span>
                    <Icon path=icon::CARD_GIFTCARD />
                </button>
            </li>
        </ul>
    }
}

/// The toolbar's account menu: legacy's account icon opens it; Sign out is
/// disabled while an operation (a Snapshot) runs (F1). A click outside
/// closes it.
#[component]
pub fn AccountMenu(state: State) -> impl IntoView {
    let open = RwSignal::new(false);
    let busy = move || state.status.with(|s| s.operation_running);
    let sign_out = move |_| {
        open.set(false);
        tauri::send::<SignOut>(NoArgs {});
    };
    view! {
        <button
            class="control account"
            title=LOCALE.key(MessageKey::AccountMenu)
            aria-haspopup="menu"
            aria-expanded=move || open.get().to_string()
            on:click=move |_| open.update(|o| *o = !*o)
        >
            <Icon path=icon::ACCOUNT_CIRCLE />
        </button>
        <Show when=move || open.get()>
            <div class="backdrop" on:click=move |_| open.set(false)></div>
            <ul class="account-menu" role="menu">
                <li>
                    <button role="menuitem" disabled=busy on:click=sign_out>
                        {LOCALE.key(MessageKey::AccountSignOut)}
                    </button>
                </li>
            </ul>
        </Show>
    }
}

/// The left navigation: Net worth and Settings; its open state is a
/// remembered setting.
#[component]
pub fn NavDrawer(state: State) -> impl IntoView {
    let open = move || state.settings.with(|s| s.nav_drawer_open);
    let toggle = move |_| {
        tauri::send::<UpdateSettings>(UpdateSettingsArgs {
            patch: SettingsPatch {
                nav_drawer_open: Some(!state.settings.with_untracked(|s| s.nav_drawer_open)),
                ..SettingsPatch::default()
            },
        });
    };
    let toggle_label = move || {
        LOCALE.key(if open() {
            MessageKey::NavClose
        } else {
            MessageKey::NavOpen
        })
    };
    view! {
        <nav class="nav-drawer" class:open=open>
            <div class="nav-header">
                <button class="control" title=toggle_label on:click=toggle>
                    {move || {
                        if open() {
                            view! { <Icon path=icon::CHEVRON_LEFT /> }
                        } else {
                            view! { <Icon path=icon::CHEVRON_RIGHT /> }
                        }
                    }}
                </button>
            </div>
            <A href="/" exact=true>
                <span
                    class="nav-item"
                    data-tour="net-worth"
                    title=LOCALE.key(MessageKey::NavNetWorth)
                >
                    <Icon path=icon::NET_WORTH />
                    <span class="nav-label">{LOCALE.key(MessageKey::NavNetWorth)}</span>
                </span>
            </A>
            <A href="/settings">
                <span
                    class="nav-item"
                    data-tour="settings"
                    title=LOCALE.key(MessageKey::NavSettings)
                >
                    <Icon path=icon::SETTINGS />
                    <span class="nav-label">{LOCALE.key(MessageKey::NavSettings)}</span>
                </span>
            </A>
        </nav>
    }
}
