// No console window next to the app in Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // SAFETY: first in `main`; no other thread runs yet.
    unsafe { exilence_ce_ng_lib::shell::apply_env_defaults() };
    exilence_ce_ng_lib::run()
}
