// A build script that cannot do its job fails the build by panicking.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::env;

fn main() {
    // On Windows (MSVC) the Common Controls v6 manifest is linked into every
    // target, not only the app binary as tauri-build does: test binaries that
    // link Tauri otherwise die with STATUS_ENTRYPOINT_NOT_FOUND before running.
    let windows_msvc = env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    let mut windows = tauri_build::WindowsAttributes::new();
    if windows_msvc {
        windows = tauri_build::WindowsAttributes::new_without_app_manifest();
        let manifest = env::current_dir()
            .expect("build script runs in the crate directory")
            .join("windows-app-manifest.xml");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    }
    tauri_build::try_build(tauri_build::Attributes::new().windows_attributes(windows))
        .expect("failed to run tauri-build");
}
