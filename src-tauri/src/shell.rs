//! The desktop shell (F15): the main window, its close action, the second
//! instance, the Net worth overlay (F14), the webview's environment
//! defaults, hardware acceleration (D7) and the UI scale (F17).

use ipc::slices::{ExitAction, Settings};
use sqlx::SqlitePool;
use tauri::{
    AppHandle, Manager, RunEvent, Runtime, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
    WindowEvent,
};

use crate::diagnostics;
use crate::state::Store;

/// The label of the main window.
pub const MAIN_WINDOW: &str = "main";

/// The label of the Net worth overlay (F14).
pub const OVERLAY_WINDOW: &str = "overlay";

/// The overlay's title, which never changes: the README's tiling-WM rules
/// match on it (F14).
pub const OVERLAY_TITLE: &str = "Exilence CE NG Overlay";

/// The overlay's default and minimum inner size in logical pixels (F14).
const OVERLAY_SIZE: (f64, f64) = (200.0, 92.0);

/// The stored hardware acceleration as of this start (D7): every window
/// gets it, also one opened after the setting changed, since they share one
/// WebView2 environment.
#[derive(Clone, Copy, Debug)]
pub struct StartupHardwareAcceleration(pub bool);

/// Handles the app's events: closing the main window hides it to the tray
/// unless the exit action is "exit" (F15), which also closes the overlay
/// (F14); the app's exit is recorded as clean, so the next launch shows no
/// crash notice (F20).
pub fn on_run_event<R: Runtime>(app: &AppHandle<R>, event: &RunEvent) {
    if let RunEvent::Exit = event {
        log::info!("exiting");
        if let Some(pool) = app.try_state::<SqlitePool>() {
            tauri::async_runtime::block_on(diagnostics::mark_clean_exit(&pool));
        }
        return;
    }
    let RunEvent::WindowEvent {
        label,
        event: WindowEvent::CloseRequested { api, .. },
        ..
    } = event
    else {
        return;
    };
    if label != MAIN_WINDOW {
        return;
    }
    let exit_action = app
        .state::<Store>()
        .read(|settings: &Settings| settings.exit_action);
    if exit_action == ExitAction::MinimizeToTray {
        api.prevent_close();
        if let Some(Err(error)) = app.get_webview_window(MAIN_WINDOW).map(|w| w.hide()) {
            log::error!("cannot hide the main window: {error}");
        }
    } else {
        // The app quits once its last window is gone.
        close_overlay(app);
    }
}

/// Opens the Net worth overlay (F14), or brings it to the front when open:
/// frameless, always on top, without a taskbar entry. The window-state
/// plugin restores its size and position.
pub fn open_overlay<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window(OVERLAY_WINDOW) {
        return window.show().and_then(|()| window.set_focus());
    }
    let (width, height) = OVERLAY_SIZE;
    let builder = WebviewWindowBuilder::new(app, OVERLAY_WINDOW, WebviewUrl::App("overlay".into()))
        .title(OVERLAY_TITLE)
        .inner_size(width, height)
        .min_inner_size(width, height)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true);
    let StartupHardwareAcceleration(hardware_acceleration) =
        *app.state::<StartupHardwareAcceleration>();
    build_window(builder, hardware_acceleration)?;
    apply_ui_scale(app);
    Ok(())
}

/// Closes the Net worth overlay if open (F14).
pub fn close_overlay<R: Runtime>(app: &AppHandle<R>) {
    if let Some(Err(error)) = app.get_webview_window(OVERLAY_WINDOW).map(|w| w.close()) {
        log::error!("cannot close the overlay: {error}");
    }
}

/// Shows, restores and focuses the main window: the tray's Show and left
/// click, and a second instance.
pub fn show_main_window<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
        return;
    };
    let shown = window
        .unminimize()
        .and_then(|()| window.show())
        .and_then(|()| window.set_focus());
    if let Err(error) = shown {
        log::error!("cannot show the main window: {error}");
    }
}

/// Runs in this instance when the app is started again: the second instance
/// exits and this one comes to the front (F15). `args` are the second
/// instance's command line (its executable first), `cwd` its working
/// directory. An `exilence-ce-ng://` link among them has already gone to the
/// deep-link plugin, and from there to the login (F1).
pub fn on_second_instance<R: Runtime>(app: &AppHandle<R>, _args: Vec<String>, _cwd: String) {
    show_main_window(app);
}

/// Applies the stored UI scale to the zoom of every window (F17).
pub fn apply_ui_scale<R: Runtime>(app: &AppHandle<R>) {
    let scale = app.state::<Store>().read(|s: &Settings| s.ui_scale);
    for (label, window) in app.webview_windows() {
        if let Err(error) = window.set_zoom(f64::from(scale) / 100.0) {
            log::error!("cannot zoom the {label} window: {error}");
        }
    }
}

/// WebView2's browser arguments (D7): wry's defaults, which any arguments
/// replace, plus `--disable-gpu` with hardware acceleration off. Every
/// window must get the same ones: they share one WebView2 environment.
pub fn webview2_args(hardware_acceleration: bool) -> String {
    let defaults = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection";
    if hardware_acceleration {
        defaults.to_owned()
    } else {
        format!("{defaults} --disable-gpu")
    }
}

/// Builds a window with the stored hardware acceleration (D7): WebView2's
/// `--disable-gpu` on Windows, WebKitGTK's hardware-acceleration policy
/// *never* on Linux. macOS has no switch. Read once at start, so a change
/// applies at the next start.
pub fn build_window<'a, R: Runtime, M: Manager<R>>(
    builder: WebviewWindowBuilder<'a, R, M>,
    hardware_acceleration: bool,
) -> tauri::Result<WebviewWindow<R>> {
    let builder = if cfg!(windows) {
        builder.additional_browser_args(&webview2_args(hardware_acceleration))
    } else {
        builder
    };
    let window = builder.build()?;
    #[cfg(target_os = "linux")]
    if !hardware_acceleration {
        window.with_webview(|webview| {
            use webkit2gtk::{HardwareAccelerationPolicy, SettingsExt, WebViewExt};
            if let Some(settings) = webview.inner().settings() {
                settings.set_hardware_acceleration_policy(HardwareAccelerationPolicy::Never);
            }
        })?;
    }
    Ok(window)
}

/// The environment variables to set before the webview starts on `os`
/// (`std::env::consts::OS`), given which are already set: on Linux,
/// `WEBKIT_DISABLE_DMABUF_RENDERER=1`, so WebKitGTK renders on NVIDIA and
/// Mesa (F15), and `GDK_BACKEND=x11`, so Wayland sessions run the app
/// through XWayland, where the overlay can stay on top (F14). A variable the
/// user set is never overridden, whatever its value.
pub fn env_defaults(os: &str, is_set: impl Fn(&str) -> bool) -> Vec<(&'static str, &'static str)> {
    let defaults: &[(&str, &str)] = match os {
        "linux" => &[
            ("WEBKIT_DISABLE_DMABUF_RENDERER", "1"),
            ("GDK_BACKEND", "x11"),
        ],
        _ => &[],
    };
    defaults
        .iter()
        .copied()
        .filter(|(name, _)| !is_set(name))
        .collect()
}

/// Sets [`env_defaults`] for this process.
///
/// # Safety
///
/// Changes the environment: call it before any other thread starts, first in
/// `main`.
pub unsafe fn apply_env_defaults() {
    let defaults = env_defaults(std::env::consts::OS, |name| {
        std::env::var_os(name).is_some()
    });
    for (name, value) in defaults {
        // SAFETY: the caller guarantees no other thread runs yet.
        unsafe { std::env::set_var(name, value) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn d7_webview2_disables_the_gpu_only_without_hardware_acceleration() {
        let on = webview2_args(true);
        let off = webview2_args(false);
        assert!(!on.contains("--disable-gpu"));
        assert!(off.ends_with(" --disable-gpu"));
        // wry's defaults stay either way.
        assert!(off.starts_with(&on));
    }

    /// Whether `os`'s defaults, with `set` already set, include `name`.
    fn sets(os: &str, set: &str, name: &str) -> bool {
        env_defaults(os, |n| n == set)
            .iter()
            .any(|(n, _)| *n == name)
    }

    #[test]
    fn f15_linux_disables_the_dmabuf_renderer_unless_set() {
        assert!(
            env_defaults("linux", |_| false).contains(&("WEBKIT_DISABLE_DMABUF_RENDERER", "1"))
        );
        // Set by the user, even to 0: left alone.
        assert!(!sets(
            "linux",
            "WEBKIT_DISABLE_DMABUF_RENDERER",
            "WEBKIT_DISABLE_DMABUF_RENDERER"
        ));
        // Other variables do not matter.
        assert!(sets(
            "linux",
            "WEBKIT_DISABLE_COMPOSITING_MODE",
            "WEBKIT_DISABLE_DMABUF_RENDERER"
        ));
        for os in ["windows", "macos"] {
            assert!(env_defaults(os, |_| false).is_empty(), "{os}");
        }
    }

    #[test]
    fn f14_linux_runs_through_xwayland_unless_gdk_backend_is_set() {
        assert!(env_defaults("linux", |_| false).contains(&("GDK_BACKEND", "x11")));
        // The user's choice, e.g. `wayland`, wins.
        assert!(!sets("linux", "GDK_BACKEND", "GDK_BACKEND"));
        assert!(sets(
            "linux",
            "GDK_BACKEND",
            "WEBKIT_DISABLE_DMABUF_RENDERER"
        ));
        assert!(sets(
            "linux",
            "WEBKIT_DISABLE_DMABUF_RENDERER",
            "GDK_BACKEND"
        ));
        for os in ["windows", "macos"] {
            assert!(!sets(os, "", "GDK_BACKEND"), "{os}");
        }
    }

    #[test]
    fn f14_readme_window_rules_match_the_overlay_title() {
        let readme = include_str!("../../README.md");
        assert!(readme.contains(&format!(
            "match:title ^({OVERLAY_TITLE})$, float on, pin on, no_focus on, move"
        )));
        assert!(readme.contains(&format!(
            "for_window [title=\"^{OVERLAY_TITLE}$\"] floating enable, sticky enable"
        )));
    }
}
