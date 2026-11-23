//! What the core asks of the desktop: the system browser, the clipboard, the
//! file manager and save dialogs. Tests replace it, so they open no browser,
//! touch no clipboard and show no dialog.

use std::path::{Path, PathBuf};
use std::pin::Pin;

use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_clipboard_manager::ClipboardExt;

use crate::shell::MAIN_WINDOW;

/// Where the user chose to save a file; `None` when they cancelled.
pub type SavePath = Pin<Box<dyn Future<Output = Result<Option<PathBuf>, String>> + Send>>;

pub trait Desktop: Send + Sync {
    /// Opens `url` in the system browser.
    fn open_url(&self, url: &str) -> Result<(), String>;
    /// Puts `text` on the clipboard.
    fn copy_text(&self, text: &str) -> Result<(), String>;
    /// Opens the directory `dir` in the file manager.
    fn open_dir(&self, dir: &Path) -> Result<(), String>;
    /// Asks where to save a file, suggesting `file_name`, in a save dialog
    /// over the main window.
    fn save_path(&self, file_name: String) -> SavePath;
}

/// The real desktop: the system's URL handler, the clipboard plugin and the
/// native save dialog.
pub struct Plugins<R: Runtime>(pub AppHandle<R>);

impl<R: Runtime> Desktop for Plugins<R> {
    fn open_url(&self, url: &str) -> Result<(), String> {
        open::that_detached(url).map_err(|e| e.to_string())
    }

    fn copy_text(&self, text: &str) -> Result<(), String> {
        self.0
            .clipboard()
            .write_text(text)
            .map_err(|e| e.to_string())
    }

    fn open_dir(&self, dir: &Path) -> Result<(), String> {
        open::that_detached(dir).map_err(|e| e.to_string())
    }

    fn save_path(&self, file_name: String) -> SavePath {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let window = self.0.get_webview_window(MAIN_WINDOW);
        // Built on the main thread, as GTK and AppKit need; awaited here.
        let posted = self.0.run_on_main_thread(move || {
            let mut dialog = rfd::AsyncFileDialog::new().set_file_name(file_name);
            if let Some(window) = &window {
                dialog = dialog.set_parent(window);
            }
            let _ = tx.send(dialog.save_file());
        });
        Box::pin(async move {
            posted.map_err(|e| e.to_string())?;
            let dialog = rx.await.map_err(|e| e.to_string())?;
            Ok(dialog.await.map(|file| file.path().to_owned()))
        })
    }
}
