pub mod auth;
pub mod auto_snapshot;
pub mod backend;
pub mod charts;
pub mod clock;
pub mod commands;
pub mod csv_export;
pub mod db;
pub mod desktop;
pub mod diagnostics;
pub mod ggg;
pub mod github;
pub mod group;
pub mod group_connection;
pub mod group_upload_queue;
pub mod items;
pub mod legacy_import;
pub mod links;
pub mod logging;
#[cfg(target_os = "linux")]
pub mod menu_entry;
pub mod net_worth;
pub mod prices;
pub mod profiles;
pub mod reset;
pub mod scheduler;
pub mod session_start;
pub mod settings;
pub mod shell;
pub mod slot;
pub mod snapshot;
pub mod snapshot_removal;
pub mod stash_tabs;
pub mod state;
pub mod tray;
pub mod updater;

use std::fmt::Display;
use std::path::Path;
use std::sync::Arc;

use ipc::slices::{
    Charts, Group, Items, LoginState, NetWorth, Prices, Profiles, ReleaseChannel, Session,
    Settings, Status, Update,
};
use ipc::{IpcError, Slices};
use sqlx::SqlitePool;
use tauri::plugin::TauriPlugin;
use tauri::{Builder, Manager, Runtime, WebviewWindowBuilder};
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_plugin_window_state::StateFlags;

use crate::backend::Backend;
use crate::clock::Clock;
use crate::desktop::Desktop;
use crate::ggg::Ggg;
use crate::scheduler::{Deadline, Jitter, Scheduler};
use crate::state::Pushing;

/// A distro package (AUR) installed this copy: built with `distro-package`.
/// Its package manager updates it, so the app never does (F16), and its
/// system-wide desktop entry declares the deep link.
pub const DISTRO_PACKAGE: bool = cfg!(feature = "distro-package");

/// Registers every command handler on `builder`.
pub fn with_commands<R: Runtime>(builder: Builder<R>) -> Builder<R> {
    builder.invoke_handler(tauri::generate_handler![
        commands::get_state,
        commands::log,
        commands::mark_notifications_read,
        commands::update_settings,
        commands::open_link,
        commands::open_issue_template,
        commands::get_changelog,
        commands::get_credits,
        commands::complete_tour,
        commands::install_update,
        commands::start_login,
        commands::copy_login_link,
        commands::get_profile_options,
        commands::create_profile,
        commands::update_profile,
        commands::set_active_profile,
        commands::remove_profile,
        commands::get_price_table,
        commands::set_custom_price,
        commands::remove_custom_price,
        commands::sign_out,
        commands::reset_data,
        commands::take_snapshot,
        commands::cancel_snapshot,
        commands::reset_income,
        commands::list_snapshots,
        commands::remove_snapshots,
        commands::export_csv,
        commands::export_diagnostics,
        commands::open_log_folder,
        commands::check_group_name,
        commands::create_group,
        commands::join_group,
        commands::leave_group,
        commands::set_player_selected,
        commands::scan_legacy_import,
        commands::run_legacy_import,
        commands::skip_legacy_import,
        commands::open_overlay,
        commands::close_overlay,
    ])
}

/// What the core talks to outside the app; tests replace each.
pub struct Environment {
    /// The backend's gRPC URL, [`backend::URL`] in the app.
    pub backend_url: String,
    /// GGG's API, [`ggg::API_URL`] in the app.
    pub ggg_url: String,
    /// poe.ninja, [`ninja::API_URL`] in the app.
    pub ninja_url: String,
    /// GitHub's raw files, [`github::RAW_URL`] in the app.
    pub github_raw_url: String,
    pub clock: Arc<dyn Clock>,
    /// The backoff's random factor.
    pub jitter: Arc<dyn Jitter>,
    pub desktop: Arc<dyn Desktop>,
    /// The legacy folders the legacy import reads (F21), this OS's in the
    /// app.
    pub legacy_folders: Vec<legacy_import::folders::LegacyFolder>,
}

/// Puts the database, the state store (holding the stored settings, Login,
/// Profiles and rate limits), the scheduler, the operation slot, the backend
/// client, the GGG, poe.ninja and GitHub clients, the price tables, the
/// login state, the updater and the desktop under the app's management. The
/// release channel in the stored settings is the one this run checks (F16).
/// A stored Debug mode turns on; a run that ended without a clean exit is
/// noted in the notification list (F20).
pub fn manage<R: Runtime>(
    app: &impl Manager<R>,
    pool: SqlitePool,
    environment: Environment,
) -> Result<(), Box<dyn std::error::Error>> {
    // Holds the item detail the start decodes for the first `items` push.
    let items_cache = items::Cache::default();
    let (settings, account, profiles, net_worth, items, charts, backend, ggg, crashed) =
        tauri::async_runtime::block_on(async {
            // Before anything uploads or rejoins a Group (F12).
            match items::sweep(&pool).await {
                Ok(removed) => log::info!("item definition sweep removed {removed} definitions"),
                Err(error) => log::error!("the item definition sweep failed: {error}"),
            }
            let settings = settings::load(&pool).await?;
            let account = auth::restore(&pool).await?;
            let profiles = match &account {
                Some(account) => profiles::load(&pool, &account.id).await?,
                None => Profiles::default(),
            };
            // Every income window starts with this run (F8).
            let (net_worth, items) = match (&account, &profiles.active_profile_id) {
                (Some(account), Some(profile_id)) => (
                    net_worth::load(&pool, &account.id, profile_id, 0.0).await?,
                    // Trends and poe.ninja pages follow once prices load (D3).
                    items::slice(
                        &items::newest(&pool, &items_cache, &account.id, profile_id).await?,
                        None,
                    ),
                ),
                _ => (NetWorth::default(), Items::default()),
            };
            let charts = charts::load_active(
                &pool,
                account.as_ref().map(|a| a.id.as_str()),
                &profiles,
                settings.chart_time_span,
                environment.clock.now(),
            )
            .await?;
            // The channel's worker needs the runtime.
            let backend = Backend::new(&environment.backend_url)?;
            let ggg = Ggg::open(
                pool.clone(),
                &environment.ggg_url,
                environment.clock.clone(),
            )
            .await?;
            let crashed = diagnostics::mark_running(&pool).await?;
            Ok::<_, Box<dyn std::error::Error>>((
                settings, account, profiles, net_worth, items, charts, backend, ggg, crashed,
            ))
        })?;
    // The log starts at Info: only a stored Debug mode changes it.
    if settings.debug_mode {
        logging::set_debug_mode(true);
    }
    let status = Status {
        debug_mode: settings.debug_mode || logging::debug_from_launch(),
        ..Status::default()
    };
    let session = Session {
        login: match account {
            Some(_) => LoginState::SignedIn,
            None => LoginState::SignedOut,
        },
        account_name: account.map(|a| a.name),
        platform: settings.platform,
        ..Session::default()
    };
    let rate_limit = ggg.rate_limit();
    let scheduler = Scheduler::new(environment.clock, environment.jitter);
    // A cooldown restored from before the restart (F7).
    if let Some(until) = rate_limit.cooldown_until {
        scheduler.set(Deadline::RateLimitCooldownEnd, until);
    }
    app.manage(shell::StartupHardwareAcceleration(
        settings.hardware_acceleration,
    ));
    app.manage(pool);
    let channel_choice = updater::channel_choice();
    app.manage(state::Store::new(Slices {
        update: Update {
            channel: if channel_choice {
                settings.release_channel
            } else {
                ReleaseChannel::Latest
            },
            channel_choice,
            ..Update::default()
        },
        settings,
        session,
        profiles,
        net_worth,
        items,
        charts,
        rate_limit,
        status,
        ..Slices::default()
    }));
    app.manage(net_worth::Income::new(scheduler.now()));
    app.manage(scheduler);
    app.manage(slot::OperationSlot::default());
    app.manage(updater::Updater::default());
    app.manage(backend);
    app.manage(ggg);
    app.manage(ninja::Ninja::new(
        &environment.ninja_url,
        auth::user_agent(),
    )?);
    app.manage(github::Github::new(
        &environment.github_raw_url,
        auth::user_agent(),
    )?);
    app.manage(prices::PriceTables::default());
    app.manage(Pushing::<Prices>::default());
    app.manage(Pushing::<Items>::default());
    app.manage(items_cache);
    app.manage(Pushing::<NetWorth>::default());
    app.manage(Pushing::<Charts>::default());
    app.manage(Pushing::<Profiles>::default());
    app.manage(profiles::FirstProfile::default());
    app.manage(session_start::Characters::default());
    app.manage(stash_tabs::StashTabs::default());
    app.manage(snapshot::Control::default());
    app.manage(session_start::BackendAttempts::default());
    app.manage(session_start::ProfilesKnown::default());
    app.manage(group::Groups::default());
    app.manage(Pushing::<Group>::default());
    app.manage(group_connection::GroupConnection::default());
    app.manage(group_upload_queue::UploadQueue::default());
    app.manage(legacy_import::LegacyFolders(environment.legacy_folders));
    app.manage(auth::Auth::new(environment.desktop.clone()));
    app.manage::<Arc<dyn Desktop>>(environment.desktop);
    if crashed {
        diagnostics::notify_crash(app.app_handle());
    }
    Ok(())
}

/// Logs `error` under a fresh correlation ID and returns it as
/// [`IpcError::Internal`].
pub fn internal(error: impl Display) -> IpcError {
    let correlation_id = uuid::Uuid::now_v7().to_string();
    log::error!("internal error {correlation_id}: {error}");
    IpcError::Internal { correlation_id }
}

/// Remembers the size, position and maximized state of the windows (F15) in
/// its own file next to the database (`client-schema.sql`). Never visibility:
/// a window hidden to the tray at quit must show at the next start.
fn window_state<R: Runtime>(data_dir: &Path) -> Result<TauriPlugin<R>, &'static str> {
    // The plugin joins the file name to the config directory, which differs
    // from the data directory on Linux; an absolute path replaces it.
    let file = data_dir.join(tauri_plugin_window_state::DEFAULT_FILENAME);
    let file = file.to_str().ok_or("the app data directory is not UTF-8")?;
    Ok(tauri_plugin_window_state::Builder::new()
        .with_filename(file)
        .with_state_flags(StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED)
        .build())
}

#[expect(
    clippy::expect_used,
    reason = "an app that cannot be built has nothing to run"
)]
pub fn run() {
    // First: a second instance hands over to the running one before
    // anything else starts. With its `deep-link` feature it forwards the
    // second instance's `exilence-ce-ng://` link to the deep-link plugin.
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(
            shell::on_second_instance,
        ))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        // Used by the core only: no capability grants the webview its
        // commands.
        .plugin(tauri_plugin_updater::Builder::new().build());
    with_commands(builder)
        .setup(|app| {
            // Without a log the app still runs; stderr is all that is left.
            if let Err(error) = logging::init(&app.path().app_log_dir()?) {
                eprintln!("cannot open the log: {error}");
            }
            diagnostics::install_panic_hook();
            let debug_var = std::env::var_os(logging::DEBUG_VAR);
            if logging::debug_at_launch(std::env::args_os(), debug_var.as_deref()) {
                logging::enable_debug_from_launch();
            }
            log::info!(
                "starting version {} on {} {}, Debug mode from launch: {}",
                env!("CARGO_PKG_VERSION"),
                std::env::consts::OS,
                std::env::consts::ARCH,
                logging::debug_from_launch()
            );
            let dir = app.path().app_data_dir()?;
            let locations = reset::Locations::of(app.path())?;
            if let Err(error) = reset::finish_pending(&dir, &locations) {
                log::error!("Reset data did not delete everything: {error}");
            }
            let pool = tauri::async_runtime::block_on(db::open(&dir))?;
            let environment = Environment {
                backend_url: backend::URL.to_owned(),
                ggg_url: ggg::API_URL.to_owned(),
                ninja_url: ninja::API_URL.to_owned(),
                github_raw_url: github::RAW_URL.to_owned(),
                clock: Arc::new(clock::SystemClock),
                jitter: Arc::new(scheduler::OsJitter),
                desktop: Arc::new(desktop::Plugins(app.handle().clone())),
                legacy_folders: legacy_import::folders::legacy_folders(
                    legacy_import::folders::Os::CURRENT,
                    &legacy_import::folders::AppDataEnv::from_process(),
                ),
            };
            manage(app, pool, environment)?;
            app.handle().plugin(window_state(&dir)?)?;
            // Built here, not from the config at launch: the migrations run
            // before any window exists.
            let main = app
                .config()
                .app
                .windows
                .iter()
                .find(|w| w.label == shell::MAIN_WINDOW)
                .ok_or("tauri.conf.json has no main window")?;
            let settings = app.state::<state::Store>().read(Settings::clone);
            let window = WebviewWindowBuilder::from_config(app, main)?;
            shell::build_window(window, settings.hardware_acceleration)?;
            shell::apply_ui_scale(app.handle());
            tray::create(app.handle())?;
            deep_links(app.handle())?;
            if app.state::<state::Store>().read(|s: &Session| s.login) == LoginState::SignedIn {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move { session_start::run(&handle).await });
            }
            updater::start(app.handle());
            scheduler::start(app.handle());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building the Tauri application")
        .run(|app, event| shell::on_run_event(app, &event));
}

/// Hands every `exilence-ce-ng://` link to the login (F1, D20): one that
/// started the app, and later ones, from a second instance (Windows, Linux)
/// or the OS (macOS).
fn deep_links(app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let deep_link = app.deep_link();
    // Bundles register the scheme on install; this also covers an AppImage
    // or a development build. macOS registers through the bundle only, a
    // distro package through its desktop entry (`packaging/linux/`).
    #[cfg(target_os = "linux")]
    if let Err(error) = install_menu_entry(app) {
        log::warn!("cannot write the menu entry: {error}");
    }
    #[cfg(any(windows, target_os = "linux"))]
    if !DISTRO_PACKAGE && let Err(error) = deep_link.register_all() {
        log::warn!(
            "cannot register the {} scheme: {error}",
            auth::DEEP_LINK_SCHEME
        );
    }
    let handle = app.clone();
    deep_link.on_open_url(move |event| {
        for url in event.urls() {
            auth::open_deep_link(&handle, url.as_str());
        }
    });
    for url in deep_link.get_current()?.unwrap_or_default() {
        auth::open_deep_link(app, url.as_str());
    }
    Ok(())
}

/// The AppImage's visible menu entry, written before the deep-link plugin
/// registers the same file ([`menu_entry`]). Not for a development build.
#[cfg(target_os = "linux")]
fn install_menu_entry(app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let Some(appimage) = app.env().appimage else {
        return Ok(());
    };
    let exe = tauri::utils::platform::current_exe()?;
    let binary = exe
        .file_name()
        .ok_or("the executable has no file name")?
        .to_string_lossy();
    menu_entry::install(
        &app.path().data_dir()?,
        std::path::Path::new(&appimage),
        &binary,
    )?;
    Ok(())
}
