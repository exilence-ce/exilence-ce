//! The command handlers. Names and arguments match `ipc::command`; every
//! handler returns `Result<_, IpcError>`.

use std::sync::Arc;

use ipc::command::{
    GroupForm, IssueTemplate, LogLevel, NewProfile, PriceKey, PriceTableRow, ProfileOptions,
    SettingsPatch, SnapshotEntry,
};
use ipc::slices::{Notifications, Platform, Profile, Settings};
use ipc::{Event, IpcError, SliceName};
use sqlx::SqlitePool;
use tauri::{AppHandle, Manager, Runtime, State};

use crate::desktop::Desktop;
use crate::github::{self, Github};
use crate::state::Store;
use crate::{
    auth, auto_snapshot, charts, csv_export, diagnostics, group, internal, legacy_import, links,
    net_worth, prices, profiles, reset, settings, shell, snapshot, snapshot_removal, updater,
};

/// `get_state(slices)`.
#[tauri::command]
pub fn get_state(store: State<'_, Store>, slices: Vec<SliceName>) -> Result<Vec<Event>, IpcError> {
    Ok(store.get(&slices))
}

/// `log(level, message)`: a UI panic or unexpected command failure (F20).
#[tauri::command]
pub fn log(level: LogLevel, message: String) -> Result<(), IpcError> {
    let level = match level {
        LogLevel::Error => log::Level::Error,
        LogLevel::Warn => log::Level::Warn,
        LogLevel::Info => log::Level::Info,
        LogLevel::Debug => log::Level::Debug,
    };
    log::log!(target: "ui", level, "{message}");
    Ok(())
}

/// `mark_notifications_read`: the user opened the notification list (F18).
#[tauri::command]
pub fn mark_notifications_read<R: Runtime>(
    app: AppHandle<R>,
    store: State<'_, Store>,
) -> Result<(), IpcError> {
    store
        .update::<Notifications, _>(&app, |n| n.unread = 0)
        .map_err(internal)
}

/// `update_settings(patch)`: stores the settings `patch` sets, then pushes
/// the `settings` slice. Debug mode applies at once (F20); a new chart time
/// span pushes the `charts` slice (F10).
#[tauri::command]
pub async fn update_settings<R: Runtime>(
    app: AppHandle<R>,
    store: State<'_, Store>,
    pool: State<'_, SqlitePool>,
    patch: SettingsPatch,
) -> Result<(), IpcError> {
    let entries = settings::save(&pool, &patch).await?;
    let old = store.read(Settings::clone);
    let mut merged = Ok(());
    store
        .update::<Settings, _>(&app, |s| merged = settings::merge(s, entries))
        .map_err(internal)?;
    merged.map_err(internal)?;
    auto_snapshot::settings_changed(&app, &old);
    if patch.ui_scale.is_some() {
        shell::apply_ui_scale(&app);
    }
    if let Some(on) = patch.debug_mode {
        diagnostics::set_debug_mode(&app, on);
    }
    if patch.chart_time_span.is_some() {
        charts::push(&app).await;
    }
    Ok(())
}

/// `open_link(target)`: opens `target` in the system browser if its host is
/// allowlisted (F15).
#[tauri::command]
pub fn open_link(target: String) -> Result<(), IpcError> {
    links::open(&target).map_err(internal)
}

/// `open_issue_template(template)`: the client repository's new-issue page
/// with `template`, in the system browser (F19, D11).
#[tauri::command]
pub fn open_issue_template(
    desktop: State<'_, Arc<dyn Desktop>>,
    template: IssueTemplate,
) -> Result<(), IpcError> {
    let target = github::issue_template_url(github::REPOSITORY, template);
    let url = links::allowed(&target)
        .ok_or_else(|| internal(format!("link not allowlisted: {target}")))?;
    desktop.open_url(url.as_str()).map_err(internal)
}

/// `get_changelog`: What's New's markdown from the client repository (F19,
/// D11).
#[tauri::command]
pub async fn get_changelog(repository: State<'_, Github>) -> Result<String, IpcError> {
    repository.file(github::CHANGELOG).await.map_err(internal)
}

/// `get_credits`: the Credits' markdown from the client repository (F19,
/// D11).
#[tauri::command]
pub async fn get_credits(repository: State<'_, Github>) -> Result<String, IpcError> {
    repository.file(github::CREDITS).await.map_err(internal)
}

/// `complete_tour`: remembers that the tour ended, then pushes the
/// `settings` slice. Whether What's New opens now: the first time only
/// (F19).
#[tauri::command]
pub async fn complete_tour<R: Runtime>(
    app: AppHandle<R>,
    store: State<'_, Store>,
    pool: State<'_, SqlitePool>,
) -> Result<bool, IpcError> {
    let open_whats_new = !store.read(|s: &Settings| s.whats_new_shown);
    let entries = settings::save_tour_completed(&pool)
        .await
        .map_err(internal)?;
    let mut merged = Ok(());
    store
        .update::<Settings, _>(&app, |s| merged = settings::merge(s, entries))
        .map_err(internal)?;
    merged.map_err(internal)?;
    Ok(open_whats_new)
}

/// `start_login(platform)`: remembers the platform and opens GGG's consent
/// page for a fresh login attempt (F1).
#[tauri::command]
pub async fn start_login<R: Runtime>(
    app: AppHandle<R>,
    platform: Platform,
) -> Result<(), IpcError> {
    auth::start_login(&app, platform).await
}

/// `copy_login_link`: puts the login attempt's authorization URL on the
/// clipboard (F1).
#[tauri::command]
pub fn copy_login_link<R: Runtime>(app: AppHandle<R>) -> Result<(), IpcError> {
    auth::copy_login_link(&app);
    Ok(())
}

/// `sign_out`: forgets this Login and returns to login (F1, D19); `Busy`
/// while the operation slot is taken.
#[tauri::command]
pub async fn sign_out<R: Runtime>(app: AppHandle<R>) -> Result<(), IpcError> {
    auth::sign_out(&app).await
}

/// `reset_data`: deletes all local data except logs, signs out and restarts
/// (F17). Returns only if that fails.
#[tauri::command]
pub async fn reset_data<R: Runtime>(app: AppHandle<R>) -> Result<(), IpcError> {
    let data_dir = app.path().app_data_dir().map_err(internal)?;
    reset::prepare(&app, &data_dir).await?;
    app.restart()
}

/// `install_update`: installs the downloaded update and restarts into it
/// (F16). Returns only if that fails. Async, so it runs off the main thread,
/// which the macOS installer may need while this waits. The exit counts as
/// clean: on Windows the installer ends the process at once (F20).
#[tauri::command]
pub async fn install_update<R: Runtime>(
    app: AppHandle<R>,
    pool: State<'_, SqlitePool>,
) -> Result<(), IpcError> {
    diagnostics::mark_clean_exit(&pool).await;
    let installed = updater::install(&app).await.map_err(internal);
    // Still running: the next crash must show.
    if let Err(error) = diagnostics::mark_running(&pool).await {
        log::warn!("cannot record the running session: {error}");
    }
    installed
}

/// `export_csv`: saves the CSV of the item table rows with these keys
/// where the user chooses (F11).
#[tauri::command(rename_all = "snake_case")]
pub async fn export_csv<R: Runtime>(
    app: AppHandle<R>,
    row_keys: Vec<String>,
) -> Result<(), IpcError> {
    csv_export::export(&app, &row_keys).await
}

/// `export_diagnostics`: saves the Diagnostics bundle where the user
/// chooses (F20).
#[tauri::command]
pub async fn export_diagnostics<R: Runtime>(app: AppHandle<R>) -> Result<(), IpcError> {
    diagnostics::export(&app).await
}

/// `open_log_folder`: opens the log directory (F20).
#[tauri::command]
pub fn open_log_folder<R: Runtime>(app: AppHandle<R>) -> Result<(), IpcError> {
    diagnostics::open_log_folder(&app)
}

/// `open_overlay`: opens the Net worth overlay, or brings it to the front
/// (F14). Async: a window built in a synchronous command deadlocks on
/// Windows.
#[tauri::command]
pub async fn open_overlay<R: Runtime>(app: AppHandle<R>) -> Result<(), IpcError> {
    shell::open_overlay(&app).map_err(internal)
}

/// `close_overlay`: closes the Net worth overlay (F14).
#[tauri::command]
pub fn close_overlay<R: Runtime>(app: AppHandle<R>) -> Result<(), IpcError> {
    shell::close_overlay(&app);
    Ok(())
}

/// `take_snapshot`: starts a Snapshot of the active Profile (F3).
#[tauri::command]
pub async fn take_snapshot<R: Runtime>(app: AppHandle<R>) -> Result<(), IpcError> {
    snapshot::start(&app).await
}

/// `cancel_snapshot`: stops the running Snapshot until it is saved (F3).
#[tauri::command]
pub fn cancel_snapshot<R: Runtime>(app: AppHandle<R>) -> Result<(), IpcError> {
    snapshot::cancel(&app);
    Ok(())
}

/// `reset_income`: sets the active Profile's income to 0 and starts its
/// window now (F8).
#[tauri::command]
pub async fn reset_income<R: Runtime>(app: AppHandle<R>) -> Result<(), IpcError> {
    net_worth::reset_income(&app).await.map_err(internal)
}

/// `list_snapshots`: the active Profile's Snapshots for removal (F12).
#[tauri::command]
pub async fn list_snapshots<R: Runtime>(app: AppHandle<R>) -> Result<Vec<SnapshotEntry>, IpcError> {
    snapshot_removal::list(&app).await
}

/// `remove_snapshots(snapshot_ids)` (F12, D5).
#[tauri::command(rename_all = "snake_case")]
pub async fn remove_snapshots<R: Runtime>(
    app: AppHandle<R>,
    snapshot_ids: Vec<String>,
) -> Result<(), IpcError> {
    snapshot_removal::remove(&app, snapshot_ids).await
}

/// `get_profile_options(league)`: the Profile dialog's leagues, and the
/// stash tabs and characters of `league` (F2).
#[tauri::command]
pub async fn get_profile_options<R: Runtime>(
    app: AppHandle<R>,
    league: Option<String>,
) -> Result<ProfileOptions, IpcError> {
    profiles::options(&app, league).await
}

/// `create_profile(profile)` (F2).
#[tauri::command]
pub async fn create_profile<R: Runtime>(
    app: AppHandle<R>,
    profile: NewProfile,
) -> Result<(), IpcError> {
    profiles::create(&app, profile).await
}

/// `update_profile(profile)` (F2).
#[tauri::command]
pub async fn update_profile<R: Runtime>(
    app: AppHandle<R>,
    profile: Profile,
) -> Result<(), IpcError> {
    profiles::update(&app, profile).await
}

/// `set_active_profile(profile_id)` (F2). Arguments in snake case, as
/// `ipc::command` serializes them.
#[tauri::command(rename_all = "snake_case")]
pub async fn set_active_profile<R: Runtime>(
    app: AppHandle<R>,
    profile_id: String,
) -> Result<(), IpcError> {
    profiles::set_active(&app, profile_id).await
}

/// `remove_profile(profile_id)` (F2).
#[tauri::command(rename_all = "snake_case")]
pub async fn remove_profile<R: Runtime>(
    app: AppHandle<R>,
    profile_id: String,
) -> Result<(), IpcError> {
    profiles::remove(&app, profile_id).await
}

/// `get_price_table(price_league)` (F6).
#[tauri::command(rename_all = "snake_case")]
pub async fn get_price_table<R: Runtime>(
    app: AppHandle<R>,
    price_league: String,
) -> Result<Vec<PriceTableRow>, IpcError> {
    prices::price_table(&app, &price_league).await
}

/// `set_custom_price(price_league, key, value)` (F6).
#[tauri::command(rename_all = "snake_case")]
pub async fn set_custom_price<R: Runtime>(
    app: AppHandle<R>,
    price_league: String,
    key: PriceKey,
    value: f64,
) -> Result<(), IpcError> {
    prices::set_custom_price(&app, &price_league, key, value).await
}

/// `remove_custom_price(price_league, key)` (F6).
#[tauri::command(rename_all = "snake_case")]
pub async fn remove_custom_price<R: Runtime>(
    app: AppHandle<R>,
    price_league: String,
    key: PriceKey,
) -> Result<(), IpcError> {
    prices::remove_custom_price(&app, &price_league, key).await
}

/// `scan_legacy_import`: Settings → Advanced → Import from Exilence CE
/// (F21).
#[tauri::command]
pub async fn scan_legacy_import<R: Runtime>(app: AppHandle<R>) -> Result<(), IpcError> {
    legacy_import::scan_command(&app).await
}

/// `run_legacy_import(account_names)` (F21).
#[tauri::command(rename_all = "snake_case")]
pub async fn run_legacy_import<R: Runtime>(
    app: AppHandle<R>,
    account_names: Vec<String>,
) -> Result<(), IpcError> {
    legacy_import::run(&app, account_names).await
}

/// `skip_legacy_import` (F21).
#[tauri::command]
pub async fn skip_legacy_import<R: Runtime>(app: AppHandle<R>) -> Result<(), IpcError> {
    legacy_import::skip(&app).await
}

/// `check_group_name(name)` (F13).
#[tauri::command]
pub async fn check_group_name<R: Runtime>(
    app: AppHandle<R>,
    name: String,
) -> Result<bool, IpcError> {
    group::check_name(&app, name).await
}

/// `create_group(group)` (F13).
#[tauri::command]
pub async fn create_group<R: Runtime>(app: AppHandle<R>, group: GroupForm) -> Result<(), IpcError> {
    group::enter(&app, group::Entry::Create, group).await
}

/// `join_group(group)` (F13).
#[tauri::command]
pub async fn join_group<R: Runtime>(app: AppHandle<R>, group: GroupForm) -> Result<(), IpcError> {
    group::enter(&app, group::Entry::Join, group).await
}

/// `leave_group` (F13).
#[tauri::command]
pub async fn leave_group<R: Runtime>(app: AppHandle<R>) -> Result<(), IpcError> {
    group::leave(&app).await
}

/// `set_player_selected(account_id, selected)` (F13).
#[tauri::command(rename_all = "snake_case")]
pub async fn set_player_selected<R: Runtime>(
    app: AppHandle<R>,
    account_id: String,
    selected: bool,
) -> Result<(), IpcError> {
    group::set_player_selected(&app, &account_id, selected).await;
    Ok(())
}
