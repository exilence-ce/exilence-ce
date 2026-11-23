//! Reset data (F17, `client-schema.sql`): cancels a running Snapshot, signs
//! out (best effort), leaves a marker and restarts; the next start deletes all
//! Exilence CE NG local data before the database or any webview opens, so no
//! file is still in use. Logs stay (F20); legacy folders are never touched.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ipc::IpcError;
use sqlx::SqlitePool;
use tauri::path::PathResolver;
use tauri::{AppHandle, Manager, Runtime};

use crate::legacy_import::folders::{AppDataEnv, Os, legacy_folders};
use crate::{auth, internal, slot};

/// The marker in the app data directory: the next start deletes the data.
pub const PENDING_FILE: &str = ".reset-data";

/// What the marker asks the next start to do. An empty marker (older builds)
/// is the default: delete everything, waiting for nothing.
#[derive(Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
struct Marker {
    /// The webview's browser process when Reset data restarted the app.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    browser_process: Option<u32>,
    /// `None`: everything. Otherwise only these paths, left by the delete of
    /// an earlier start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    left: Option<Vec<PathBuf>>,
}

impl Marker {
    /// The marker in `data_dir`, if there is one.
    fn read(data_dir: &Path) -> io::Result<Option<Self>> {
        let bytes = match fs::read(data_dir.join(PENDING_FILE)) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        if bytes.is_empty() {
            return Ok(Some(Self::default()));
        }
        Ok(Some(serde_json::from_slice(&bytes).unwrap_or_else(
            |error| {
                log::warn!("unreadable Reset data marker, deleting everything: {error}");
                Self::default()
            },
        )))
    }

    /// Writes the marker to `data_dir` in one step: a marker cut short would
    /// ask for everything.
    fn write(&self, data_dir: &Path) -> io::Result<()> {
        let partial = data_dir.join(format!("{PENDING_FILE}.partial"));
        fs::write(&partial, serde_json::to_vec(self)?)?;
        fs::rename(&partial, data_dir.join(PENDING_FILE))
    }
}

/// Whether process `pid` is gone (or cannot be found) within `bound`.
#[cfg(unix)]
fn exited_within(pid: u32, bound: Duration) -> bool {
    // 0 and negative IDs name process groups.
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return true;
    };
    if pid == 0 {
        return true;
    }
    let start = std::time::Instant::now();
    loop {
        // SAFETY: signal 0 sends nothing; it only checks that `pid` exists.
        if unsafe { libc::kill(pid, 0) } == -1
            && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
        {
            return true;
        }
        if start.elapsed() >= bound {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Whether process `pid` is gone (or cannot be found) within `bound`.
#[cfg(windows)]
fn exited_within(pid: u32, bound: Duration) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
    };
    // SAFETY: no pointers; a null handle means no such process.
    let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    if process.is_null() {
        return true;
    }
    // Below `INFINITE` (`u32::MAX`).
    let millis = u32::try_from(bound.as_millis()).unwrap_or(u32::MAX - 1);
    // SAFETY: `process` is an open handle, closed right after.
    let waited = unsafe { WaitForSingleObject(process, millis) };
    // SAFETY: as above, closed once.
    unsafe { CloseHandle(process) };
    waited != WAIT_TIMEOUT
}

/// `reset_data` up to the restart: takes the operation slot from a running
/// Snapshot, stops background work, has the backend sign this Login out (best
/// effort, D19), marks `data_dir` for deletion, naming the webview's browser
/// process for the next start to wait for, and closes the database.
pub async fn prepare<R: Runtime>(app: &AppHandle<R>, data_dir: &Path) -> Result<(), IpcError> {
    let _held = slot::cancel_and_hold(app).await;
    log::info!("resetting data");
    auth::stop_background_work(app);
    if let Err(error) = auth::sign_out_at_backend(app).await {
        log::warn!("cannot sign out before resetting data: {error}");
    }
    let marker = Marker {
        browser_process: browser_process(app).await,
        left: None,
    };
    marker.write(data_dir).map_err(internal)?;
    app.state::<SqlitePool>().close().await;
    Ok(())
}

/// The ID of WebView2's browser process, which all of the app's webviews
/// share and which may hold the webview data for a while after the app
/// exited. `None` if it cannot be read in time.
#[cfg(windows)]
async fn browser_process<R: Runtime>(app: &AppHandle<R>) -> Option<u32> {
    let window = app.webview_windows().into_values().next()?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let asked = window.with_webview(move |webview| {
        let mut pid = 0;
        // SAFETY: COM calls on the webview's own thread; `pid` outlives them.
        let read = unsafe {
            webview
                .controller()
                .CoreWebView2()
                .and_then(|core| core.BrowserProcessId(&mut pid))
        };
        // The receiver may have stopped waiting.
        let _ = sender.send(read.map(|()| pid));
    });
    if let Err(error) = asked {
        log::warn!("cannot ask for the webview's browser process: {error}");
        return None;
    }
    match tokio::time::timeout(Duration::from_secs(1), receiver).await {
        Ok(Ok(Ok(pid))) => Some(pid),
        Ok(Ok(Err(error))) => {
            log::warn!("cannot read the webview's browser process: {error}");
            None
        }
        // The test runtime never calls back.
        Ok(Err(_)) | Err(_) => None,
    }
}

/// Only WebView2 (Windows) is known to outlive the app.
#[cfg(not(windows))]
async fn browser_process<R: Runtime>(_app: &AppHandle<R>) -> Option<u32> {
    None
}

/// Where the app keeps its data, and what in there stays.
#[derive(Clone, Debug)]
pub struct Locations {
    /// Every directory named after the app: data, local data (with the
    /// webview data), config and cache.
    pub roots: Vec<PathBuf>,
    /// Never deleted, nor anything inside: the log directory and the legacy
    /// folders.
    pub keep: Vec<PathBuf>,
}

impl Locations {
    /// This app's locations.
    pub fn of<R: Runtime>(paths: &PathResolver<R>) -> tauri::Result<Self> {
        let mut roots = vec![
            paths.app_data_dir()?,
            paths.app_local_data_dir()?,
            paths.app_config_dir()?,
            paths.app_cache_dir()?,
        ];
        roots.sort();
        roots.dedup();
        let mut keep = vec![paths.app_log_dir()?];
        keep.extend(
            legacy_folders(Os::CURRENT, &AppDataEnv::from_process())
                .into_iter()
                .map(|f| f.path),
        );
        Ok(Self { roots, keep })
    }

    /// Deletes everything under the roots except what is kept. Goes on past
    /// a failure.
    fn delete(&self) -> Failures {
        let mut failures = Failures::default();
        for root in &self.roots {
            if self.kept(root) {
                continue;
            }
            self.clear(root, &mut failures);
        }
        failures
    }

    /// Deletes `left`, what the delete of an earlier start left, and nothing
    /// else: data written since then stays. Skips a path outside the roots
    /// or kept.
    fn delete_left(&self, left: &[PathBuf]) -> Failures {
        let mut failures = Failures::default();
        for path in left {
            if !self.roots.iter().any(|root| path.starts_with(root)) || self.kept(path) {
                log::warn!("Reset data does not delete {}", path.display());
                continue;
            }
            let file_type = match fs::symlink_metadata(path) {
                Ok(metadata) => Ok(metadata.file_type()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => Err(error),
            };
            if let Err(error) = remove(path, file_type) {
                failures.add_left(path, error);
            }
        }
        failures
    }

    /// Whether `path` is kept or inside a kept directory.
    fn kept(&self, path: &Path) -> bool {
        self.keep.iter().any(|k| path.starts_with(k))
    }

    fn clear(&self, dir: &Path, failures: &mut Failures) {
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return,
            Err(error) => return failures.add(dir, error),
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    failures.add(dir, error);
                    continue;
                }
            };
            let path = entry.path();
            if self.kept(&path) {
                continue;
            }
            if self.keep.iter().any(|k| k.starts_with(&path)) {
                self.clear(&path, failures);
                continue;
            }
            if let Err(error) = remove(&path, entry.file_type()) {
                failures.add_left(&path, error);
            }
        }
    }
}

/// Removes `path`, a directory with all inside; a symlink is removed, never
/// followed.
fn remove(path: &Path, file_type: io::Result<fs::FileType>) -> io::Result<()> {
    if file_type?.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

/// What a delete could not remove, each failure logged.
#[derive(Default)]
struct Failures {
    /// Entries still there, which a later start deletes. A directory that
    /// cannot be listed is not among them: what it holds may be newer.
    left: Vec<PathBuf>,
    /// The first failure, naming its path.
    first: Option<io::Error>,
}

impl Failures {
    /// `error` deleting `path`.
    fn add(&mut self, path: &Path, error: io::Error) {
        let error = cannot_delete(path, error);
        log::error!("{error}");
        self.first.get_or_insert(error);
    }

    /// `error` deleting `path`, which stays for a later start.
    fn add_left(&mut self, path: &Path, error: io::Error) {
        self.add(path, error);
        self.left.push(path.to_owned());
    }
}

/// `error` deleting `path`, naming it.
fn cannot_delete(path: &Path, error: io::Error) -> io::Error {
    io::Error::new(
        error.kind(),
        format!("cannot delete {}: {error}", path.display()),
    )
}

/// How long the next start waits for the webview's browser process to exit:
/// WebView2's may hold the webview data for a while after the app closed.
/// Bounded, so a start never hangs, also when the ID now names another
/// process.
const BROWSER_EXIT_WAIT: Duration = Duration::from_secs(5);

/// At start, before the database opens: if Reset data left its marker in
/// `data_dir`, waits for the webview's browser process it names, then
/// deletes the data of `locations`. What cannot be deleted is left to the
/// next start, recorded in the marker; the marker goes once nothing is left.
pub fn finish_pending(data_dir: &Path, locations: &Locations) -> io::Result<()> {
    finish_pending_within(data_dir, locations, BROWSER_EXIT_WAIT)
}

fn finish_pending_within(
    data_dir: &Path,
    locations: &Locations,
    browser_exit: Duration,
) -> io::Result<()> {
    let Some(pending) = Marker::read(data_dir)? else {
        return Ok(());
    };
    let marker = data_dir.join(PENDING_FILE);
    if let Some(pid) = pending.browser_process {
        log::info!("waiting for webview browser process {pid} to exit");
        if exited_within(pid, browser_exit) {
            log::info!("webview browser process {pid} exited");
        } else {
            log::warn!("webview browser process {pid} still runs after {browser_exit:?}");
        }
    }
    let mut locations = locations.clone();
    locations.keep.push(marker.clone());
    let mut failures = match &pending.left {
        None => {
            log::info!("deleting local data (Reset data)");
            locations.delete()
        }
        Some(left) => {
            log::info!("deleting what Reset data left");
            locations.delete_left(left)
        }
    };
    let first = failures.first.take().map_or(Ok(()), Err);
    if !failures.left.is_empty() {
        let retry = Marker {
            browser_process: None,
            left: Some(failures.left),
        };
        match retry.write(data_dir) {
            Ok(()) => return first,
            // The marker as it is would delete newer data too: what is
            // left stays instead.
            Err(error) => log::error!("cannot record what Reset data left: {error}"),
        }
    }
    let removed = fs::remove_file(&marker).map_err(|error| cannot_delete(&marker, error));
    first.and(removed)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::process::{Child, Command, Stdio};
    use std::thread;
    use std::time::Instant;

    use super::*;

    /// A process that runs until the test ends it, standing in for the
    /// webview's browser process.
    fn long_running() -> Child {
        let (program, args): (&str, &[&str]) = if cfg!(windows) {
            ("ping", &["-n", "120", "127.0.0.1"])
        } else {
            ("sleep", &["120"])
        };
        Command::new(program)
            .args(args)
            .stdout(Stdio::null())
            .spawn()
            .unwrap()
    }

    fn touch(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"x").unwrap();
    }

    /// Legacy folders under `app_data` as legacy finds them on Windows, each
    /// with a file in it.
    fn legacy(app_data: &Path) -> Vec<PathBuf> {
        let env = AppDataEnv {
            app_data: Some(OsString::from(app_data)),
            ..AppDataEnv::default()
        };
        let folders: Vec<_> = legacy_folders(Os::Windows, &env)
            .into_iter()
            .map(|f| f.path)
            .collect();
        for folder in &folders {
            touch(&folder.join("IndexedDB").join("000003.log"));
        }
        folders
    }

    #[test]
    fn f17_reset_data_keeps_logs_and_legacy_folders_and_deletes_ng_data() {
        let app_data = tempfile::tempdir().unwrap();
        let ng = app_data.path().join("com.exilence.exilence-ce-ng");
        let cache = app_data
            .path()
            .join("cache")
            .join("com.exilence.exilence-ce-ng");
        let logs = ng.join("logs");
        let ng_files = [
            ng.join("exilence-ce-ng.db"),
            ng.join("exilence-ce-ng.db-wal"),
            ng.join("exilence-ce-ng.db-shm"),
            ng.join(".window-state.json"),
            ng.join("EBWebView").join("Default").join("Cookies"),
            cache.join("blob"),
        ];
        for file in &ng_files {
            touch(file);
        }
        touch(&logs.join("exilence-ce-ng.log"));
        let legacy = legacy(app_data.path());
        let mut keep = vec![logs.clone()];
        keep.extend(legacy.iter().cloned());
        let locations = Locations {
            roots: vec![ng.clone(), cache.clone()],
            keep,
        };

        assert!(locations.delete().first.is_none());

        for file in &ng_files {
            assert!(!file.exists(), "{}", file.display());
        }
        assert!(logs.join("exilence-ce-ng.log").exists());
        for folder in &legacy {
            assert!(folder.join("IndexedDB").join("000003.log").exists());
        }
        // Even a root holding the legacy folders leaves them alone.
        touch(&ng.join("exilence-ce-ng.db"));
        let parent = Locations {
            roots: vec![app_data.path().to_owned()],
            ..locations
        };
        assert!(parent.delete().first.is_none());
        assert!(!ng.join("exilence-ce-ng.db").exists());
        assert!(logs.join("exilence-ce-ng.log").exists());
        for folder in &legacy {
            assert!(folder.join("IndexedDB").join("000003.log").exists());
        }
    }

    #[test]
    fn f17_pending_reset_runs_once_at_start() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let locations = Locations {
            roots: vec![data.clone()],
            keep: vec![data.join("logs")],
        };
        touch(&data.join("exilence-ce-ng.db"));
        finish_pending(&data, &locations).unwrap();
        assert!(data.join("exilence-ce-ng.db").exists(), "no marker: kept");

        // Empty, as older builds write it: everything, no wait.
        fs::write(data.join(PENDING_FILE), b"").unwrap();
        touch(&data.join("logs").join("exilence-ce-ng.log"));
        finish_pending(&data, &locations).unwrap();
        assert!(!data.join("exilence-ce-ng.db").exists());
        assert!(!data.join(PENDING_FILE).exists());
        assert!(data.join("logs").join("exilence-ce-ng.log").exists());
    }

    /// Webview data under `data` and a marker naming `browser` as the
    /// webview's browser process: the data's cookies and its locations.
    fn pending_with_browser(data: &Path, browser: &Child) -> (PathBuf, Locations) {
        let cookies = data.join("EBWebView").join("Default").join("Cookies");
        touch(&cookies);
        let marker = Marker {
            browser_process: Some(browser.id()),
            left: None,
        };
        marker.write(data).unwrap();
        let locations = Locations {
            roots: vec![data.to_owned()],
            keep: vec![],
        };
        (cookies, locations)
    }

    #[test]
    fn f17_reset_deletes_the_webview_data_once_its_browser_process_exited() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let mut browser = long_running();
        let (cookies, locations) = pending_with_browser(&data, &browser);

        let reset = {
            let data = data.clone();
            thread::spawn(move || {
                finish_pending(&data, &locations).unwrap();
                Instant::now()
            })
        };
        // The browser process runs a while after the restart.
        thread::sleep(Duration::from_millis(300));
        let exiting = Instant::now();
        browser.kill().unwrap();
        browser.wait().unwrap();

        let finished = reset.join().unwrap();
        assert!(finished >= exiting, "deleted while the browser process ran");
        assert!(
            finished - exiting < BROWSER_EXIT_WAIT / 2,
            "waited past the exit"
        );
        assert!(!cookies.exists());
        assert!(!data.join(PENDING_FILE).exists());
    }

    #[test]
    fn f17_reset_waits_for_a_browser_process_that_never_exits_only_so_long() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        // Still running, or its ID taken by another process.
        let mut browser = long_running();
        let (cookies, locations) = pending_with_browser(&data, &browser);

        let start = Instant::now();
        finish_pending_within(&data, &locations, Duration::from_millis(200)).unwrap();
        let waited = start.elapsed();
        browser.kill().unwrap();
        browser.wait().unwrap();

        // Windows timeouts may end a timer tick early.
        assert!(waited >= Duration::from_millis(150), "{waited:?}");
        assert!(waited < Duration::from_secs(3), "{waited:?}");
        assert!(!cookies.exists());
        assert!(!data.join(PENDING_FILE).exists());
    }

    /// Keeps `file` from being deleted until the returned guard drops.
    #[cfg(unix)]
    fn hold(file: &Path) -> impl Sized {
        use std::os::unix::fs::PermissionsExt;
        struct ReadOnly(PathBuf);
        impl Drop for ReadOnly {
            fn drop(&mut self) {
                fs::set_permissions(&self.0, fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        let dir = file.parent().unwrap().to_owned();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();
        ReadOnly(dir)
    }

    /// Keeps `file` from being deleted until the returned guard drops.
    #[cfg(windows)]
    fn hold(file: &Path) -> impl Sized {
        use std::os::windows::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(file)
            .unwrap()
    }

    #[test]
    fn f17_a_failed_delete_is_retried_at_the_next_start_without_newer_data() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        let local = dir.path().join("local");
        let log = data.join("logs").join("exilence-ce-ng.log");
        let db = data.join("exilence-ce-ng.db");
        let window_state = data.join(".window-state.json");
        let cookies = local.join("EBWebView").join("Default").join("Cookies");
        for file in [&log, &db, &window_state, &cookies] {
            touch(file);
        }
        fs::write(data.join(PENDING_FILE), b"").unwrap();
        let locations = Locations {
            roots: vec![data.clone(), local.clone()],
            keep: vec![data.join("logs")],
        };

        let held = hold(&cookies);
        let error = finish_pending(&data, &locations).unwrap_err();
        drop(held);
        assert!(error.to_string().contains("EBWebView"), "{error}");
        assert!(!db.exists());
        assert!(!window_state.exists());
        // The restarted app's data.
        touch(&db);
        touch(&window_state);

        finish_pending(&data, &locations).unwrap();
        assert!(!local.join("EBWebView").exists());
        assert!(db.exists());
        assert!(window_state.exists());
        assert!(log.exists());
        assert!(!data.join(PENDING_FILE).exists());
    }
}
