//! Diagnostics (F20): the crash notice, the panic hook, Debug mode and the
//! Diagnostics bundle. Debug mode's request lines are [`ninja::http::send`].

use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, BufWriter, Seek, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::SystemTime;

use ipc::slices::{NotificationKind, RateLimit, Settings, Status, Update};
use ipc::{IpcError, Message, MessageKey};
use serde_json::{Value, json};
use sqlx::SqlitePool;
use tauri::{AppHandle, Manager, Runtime};
use zip::CompressionMethod;
use zip::write::SimpleFileOptions;

use crate::desktop::Desktop;
use crate::ggg::Ggg;
use crate::state::{self, Store};
use crate::{internal, logging};

/// The `settings` key that is false while the app runs and true after a
/// clean exit (`client-schema.sql`).
const CLEAN_EXIT: &str = "clean_exit";

/// The bundle's report of versions, system, settings and rate limits.
pub const REPORT_FILE: &str = "diagnostics.json";

/// Marks this run as running: until [`mark_clean_exit`], the next launch
/// takes it for a crash. Whether the last run ended without a clean exit.
pub async fn mark_running(pool: &SqlitePool) -> sqlx::Result<bool> {
    let stored: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?1")
        .bind(CLEAN_EXIT)
        .fetch_optional(pool)
        .await?;
    store_clean_exit(pool, false).await?;
    Ok(stored.as_deref() == Some("false"))
}

/// Records a clean exit, so the next launch shows no crash notice: at the
/// app's exit, and before the updater's installer ends the process.
pub async fn mark_clean_exit(pool: &SqlitePool) {
    if let Err(error) = store_clean_exit(pool, true).await {
        log::warn!("cannot record the clean exit: {error}");
    }
}

async fn store_clean_exit(pool: &SqlitePool, clean: bool) -> sqlx::Result<()> {
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?1, ?2) \
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
    )
    .bind(CLEAN_EXIT)
    .bind(clean.to_string())
    .execute(pool)
    .await
    .map(drop)
}

/// The crash notice: a list-only notification that the last session ended
/// unexpectedly, pointing to Export diagnostics.
pub fn notify_crash<R: Runtime>(app: &AppHandle<R>) {
    log::warn!("the last session ended unexpectedly");
    state::push(
        app,
        NotificationKind::Warning,
        MessageKey::NotificationLastSessionCrashed.into(),
        Some(MessageKey::NotificationLastSessionCrashedDescription.into()),
    );
}

/// Logs every panic of the core, with its thread and a backtrace, before
/// the previous hook runs.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let backtrace = std::backtrace::Backtrace::force_capture();
        log::error!(
            "panic in thread {}: {info}\n{backtrace}",
            thread.name().unwrap_or("unnamed")
        );
        log::logger().flush();
        previous(info);
    }));
}

/// Applies the Debug mode setting at once (F20) and pushes the `status`
/// slice's marker.
pub fn set_debug_mode<R: Runtime>(app: &AppHandle<R>, on: bool) {
    logging::set_debug_mode(on);
    log::info!("Debug mode {}", if on { "on" } else { "off" });
    let on = on || logging::debug_from_launch();
    if let Err(error) = app
        .state::<Store>()
        .update::<Status, _>(app, |s| s.debug_mode = on)
    {
        log::error!("cannot push the status: {error}");
    }
}

/// `export_diagnostics` (F20): asks where to save the Diagnostics bundle,
/// saves it there and notes it in the list; nothing when the user cancels.
/// A failure is notified with a toast.
pub async fn export<R: Runtime>(app: &AppHandle<R>) -> Result<(), IpcError> {
    let desktop = app.state::<Arc<dyn Desktop>>().inner().clone();
    let saved = match desktop.save_path(bundle_file_name(SystemTime::now())).await {
        Ok(None) => return Ok(()),
        Ok(Some(path)) => {
            let report = report(app);
            let written = path.clone();
            tauri::async_runtime::spawn_blocking(move || save_bundle(&written, &report))
                .await
                .map_err(|e| e.to_string())
                .and_then(|saved| saved.map_err(|e| e.to_string()))
                .map(|()| path)
        }
        Err(error) => Err(format!("the save dialog failed: {error}")),
    };
    match saved {
        Ok(path) => {
            log::info!("Diagnostics bundle saved to {}", path.display());
            let message = Message::new(MessageKey::NotificationDiagnosticsExported)
                .arg("path", path.display().to_string());
            state::push(app, NotificationKind::Success, message, None);
            Ok(())
        }
        Err(error) => {
            let error = internal(format!("cannot export diagnostics: {error}"));
            notify_failure(app, MessageKey::NotificationDiagnosticsExportFailed, &error);
            Err(error)
        }
    }
}

/// `open_log_folder` (F20): opens the log directory in the file manager.
pub fn open_log_folder<R: Runtime>(app: &AppHandle<R>) -> Result<(), IpcError> {
    let opened = logging::dir()
        .ok_or_else(|| "the log is not open".to_owned())
        .and_then(|dir| app.state::<Arc<dyn Desktop>>().open_dir(dir));
    opened.map_err(|error| {
        let error = internal(format!("cannot open the log folder: {error}"));
        notify_failure(app, MessageKey::NotificationLogFolderFailed, &error);
        error
    })
}

fn notify_failure<R: Runtime>(app: &AppHandle<R>, title: MessageKey, error: &IpcError) {
    state::push(
        app,
        NotificationKind::Error,
        title.into(),
        Some(error.message()),
    );
}

/// The name the save dialog suggests, with the time of the export.
pub fn bundle_file_name(now: SystemTime) -> String {
    let at = humantime::format_rfc3339_seconds(now)
        .to_string()
        .replace(':', "-");
    format!("exilence-ce-ng-diagnostics-{at}.zip")
}

/// The bundle's report: app version and channel, Debug mode, OS and
/// architecture, webview version, the settings (which hold no secrets) and
/// the rate-limit state, with every learnt policy.
pub fn report<R: Runtime>(app: &AppHandle<R>) -> Value {
    let store = app.state::<Store>();
    let channel = store.read(|u: &Update| u.channel);
    let settings = store.read(Settings::clone);
    let rate_limit = store.read(RateLimit::clone);
    json!({
        "app": {
            "version": env!("CARGO_PKG_VERSION"),
            "channel": channel,
            "debug_mode": store.read(|s: &Status| s.debug_mode),
            "debug_from_launch": logging::debug_from_launch(),
        },
        "system": system(),
        "settings": settings,
        "rate_limit": {
            "cooldown_until": rate_limit.cooldown_until,
            "allowance": rate_limit.allowance,
            "policies": app.state::<Ggg>().policies(),
        },
    })
}

fn system() -> Value {
    #[cfg_attr(not(target_os = "linux"), allow(unused_mut))]
    let mut system = json!({
        "os": std::env::consts::OS,
        "os_version": os_info::get().to_string(),
        "arch": std::env::consts::ARCH,
        "webview": tauri::webview_version().unwrap_or_else(|e| format!("unknown: {e}")),
    });
    #[cfg(target_os = "linux")]
    {
        system["linux"] = linux();
    }
    system
}

/// The session type, the GPUs with their drivers and the effective
/// `GDK_BACKEND` and `WEBKIT_*` values.
#[cfg(target_os = "linux")]
fn linux() -> Value {
    use std::collections::BTreeMap;
    use std::env;

    let webkit: BTreeMap<String, String> = env::vars()
        .filter(|(name, _)| name.starts_with("WEBKIT_"))
        .collect();
    json!({
        "session_type": env::var("XDG_SESSION_TYPE").ok(),
        "wayland_display": env::var("WAYLAND_DISPLAY").ok(),
        "x11_display": env::var("DISPLAY").ok(),
        "kernel": read_trimmed(Path::new("/proc/sys/kernel/osrelease")),
        "gpus": gpus(Path::new("/sys/class/drm")),
        "gdk_backend": env::var("GDK_BACKEND").ok(),
        "webkit": webkit,
    })
}

#[cfg(target_os = "linux")]
fn read_trimmed(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_owned())
}

/// Every `cardN` under `drm`, with its PCI vendor and device, kernel driver
/// and that driver's module version where it has one.
#[cfg(target_os = "linux")]
fn gpus(drm: &Path) -> Vec<Value> {
    let Ok(entries) = fs::read_dir(drm) else {
        return Vec::new();
    };
    let mut cards: Vec<(String, std::path::PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let n = name.strip_prefix("card")?;
            (!n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
                .then(|| (name, entry.path().join("device")))
        })
        .collect();
    cards.sort();
    cards
        .into_iter()
        .map(|(card, device)| {
            let vendor_id = read_trimmed(&device.join("vendor"));
            let vendor = match vendor_id.as_deref() {
                Some("0x10de") => Some("NVIDIA"),
                Some("0x1002") => Some("AMD"),
                Some("0x8086") => Some("Intel"),
                _ => None,
            };
            let driver = fs::read_link(device.join("driver"))
                .ok()
                .and_then(|p| p.file_name()?.to_str().map(str::to_owned));
            let driver_version = driver
                .as_ref()
                .and_then(|d| read_trimmed(&Path::new("/sys/module").join(d).join("version")));
            json!({
                "card": card,
                "vendor": vendor,
                "vendor_id": vendor_id,
                "device_id": read_trimmed(&device.join("device")),
                "driver": driver,
                "driver_version": driver_version,
            })
        })
        .collect()
}

/// Saves the bundle as `path`; a partly written file is removed.
fn save_bundle(path: &Path, report: &Value) -> io::Result<()> {
    let written = File::create(path).and_then(|file| {
        let file = write_bundle(BufWriter::new(file), logging::dir(), report)?;
        file.into_inner().map_err(|e| e.into_error())?.sync_all()
    });
    if written.is_err() {
        let _ = fs::remove_file(path);
    }
    written
}

/// Writes the Diagnostics bundle into `out`: `report` as [`REPORT_FILE`] and
/// every log file of `log_dir` under `logs/`, each line redacted again with
/// every secret known now.
pub fn write_bundle<W: Write + Seek>(
    out: W,
    log_dir: Option<&Path>,
    report: &Value,
) -> io::Result<W> {
    let mut zip = zip::ZipWriter::new(out);
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    zip.start_file(REPORT_FILE, options)?;
    let report = serde_json::to_string_pretty(report).map_err(io::Error::other)?;
    zip.write_all(logging::redact(&report).as_bytes())?;
    for n in 0..logging::FILES {
        let Some(dir) = log_dir else { break };
        let name = logging::file_name(n);
        let file = match File::open(dir.join(&name)) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        zip.start_file(format!("logs/{name}"), options)?;
        let mut lines = BufReader::new(file);
        let mut line = Vec::new();
        while lines.read_until(b'\n', &mut line)? > 0 {
            zip.write_all(logging::redact(&String::from_utf8_lossy(&line)).as_bytes())?;
            line.clear();
        }
    }
    Ok(zip.finish()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_file_name_carries_the_time_without_colons() {
        let at = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_800_000_000);
        assert_eq!(
            bundle_file_name(at),
            "exilence-ce-ng-diagnostics-2027-01-15T08-00-00Z.zip"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn f20_gpus_name_vendor_and_driver() {
        let drm = tempfile::tempdir().unwrap();
        let card = drm.path().join("card0").join("device");
        fs::create_dir_all(&card).unwrap();
        fs::write(card.join("vendor"), "0x1002\n").unwrap();
        fs::write(card.join("device"), "0x73bf\n").unwrap();
        let driver = drm.path().join("drivers").join("amdgpu");
        fs::create_dir_all(&driver).unwrap();
        std::os::unix::fs::symlink(&driver, card.join("driver")).unwrap();
        // A connector, not a card.
        fs::create_dir_all(drm.path().join("card0-DP-1")).unwrap();

        let gpus = gpus(drm.path());
        assert_eq!(gpus.len(), 1);
        assert_eq!(gpus[0]["card"], "card0");
        assert_eq!(gpus[0]["vendor"], "AMD");
        assert_eq!(gpus[0]["device_id"], "0x73bf");
        assert_eq!(gpus[0]["driver"], "amdgpu");
    }
}
