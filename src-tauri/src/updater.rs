//! The updater (F16, ADR 0003): release builds check the release channel's
//! update file at launch and every 5 minutes, signed in or not, download a
//! found update in the background and install it when the user clicks the
//! title bar's banner. Only the core uses `tauri-plugin-updater`; the webview
//! has no updater permission.
//!
//! The periodic check is a [`Deadline::UpdateCheck`] of the scheduler.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};

use ipc::slices::{ReleaseChannel, Update, UpdateState};
use semver::Version;
use tauri::{AppHandle, Manager, Runtime, Url};
use tauri_plugin_updater::UpdaterExt;

use crate::github;
use crate::scheduler::{Deadline, Scheduler};
use crate::slot;
use crate::state::Store;

/// Whether this build checks for updates: release builds that no package
/// manager updates (F16).
pub const ENABLED: bool = cfg!(not(debug_assertions)) && !crate::DISTRO_PACKAGE;

/// Whether this copy can choose its release channel (ADR 0006): not a distro
/// package, which its package manager updates, nor an `.rpm`, which only
/// stable releases carry because an RPM version cannot hold a prerelease.
pub fn channel_choice() -> bool {
    !crate::DISTRO_PACKAGE
        && tauri::utils::platform::bundle_type() != Some(tauri::utils::config::BundleType::Rpm)
}

/// Time from the end of one check to the next, in milliseconds.
pub const CHECK_INTERVAL_MS: i64 = 5 * 60 * 1000;

/// The tag of the fixed release that holds the update files (ADR 0003).
const UPDATER_TAG: &str = "updater";

/// The updater's state, managed by the app.
#[derive(Default)]
pub struct Updater {
    /// A check runs.
    checking: AtomicBool,
    /// The downloaded update, its signature verified.
    ready: Mutex<Option<Ready>>,
}

struct Ready {
    update: tauri_plugin_updater::Update,
    bytes: Vec<u8>,
}

/// The update file of `channel` on the `updater` release (ADR 0003).
pub fn update_file(channel: ReleaseChannel) -> &'static str {
    match channel {
        ReleaseChannel::Latest => "latest.json",
        ReleaseChannel::Beta => "beta.json",
    }
}

/// Where `channel`'s update file is: the `updater` release of
/// [`github::REPOSITORY`] (ADR 0003).
#[expect(clippy::expect_used, reason = "a GitHub release asset URL is valid")]
pub fn endpoint(channel: ReleaseChannel) -> Url {
    Url::parse(&format!(
        "https://github.com/{}/releases/download/{UPDATER_TAG}/{}",
        github::REPOSITORY,
        update_file(channel)
    ))
    .expect("a GitHub release asset URL is valid")
}

/// Whether a release of `remote` is an update for this build of `current`:
/// only a higher version, whatever the channel. After switching from Beta to
/// Latest Stable the installed version stays until a higher stable release
/// exists (D23).
pub fn is_update(current: &Version, remote: &Version) -> bool {
    remote.cmp_precedence(current).is_gt()
}

/// Download progress in percent; 0 while the size is unknown.
pub fn percent(received: u64, total: Option<u64>) -> u8 {
    match total {
        Some(total) if total > 0 => u8::try_from(received.min(total) * 100 / total).unwrap_or(100),
        _ => 0,
    }
}

/// When the next periodic check is due after one that ended at
/// `checked_at`, by wall clock (Unix ms).
pub fn next_check_at(checked_at: i64) -> i64 {
    checked_at + CHECK_INTERVAL_MS
}

/// Starts the checks in release builds: one now, then every 5 minutes until
/// an update is ready (F16).
pub fn start<R: Runtime>(app: &AppHandle<R>) {
    check_now(app);
}

/// Checks now in the background, in release builds, unless a check runs or
/// an update is ready: at launch, the tray's "Check for Updates…" and every
/// [`Deadline::UpdateCheck`]. Unless an update is then ready, the next check
/// is due [`CHECK_INTERVAL_MS`] later.
pub fn check_now<R: Runtime>(app: &AppHandle<R>) {
    let updater = app.state::<Updater>();
    let ready = updater
        .ready
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .is_some();
    if !ENABLED || ready || updater.checking.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let ready = check(&app).await;
        app.state::<Updater>()
            .checking
            .store(false, Ordering::SeqCst);
        let scheduler = app.state::<Scheduler>();
        if ready {
            scheduler.clear(Deadline::UpdateCheck);
        } else {
            scheduler.set(Deadline::UpdateCheck, next_check_at(scheduler.now()));
        }
    });
}

/// Installs the downloaded update, its signature checked again, and restarts
/// into the new version (F16). A running Snapshot is cancelled first; the
/// slot stays held until the restart. On Windows the installer takes over
/// and the app exits right away.
pub async fn install<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let updater = app.state::<Updater>();
    let lock = || updater.ready.lock().unwrap_or_else(PoisonError::into_inner);
    if lock().is_none() {
        return Err("no update is ready to install".to_owned());
    }
    let _held = slot::cancel_and_hold(app).await;
    let ready = lock();
    let Ready { update, bytes } = ready.as_ref().ok_or("no update is ready to install")?;
    update
        .install(bytes)
        .map_err(|error| format!("cannot install update {}: {error}", update.version))?;
    drop(ready);
    app.restart()
}

/// One check on the channel this run uses. Whether an update is ready.
async fn check<R: Runtime>(app: &AppHandle<R>) -> bool {
    let channel = app.state::<Store>().read(|update: &Update| update.channel);
    match download(app, channel).await {
        Ok(ready) => ready,
        Err(error) => {
            // A bad signature lands here too: the update is rejected.
            log::error!("update check on {channel:?} failed: {error}");
            set_state(app, UpdateState::None);
            false
        }
    }
}

/// Finds an update in `channel`'s file, downloads it and verifies its
/// signature against the update key built into the app.
async fn download<R: Runtime>(
    app: &AppHandle<R>,
    channel: ReleaseChannel,
) -> tauri_plugin_updater::Result<bool> {
    let found = app
        .updater_builder()
        .endpoints(vec![endpoint(channel)])?
        .version_comparator(|current, release| is_update(&current, &release.version))
        .build()?
        .check()
        .await?;
    let Some(update) = found else {
        set_state(app, UpdateState::None);
        return Ok(false);
    };
    let version = update.version.clone();
    set_state(
        app,
        UpdateState::Available {
            version: version.clone(),
        },
    );
    let mut received = 0u64;
    let mut shown = None;
    let on_chunk = |chunk: usize, total: Option<u64>| {
        received += chunk as u64;
        let percent = percent(received, total);
        if shown != Some(percent) {
            shown = Some(percent);
            set_state(
                app,
                UpdateState::Downloading {
                    version: version.clone(),
                    percent,
                },
            );
        }
    };
    // `download` rejects bytes whose signature does not match the key.
    let bytes = update.download(on_chunk, || {}).await?;
    *app.state::<Updater>()
        .ready
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = Some(Ready { update, bytes });
    set_state(app, UpdateState::Ready { version });
    Ok(true)
}

/// Sets the `update` slice's state, pushing it only when it changes.
fn set_state<R: Runtime>(app: &AppHandle<R>, state: UpdateState) {
    let store = app.state::<Store>();
    if store.read(|update: &Update| update.state == state) {
        return;
    }
    if let Err(error) = store.update::<Update, _>(app, |update| update.state = state) {
        log::error!("cannot push the update state: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(version: &str) -> Version {
        Version::parse(version).unwrap()
    }

    /// The plugin accepts the built-in configuration: the update key, and
    /// signatures bound to the version the update file announces, so a
    /// tampered file cannot pass off an older release as newer (D23).
    #[test]
    fn f16_updater_config_binds_signatures_to_the_update_key_and_version() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let updater: tauri_plugin_updater::Config =
            serde_json::from_value(config["plugins"]["updater"].clone()).unwrap();
        assert!(!updater.pubkey.is_empty());
        assert!(updater.require_signed_version);
        assert!(!updater.allow_downgrades);
        // The core sets the endpoints from the release channel.
        assert!(updater.endpoints.is_empty());
    }

    #[test]
    fn f16_update_files_are_read_from_the_updater_release() {
        assert_eq!(
            endpoint(ReleaseChannel::Beta).as_str(),
            "https://github.com/exilence-ce/exilence-ce/releases/download/updater/beta.json"
        );
        assert_eq!(
            endpoint(ReleaseChannel::Latest).as_str(),
            "https://github.com/exilence-ce/exilence-ce/releases/download/updater/latest.json"
        );
    }

    #[test]
    fn d23_only_a_higher_version_is_an_update() {
        // Beta offers the higher of stable and beta; both move forward.
        assert!(is_update(&v("2.0.0-beta.3"), &v("2.0.0")));
        assert!(is_update(&v("2.0.0-beta.9"), &v("2.0.0-beta.10")));
        assert!(is_update(&v("2.0.0"), &v("2.0.1")));
        // The same version is not an update.
        assert!(!is_update(&v("2.0.0"), &v("2.0.0")));
        assert!(!is_update(&v("2.0.0"), &v("2.0.0+rebuild")));
    }

    #[test]
    fn d23_switching_from_beta_to_latest_stable_never_downgrades() {
        let installed = v("2.1.0-beta.2");
        // `latest.json` still names the older stable release: stay.
        assert!(!is_update(&installed, &v("2.0.1")));
        // Until a higher stable release exists.
        assert!(is_update(&installed, &v("2.1.0")));
    }

    #[test]
    fn f16_download_progress_in_percent() {
        assert_eq!(percent(0, Some(200)), 0);
        assert_eq!(percent(50, Some(200)), 25);
        assert_eq!(percent(200, Some(200)), 100);
        assert_eq!(percent(300, Some(200)), 100);
        assert_eq!(percent(100, None), 0);
        assert_eq!(percent(100, Some(0)), 0);
    }

    #[test]
    fn f16_checks_every_five_minutes_by_wall_clock() {
        let checked_at = 1_700_000_000_000;
        assert_eq!(next_check_at(checked_at), checked_at + 300_000);
    }
}
