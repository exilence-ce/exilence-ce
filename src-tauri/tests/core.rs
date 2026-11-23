//! Core integration tests: command handlers on Tauri's mock runtime with a
//! real database in a temp directory (parity verification, layers).

// Test support code outside `#[test]` functions: a panic is the failure.
#![allow(clippy::unwrap_used, clippy::expect_used)]

#[path = "core/auto_snapshot.rs"]
mod auto_snapshot;
#[path = "core/charts.rs"]
mod charts;
#[path = "core/custom_prices.rs"]
mod custom_prices;
#[path = "core/diagnostics.rs"]
mod diagnostics;
#[path = "core/fake_backend.rs"]
mod fake_backend;
#[path = "core/fake_ggg.rs"]
mod fake_ggg;
#[path = "core/group.rs"]
mod group;
#[path = "core/group_resilience.rs"]
mod group_resilience;
#[path = "core/items.rs"]
mod items;
#[path = "core/legacy_import.rs"]
mod legacy_import;
#[path = "core/login.rs"]
mod login;
#[path = "core/onboarding.rs"]
mod onboarding;
#[path = "core/overlay.rs"]
mod overlay;
#[path = "core/prices.rs"]
mod prices;
#[path = "core/profiles.rs"]
mod profiles;
#[path = "core/rate_limits.rs"]
mod rate_limits;
#[path = "core/session_start.rs"]
mod session_start;
#[path = "core/settings_page.rs"]
mod settings_page;
#[path = "core/sign_out.rs"]
mod sign_out;
#[path = "core/snapshot.rs"]
mod snapshot;
#[path = "core/snapshot_removal.rs"]
mod snapshot_removal;

use std::cell::Cell;
use std::rc::Rc;
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::Duration;

use exilence_ce_ng_lib::desktop::{Desktop, SavePath};
use exilence_ce_ng_lib::legacy_import::folders::LegacyFolder;
use exilence_ce_ng_lib::session_start::Characters;
use exilence_ce_ng_lib::shell::MAIN_WINDOW;
use exilence_ce_ng_lib::{
    Environment, db, links, logging, manage, settings, shell, state, state::Store, with_commands,
};
use fake_ggg::{FakeClock, FakeJitter};
use ipc::command::{
    Command, GetState, GetStateArgs, InstallUpdate, MarkNotificationsRead, NoArgs, OpenLink,
    OpenLinkArgs, SettingsPatch, UpdateSettings, UpdateSettingsArgs,
};
use ipc::slices::{
    ExitAction, Notification, NotificationKind, Notifications, ReleaseChannel, Session, Settings,
    Update, UpdateState,
};
use ipc::{Event, IpcError, Message, MessageKey, SliceName};
use sqlx::SqlitePool;
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{
    INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder, mock_context, noop_assets,
};
use tauri::webview::InvokeRequest;
use tauri::{App, Manager, RunEvent, WebviewWindow, WebviewWindowBuilder};
use tempfile::TempDir;

/// The tables of `client-schema.sql`.
const CLIENT_SCHEMA_TABLES: &[&str] = &[
    "accounts",
    "custom_prices",
    "item_definitions",
    "legacy_import",
    "login",
    "pending_removals",
    "profiles",
    "rate_limit_policies",
    "settings",
    "snapshot_tabs",
    "snapshots",
];

/// How long a test waits for an event or a condition.
const WAIT: Duration = Duration::from_secs(10);

/// Waits until `check` finds something; a timeout names the caller's line.
#[track_caller]
fn poll<T>(check: impl Fn() -> Option<T>) -> T {
    let deadline = std::time::Instant::now() + WAIT;
    loop {
        if let Some(found) = check() {
            return found;
        }
        assert!(std::time::Instant::now() < deadline, "timed out");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The next event, which must be a toast.
fn next_toast(toasts: &mpsc::Receiver<Event>) -> ipc::slices::Toast {
    match toasts.recv_timeout(WAIT).expect("a toast") {
        Event::Toast(toast) => toast,
        event => panic!("not a toast: {event:?}"),
    }
}

/// How many requests for `url_path` `server` has received.
fn received(server: &wiremock::MockServer, url_path: &str) -> usize {
    tauri::async_runtime::block_on(server.received_requests())
        .unwrap()
        .iter()
        .filter(|r| r.url.path() == url_path)
        .count()
}

/// Where every test of this process logs (F20): the log is process-wide.
fn log_dir() -> &'static std::path::Path {
    static DIR: OnceLock<TempDir> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = tempfile::tempdir().unwrap();
        logging::init(dir.path()).unwrap();
        dir
    })
    .path()
}

/// A `wiremock` server for this test alone, on a port no earlier server of
/// this process had. An earlier test's background work outlives it (its
/// app's tasks run on the shared runtime) and may still call that test's
/// GGG or poe.ninja: `MockServer::start` would hand it a pooled server a
/// later test now uses, and a port the OS hands out again would reach a
/// later test's server; either eats that test's one-time answers.
async fn mock_server() -> wiremock::MockServer {
    /// Below the ephemeral ranges of Linux (from 32768), macOS and Windows
    /// (from 49152), so the OS never hands these ports out by itself.
    const PORTS: std::ops::Range<u32> = 20_000..32_000;
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    // Each process starts elsewhere, so parallel test processes rarely meet.
    let start = std::process::id().wrapping_mul(7919);
    let listener = loop {
        let n = start.wrapping_add(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
        let port = PORTS.start + n % PORTS.len() as u32;
        if let Ok(listener) = std::net::TcpListener::bind(("127.0.0.1", port as u16)) {
            break listener;
        }
    };
    wiremock::MockServer::builder()
        .listener(listener)
        .start()
        .await
}

/// No backend listens here.
const NO_BACKEND: &str = "http://127.0.0.1:1";
/// No GGG listens here.
const NO_GGG: &str = "http://127.0.0.1:1";
/// No poe.ninja listens here.
const NO_NINJA: &str = "http://127.0.0.1:1";
/// No GitHub listens here.
const NO_GITHUB: &str = "http://127.0.0.1:1";

/// The desktop as the core sees it: records instead of opening a browser,
/// writing the clipboard or opening a folder; the save dialog answers with
/// [`FakeDesktop::save_to`].
#[derive(Default)]
struct FakeDesktop {
    opened: Mutex<Vec<String>>,
    copied: Mutex<Vec<String>>,
    opened_dirs: Mutex<Vec<std::path::PathBuf>>,
    /// Where the user saves; `None` cancels the dialog.
    save_to: Mutex<Option<std::path::PathBuf>>,
    /// The file names the save dialog suggested.
    suggested: Mutex<Vec<String>>,
}

impl Desktop for FakeDesktop {
    fn open_url(&self, url: &str) -> Result<(), String> {
        self.opened.lock().unwrap().push(url.to_owned());
        Ok(())
    }

    fn copy_text(&self, text: &str) -> Result<(), String> {
        self.copied.lock().unwrap().push(text.to_owned());
        Ok(())
    }

    fn open_dir(&self, dir: &std::path::Path) -> Result<(), String> {
        self.opened_dirs.lock().unwrap().push(dir.to_owned());
        Ok(())
    }

    fn save_path(&self, file_name: String) -> SavePath {
        self.suggested.lock().unwrap().push(file_name);
        let path = self.save_to.lock().unwrap().clone();
        Box::pin(async move { Ok(path) })
    }
}

/// What a harness's app talks to besides the desktop.
#[derive(Clone)]
struct Fakes {
    backend_url: String,
    ggg_url: String,
    ninja_url: String,
    github_raw_url: String,
    clock: Arc<FakeClock>,
    jitter: Arc<FakeJitter>,
    /// The legacy folders the legacy import scans (F21); none by default,
    /// so no test reads this machine's.
    legacy_folders: Vec<LegacyFolder>,
}

impl Default for Fakes {
    fn default() -> Self {
        Self {
            backend_url: NO_BACKEND.to_owned(),
            ggg_url: NO_GGG.to_owned(),
            ninja_url: NO_NINJA.to_owned(),
            github_raw_url: NO_GITHUB.to_owned(),
            clock: FakeClock::new(),
            jitter: FakeJitter::new(),
            legacy_folders: Vec::new(),
        }
    }
}

/// The app as `run` builds it, on the mock runtime, with its database in a
/// temp directory, a fake desktop, and the main window.
struct Harness {
    app: App<MockRuntime>,
    window: WebviewWindow<MockRuntime>,
    dir: TempDir,
    desktop: Arc<FakeDesktop>,
    fakes: Fakes,
}

impl Harness {
    fn new() -> Self {
        Self::with(Fakes::default())
    }

    /// With the backend at `url`.
    fn with_backend(url: &str) -> Self {
        Self::with(Fakes {
            backend_url: url.to_owned(),
            ..Fakes::default()
        })
    }

    fn with(fakes: Fakes) -> Self {
        Self::open(tempfile::tempdir().unwrap(), fakes)
    }

    /// The app on the database in `dir`, as at a start.
    fn open(dir: TempDir, fakes: Fakes) -> Self {
        log_dir();
        let app = with_commands(mock_builder())
            .build(mock_context(noop_assets()))
            .unwrap();
        let pool = tauri::async_runtime::block_on(db::open(dir.path())).unwrap();
        let desktop = Arc::new(FakeDesktop::default());
        let environment = Environment {
            backend_url: fakes.backend_url.clone(),
            ggg_url: fakes.ggg_url.clone(),
            ninja_url: fakes.ninja_url.clone(),
            github_raw_url: fakes.github_raw_url.clone(),
            clock: fakes.clock.clone(),
            jitter: fakes.jitter.clone(),
            desktop: desktop.clone(),
            legacy_folders: fakes.legacy_folders.clone(),
        };
        manage(&app, pool, environment).unwrap();
        let window = WebviewWindowBuilder::new(&app, MAIN_WINDOW, Default::default())
            .build()
            .unwrap();
        Self {
            app,
            window,
            dir,
            desktop,
            fakes,
        }
    }

    /// Quits cleanly and starts again on the same database, with the same
    /// fakes.
    fn restart(self) -> Self {
        shell::on_run_event(self.app.handle(), &RunEvent::Exit);
        self.crash()
    }

    /// Ends without a clean exit, as a crash does, and starts again on the
    /// same database, with the same fakes.
    fn crash(self) -> Self {
        let Self {
            app,
            window,
            dir,
            fakes,
            ..
        } = self;
        drop(window);
        drop(app);
        Self::open(dir, fakes)
    }

    /// Runs command `C` as the webview would.
    fn invoke<C: Command>(&self, args: &C::Args) -> Result<C::Output, IpcError> {
        let request = InvokeRequest {
            cmd: C::NAME.into(),
            callback: CallbackFn(0),
            error: CallbackFn(1),
            // The window's own origin: `tauri://localhost`, but
            // `http://tauri.localhost` on Windows.
            url: self.window.url().unwrap(),
            body: InvokeBody::Json(serde_json::to_value(args).unwrap()),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_owned(),
        };
        get_ipc_response(&self.window, request)
            .map(|body| body.deserialize().unwrap())
            .map_err(|error| serde_json::from_value(error).unwrap())
    }

    /// Collects the events emitted under `name` from now on.
    fn events(&self, name: &str) -> mpsc::Receiver<Event> {
        let (tx, rx) = mpsc::channel();
        state::listen(&self.app, name, move |event| {
            tx.send(serde_json::from_str(event.payload()).unwrap())
                .unwrap();
        });
        rx
    }

    fn pool(&self) -> SqlitePool {
        self.app.state::<SqlitePool>().inner().clone()
    }

    /// The characters' leagues, once the session start's GGG part loaded
    /// them.
    #[track_caller]
    fn wait_for_characters(&self) -> Vec<String> {
        poll(|| Some(self.app.state::<Characters>().leagues()).filter(|l| !l.is_empty()))
    }

    /// Waits until the session start's GGG part has loaded the characters,
    /// the stash tab list and the prices: the session is ready (F3). What
    /// it does after that starts nothing a test waits for.
    #[track_caller]
    fn wait_for_ready(&self) {
        poll(|| {
            self.app
                .state::<Store>()
                .read(|s: &Session| s.ready)
                .then_some(())
        });
    }

    /// With a stored Login, as at a launch: see [`WithLogin`].
    fn with_login() -> WithLogin {
        WithLogin {
            fakes: Fakes::default(),
            ggg_expires_at: fake_backend::ACCESS_TOKEN_EXPIRES_AT,
            profiles: None,
        }
    }
}

/// Builds a [`Harness`] signed in as at a launch: a stored Login of the
/// fake backend's account, whose GGG token expires at
/// [`fake_backend::ACCESS_TOKEN_EXPIRES_AT`]. The backend, GGG and
/// poe.ninja are unreachable, and the account has no Profile, unless set.
#[must_use]
struct WithLogin {
    fakes: Fakes,
    ggg_expires_at: i64,
    /// The Profiles and the active one's id.
    profiles: Option<(Vec<wire::v1::Profile>, String)>,
}

impl WithLogin {
    fn backend(mut self, url: &str) -> Self {
        url.clone_into(&mut self.fakes.backend_url);
        self
    }

    fn ggg(mut self, url: &str) -> Self {
        url.clone_into(&mut self.fakes.ggg_url);
        self
    }

    fn ninja(mut self, url: &str) -> Self {
        url.clone_into(&mut self.fakes.ninja_url);
        self
    }

    fn clock(mut self, clock: Arc<FakeClock>) -> Self {
        self.fakes.clock = clock;
        self
    }

    fn legacy_folders(mut self, folders: Vec<LegacyFolder>) -> Self {
        self.fakes.legacy_folders = folders;
        self
    }

    fn ggg_expires_at(mut self, at: i64) -> Self {
        self.ggg_expires_at = at;
        self
    }

    /// `profiles` with `active` the active one, stored as a `StartSession`
    /// would and pushed.
    fn profiles(mut self, profiles: &[wire::v1::Profile], active: &str) -> Self {
        self.profiles = Some((profiles.to_vec(), active.to_owned()));
        self
    }

    fn build(self) -> Harness {
        let dir = tempfile::tempdir().unwrap();
        tauri::async_runtime::block_on(async {
            let pool = db::open(dir.path()).await?;
            sqlx::query("INSERT INTO accounts (id, name) VALUES (?1, ?2)")
                .bind(fake_backend::ACCOUNT_ID)
                .bind(fake_backend::ACCOUNT_NAME)
                .execute(&pool)
                .await?;
            sqlx::query(
                "INSERT INTO login \
                 (id, account_id, session_token, ggg_access_token, ggg_expires_at) \
                 VALUES (1, ?1, ?2, ?3, ?4)",
            )
            .bind(fake_backend::ACCOUNT_ID)
            .bind(fake_backend::SESSION_TOKEN)
            .bind(fake_backend::ACCESS_TOKEN)
            .bind(self.ggg_expires_at)
            .execute(&pool)
            .await?;
            pool.close().await;
            Ok::<_, Box<dyn std::error::Error>>(())
        })
        .unwrap();
        let harness = Harness::open(dir, self.fakes);
        if let Some((stored, active)) = &self.profiles {
            tauri::async_runtime::block_on(async {
                exilence_ce_ng_lib::profiles::replace(
                    &harness.pool(),
                    fake_backend::ACCOUNT_ID,
                    stored,
                    active,
                )
                .await
                .unwrap();
                exilence_ce_ng_lib::profiles::push(harness.app.handle()).await;
            });
        }
        harness
    }
}

#[test]
fn command_pushes_its_slice_whole() {
    let harness = Harness::new();
    let unread = Notification {
        id: 1,
        at: 1_700_000_000_000,
        kind: NotificationKind::Info,
        message: Message::new(MessageKey::ErrorOffline),
        description: None,
    };
    harness
        .app
        .state::<Store>()
        .update::<Notifications, _>(harness.app.handle(), |n| {
            n.newest.push(unread.clone());
            n.unread = 1;
        })
        .unwrap();
    let events = harness.events(SliceName::Notifications.event_name());

    harness.invoke::<MarkNotificationsRead>(&NoArgs {}).unwrap();

    let expected = Notifications {
        newest: vec![unread],
        unread: 0,
    };
    let event = events.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(event, Event::Notifications(expected.clone()));
    let state = harness
        .invoke::<GetState>(&GetStateArgs {
            slices: vec![SliceName::Notifications, SliceName::Session],
        })
        .unwrap();
    assert_eq!(
        state,
        [
            Event::Notifications(expected),
            Event::Session(Default::default())
        ]
    );
}

#[test]
fn f18_notification_times_come_from_the_clock() {
    let harness = Harness::new();
    let notify = || {
        state::push(
            harness.app.handle(),
            NotificationKind::Info,
            Message::new(MessageKey::ErrorOffline),
            None,
        )
    };
    notify();
    harness.fakes.clock.advance(1_500);
    notify();
    let times: Vec<i64> = harness
        .app
        .state::<Store>()
        .read(|n: &Notifications| n.newest.iter().map(|n| n.at).collect());
    assert_eq!(times, [fake_ggg::START + 1_500, fake_ggg::START]);
}

#[test]
fn get_state_returns_every_slice_in_order() {
    let harness = Harness::new();
    let state = harness
        .invoke::<GetState>(&GetStateArgs {
            slices: SliceName::ALL.to_vec(),
        })
        .unwrap();
    let names: Vec<_> = state.iter().map(Event::name).collect();
    let expected: Vec<_> = SliceName::ALL.iter().map(|s| s.event_name()).collect();
    assert_eq!(names, expected);
}

#[test]
fn database_holds_every_table_of_the_client_schema() {
    let harness = Harness::new();
    let pool = harness.pool();
    let tables: Vec<String> = tauri::async_runtime::block_on(
        sqlx::query_scalar(
            "SELECT name FROM sqlite_schema WHERE type = 'table' \
             AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' \
             AND name NOT LIKE '\\_sqlx\\_%' ESCAPE '\\' ORDER BY name",
        )
        .fetch_all(&pool),
    )
    .unwrap();
    assert_eq!(tables, CLIENT_SCHEMA_TABLES);

    // The migration is the schema: it creates exactly these tables.
    let migration = include_str!("../migrations/0001_initial.sql");
    let mut created: Vec<&str> = migration
        .lines()
        .filter_map(|line| line.strip_prefix("CREATE TABLE "))
        .filter_map(|rest| rest.split_whitespace().next())
        .collect();
    created.sort_unstable();
    assert_eq!(created, CLIENT_SCHEMA_TABLES);

    let (foreign_keys, journal_mode): (i64, String) = tauri::async_runtime::block_on(async {
        let foreign_keys = sqlx::query_scalar("PRAGMA foreign_keys")
            .fetch_one(&pool)
            .await
            .unwrap();
        let journal_mode = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&pool)
            .await
            .unwrap();
        (foreign_keys, journal_mode)
    });
    assert_eq!((foreign_keys, journal_mode.as_str()), (1, "wal"));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(harness.dir.path().join(db::FILE_NAME))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

#[test]
fn database_of_a_newer_version_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    tauri::async_runtime::block_on(async {
        let pool = db::open(dir.path()).await.unwrap();
        sqlx::query(
            "INSERT INTO _sqlx_migrations \
             (version, description, success, checksum, execution_time) \
             VALUES (9999, 'from the future', 1, x'00', 0)",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;
        let error = db::open(dir.path()).await.unwrap_err();
        assert!(
            matches!(error, db::OpenError::NewerVersion { migration: 9999 }),
            "{error}"
        );
    });
}

fn update_settings(harness: &Harness, patch: SettingsPatch) {
    harness
        .invoke::<UpdateSettings>(&UpdateSettingsArgs { patch })
        .unwrap();
}

fn settings_slice(harness: &Harness) -> Settings {
    harness.app.state::<Store>().read(Settings::clone)
}

#[test]
fn f15_exit_action_and_nav_drawer_are_stored_and_survive_a_restart() {
    let harness = Harness::new();
    assert_eq!(settings_slice(&harness), Settings::default());
    let events = harness.events(SliceName::Settings.event_name());

    update_settings(
        &harness,
        SettingsPatch {
            nav_drawer_open: Some(true),
            ..SettingsPatch::default()
        },
    );
    update_settings(
        &harness,
        SettingsPatch {
            exit_action: Some(ExitAction::Exit),
            ..SettingsPatch::default()
        },
    );

    let expected = Settings {
        exit_action: ExitAction::Exit,
        nav_drawer_open: true,
        ..Settings::default()
    };
    events.recv_timeout(Duration::from_secs(5)).unwrap();
    let event = events.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(event, Event::Settings(expected.clone()));
    let rows: Vec<(String, String)> = tauri::async_runtime::block_on(
        sqlx::query_as("SELECT key, value FROM settings ORDER BY key").fetch_all(&harness.pool()),
    )
    .unwrap();
    assert_eq!(
        rows,
        [
            // The crash marker while running (F20).
            ("clean_exit".to_owned(), "false".to_owned()),
            ("exit_action".to_owned(), r#""exit""#.to_owned()),
            ("nav_drawer_open".to_owned(), "true".to_owned()),
        ]
    );

    let harness = harness.restart();
    assert_eq!(settings_slice(&harness), expected);
}

fn update_slice(harness: &Harness) -> Update {
    harness.app.state::<Store>().read(Update::clone)
}

#[test]
fn f16_release_channel_change_takes_effect_after_a_restart() {
    let harness = Harness::new();
    assert_eq!(update_slice(&harness).channel, ReleaseChannel::Latest);

    update_settings(
        &harness,
        SettingsPatch {
            release_channel: Some(ReleaseChannel::Beta),
            ..SettingsPatch::default()
        },
    );
    // Stored and shown in the setting, but this run keeps checking Latest
    // Stable.
    assert_eq!(
        settings_slice(&harness).release_channel,
        ReleaseChannel::Beta
    );
    assert_eq!(update_slice(&harness).channel, ReleaseChannel::Latest);

    let harness = harness.restart();
    assert_eq!(
        update_slice(&harness),
        Update {
            channel: ReleaseChannel::Beta,
            state: UpdateState::None,
        }
    );
}

#[test]
fn f16_install_update_without_a_downloaded_update_fails() {
    let harness = Harness::new();
    let error = harness.invoke::<InstallUpdate>(&NoArgs {}).unwrap_err();
    assert!(matches!(error, IpcError::Internal { .. }), "{error:?}");
}

#[test]
fn f17_stored_setting_that_does_not_fit_keeps_its_default() {
    let dir = tempfile::tempdir().unwrap();
    let loaded = tauri::async_runtime::block_on(async {
        let pool = db::open(dir.path()).await.unwrap();
        sqlx::query(
            "INSERT INTO settings (key, value) VALUES \
             ('exit_action', '\"close\"'), ('nav_drawer_open', 'true'), ('clean_exit', 'false')",
        )
        .execute(&pool)
        .await
        .unwrap();
        settings::load(&pool).await.unwrap()
    });
    assert_eq!(
        loaded,
        Settings {
            nav_drawer_open: true,
            ..Settings::default()
        }
    );
}

/// Closes the main window from the running app, as its close button does,
/// and runs the app until it has handled that; whether the main window
/// survived.
fn main_window_survives_close(exit_action: ExitAction) -> bool {
    let harness = Harness::new();
    update_settings(
        &harness,
        SettingsPatch {
            exit_action: Some(exit_action),
            ..SettingsPatch::default()
        },
    );
    let survived = Rc::new(Cell::new(false));
    let seen = Rc::clone(&survived);
    let Harness { app, .. } = harness;
    // The close request is handled in the first iteration; with "exit" the
    // last window is gone and the app quits before the iteration ends.
    // Otherwise the window is destroyed, which quits in the next one.
    app.run_return(move |handle, event| {
        shell::on_run_event(handle, &event);
        match event {
            RunEvent::Ready => handle
                .get_webview_window(MAIN_WINDOW)
                .unwrap()
                .close()
                .unwrap(),
            RunEvent::MainEventsCleared => {
                if let Some(window) = handle.get_webview_window(MAIN_WINDOW) {
                    seen.set(true);
                    window.destroy().unwrap();
                }
            }
            _ => {}
        }
    });
    survived.get()
}

#[test]
fn f15_closing_the_main_window_hides_it_to_the_tray_by_default() {
    assert!(main_window_survives_close(ExitAction::MinimizeToTray));
}

#[test]
fn f15_closing_the_main_window_quits_when_the_exit_action_is_exit() {
    assert!(!main_window_survives_close(ExitAction::Exit));
}

#[test]
fn f15_open_link_opens_only_allowlisted_https_hosts() {
    let harness = Harness::new();
    for target in [
        "http://ko-fi.com/ExilenceCE",
        "https://example.com/",
        "https://ko-fi.com.example.com/",
        "https://ko-fi.com@example.com/",
        "https://user@ko-fi.com/",
        "https://ko-fi.com:8443/",
        "file:///etc/passwd",
        "javascript:alert(1)",
        "ko-fi.com",
    ] {
        let error = harness
            .invoke::<OpenLink>(&OpenLinkArgs {
                target: target.to_owned(),
            })
            .unwrap_err();
        assert!(matches!(error, IpcError::Internal { .. }), "{target}");
        assert!(links::allowed(target).is_none(), "{target}");
    }
    for target in [ipc::links::KOFI, ipc::links::DISCORD, "https://KO-FI.com/x"] {
        assert!(links::allowed(target).is_some(), "{target}");
    }
}
