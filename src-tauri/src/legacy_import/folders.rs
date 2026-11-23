//! Where legacy keeps its data: Electron's `userData` = `appData/<package name>`. CE ≥ 1.2.3 is
//! `exilence-ce-app`; Exilence Next and CE ≤ 1.2.2 used `exilence-next-app`, and the rename
//! moved no data, so both are read.

use std::ffi::OsString;
use std::path::PathBuf;

/// Which legacy build wrote a folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyApp {
    /// CE ≥ 1.2.3.
    Ce,
    /// Exilence Next and CE ≤ 1.2.2.
    Next,
}

impl LegacyApp {
    pub const ALL: [Self; 2] = [Self::Ce, Self::Next];

    pub fn dir_name(self) -> &'static str {
        match self {
            Self::Ce => "exilence-ce-app",
            Self::Next => "exilence-next-app",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Windows,
    MacOs,
    Linux,
}

impl Os {
    /// The OS this build runs on.
    pub const CURRENT: Self = if cfg!(windows) {
        Self::Windows
    } else if cfg!(target_os = "macos") {
        Self::MacOs
    } else {
        Self::Linux
    };
}

/// The environment variables Electron's `appData` derives from.
#[derive(Debug, Clone, Default)]
pub struct AppDataEnv {
    pub app_data: Option<OsString>,
    pub home: Option<OsString>,
    pub xdg_config_home: Option<OsString>,
}

impl AppDataEnv {
    pub fn from_process() -> Self {
        Self {
            app_data: std::env::var_os("APPDATA"),
            home: std::env::var_os("HOME"),
            xdg_config_home: std::env::var_os("XDG_CONFIG_HOME"),
        }
    }
}

/// A legacy folder candidate; it may not exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyFolder {
    pub app: LegacyApp,
    pub path: PathBuf,
}

/// The legacy folders to probe on `os`, `exilence-ce-app` first: under `%APPDATA%`,
/// `~/Library/Application Support` or `${XDG_CONFIG_HOME:-~/.config}`. Empty when the base
/// directory cannot be determined.
pub fn legacy_folders(os: Os, env: &AppDataEnv) -> Vec<LegacyFolder> {
    let non_empty = |v: &Option<OsString>| v.as_ref().filter(|v| !v.is_empty()).map(PathBuf::from);
    let app_data = match os {
        Os::Windows => non_empty(&env.app_data),
        Os::MacOs => non_empty(&env.home).map(|h| h.join("Library").join("Application Support")),
        Os::Linux => non_empty(&env.xdg_config_home)
            .or_else(|| non_empty(&env.home).map(|h| h.join(".config"))),
    };
    app_data
        .map(|base| {
            LegacyApp::ALL
                .map(|app| LegacyFolder {
                    app,
                    path: base.join(app.dir_name()),
                })
                .to_vec()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(app_data: Option<&str>, home: Option<&str>, xdg: Option<&str>) -> AppDataEnv {
        AppDataEnv {
            app_data: app_data.map(OsString::from),
            home: home.map(OsString::from),
            xdg_config_home: xdg.map(OsString::from),
        }
    }

    fn paths(os: Os, env: &AppDataEnv) -> Vec<(LegacyApp, PathBuf)> {
        legacy_folders(os, env)
            .into_iter()
            .map(|f| (f.app, f.path))
            .collect()
    }

    fn both(base: PathBuf) -> Vec<(LegacyApp, PathBuf)> {
        vec![
            (LegacyApp::Ce, base.join("exilence-ce-app")),
            (LegacyApp::Next, base.join("exilence-next-app")),
        ]
    }

    #[test]
    fn f21_legacy_folders_windows() {
        let appdata = r"C:\Users\Foo\AppData\Roaming";
        let e = env(Some(appdata), Some(r"C:\Users\Foo"), Some("/ignored"));
        assert_eq!(paths(Os::Windows, &e), both(PathBuf::from(appdata)));
        assert!(paths(Os::Windows, &env(None, Some(r"C:\Users\Foo"), None)).is_empty());
    }

    #[test]
    fn f21_legacy_folders_macos() {
        let e = env(None, Some("/Users/foo"), Some("/ignored"));
        let base = PathBuf::from("/Users/foo/Library/Application Support");
        assert_eq!(paths(Os::MacOs, &e), both(base));
        assert!(paths(Os::MacOs, &env(None, None, None)).is_empty());
    }

    #[test]
    fn f21_legacy_folders_linux() {
        let xdg = env(Some("/ignored"), Some("/home/foo"), Some("/home/foo/cfg"));
        assert_eq!(paths(Os::Linux, &xdg), both(PathBuf::from("/home/foo/cfg")));
        // `${XDG_CONFIG_HOME:-~/.config}`: unset or empty falls back.
        let fallback = both(PathBuf::from("/home/foo/.config"));
        assert_eq!(
            paths(Os::Linux, &env(None, Some("/home/foo"), None)),
            fallback
        );
        assert_eq!(
            paths(Os::Linux, &env(None, Some("/home/foo"), Some(""))),
            fallback
        );
        assert!(paths(Os::Linux, &env(None, None, None)).is_empty());
    }
}
