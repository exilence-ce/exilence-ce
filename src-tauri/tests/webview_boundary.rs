//! The webview boundary: capabilities and CSP (client architecture, webview
//! boundary).

// Test support code outside `#[test]` functions: a panic is the failure.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeSet;

use serde_json::Value;

const MANIFEST_DIR: &str = env!("CARGO_MANIFEST_DIR");

fn json(path: &str) -> Value {
    let text = std::fs::read_to_string(format!("{MANIFEST_DIR}/{path}")).unwrap();
    serde_json::from_str(&text).unwrap()
}

#[test]
fn capabilities_grant_only_events_and_title_bar_controls() {
    let mut permissions = BTreeSet::new();
    for entry in std::fs::read_dir(format!("{MANIFEST_DIR}/capabilities")).unwrap() {
        let name = entry.unwrap().file_name().into_string().unwrap();
        let capability = json(&format!("capabilities/{name}"));
        for permission in capability["permissions"].as_array().unwrap() {
            permissions.insert(permission.as_str().unwrap().to_owned());
        }
    }
    let expected = BTreeSet::from(
        [
            "core:event:allow-listen",
            "core:event:allow-unlisten",
            "core:window:allow-close",
            "core:window:allow-internal-toggle-maximize",
            "core:window:allow-is-maximized",
            "core:window:allow-minimize",
            "core:window:allow-start-dragging",
            "core:window:allow-toggle-maximize",
        ]
        .map(String::from),
    );
    assert_eq!(permissions, expected);
}

#[test]
fn f14_overlay_may_only_listen_and_drag() {
    let capability = json("capabilities/overlay.json");
    assert_eq!(capability["windows"], serde_json::json!(["overlay"]));
    let permissions: BTreeSet<&str> = capability["permissions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p.as_str().unwrap())
        .collect();
    assert_eq!(
        permissions,
        BTreeSet::from([
            "core:event:allow-listen",
            "core:event:allow-unlisten",
            "core:window:allow-start-dragging",
        ])
    );
}

#[test]
fn no_fs_shell_http_or_dialog_plugin() {
    let manifest = std::fs::read_to_string(format!("{MANIFEST_DIR}/Cargo.toml")).unwrap();
    let config = json("tauri.conf.json");
    for plugin in ["fs", "shell", "http", "dialog"] {
        assert!(
            !manifest.contains(&format!("tauri-plugin-{plugin}")),
            "tauri-plugin-{plugin} is a dependency"
        );
        assert!(
            config["plugins"].get(plugin).is_none(),
            "{plugin} is configured"
        );
    }
}

#[test]
fn csp_allows_only_the_app_and_the_ggg_image_cdn() {
    let config = json("tauri.conf.json");
    let csp = config["app"]["security"]["csp"].as_object().unwrap();
    // The app itself: its origin, its IPC, and instantiating its WASM.
    let app_sources = [
        "'self'",
        "'none'",
        "'wasm-unsafe-eval'",
        "ipc:",
        "http://ipc.localhost",
    ];
    let mut foreign = BTreeSet::new();
    for sources in csp.values() {
        for source in sources.as_str().unwrap().split_whitespace() {
            if !app_sources.contains(&source) {
                foreign.insert(source);
            }
        }
    }
    assert_eq!(foreign, BTreeSet::from(["https://web.poecdn.com"]));
    assert!(
        csp["img-src"]
            .as_str()
            .unwrap()
            .contains("https://web.poecdn.com")
    );
    assert_eq!(csp["default-src"], "'self'");
    // Trunk's loader fetches the UI's WASM from the app's origin; without
    // 'self' a bundled build's window stays blank (devUrl runs have no CSP).
    assert!(
        csp["connect-src"]
            .as_str()
            .unwrap()
            .split_whitespace()
            .any(|source| source == "'self'")
    );
    assert!(
        config["app"]["security"]
            .get("dangerousDisableAssetCspModification")
            .is_none()
    );
}
