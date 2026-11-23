//! The AppImage's menu entry on Linux. The deep-link plugin registers the
//! `exilence-ce-ng://` handler in `<binary>-handler.desktop`, hidden from
//! launchers, and an AppImage has no installer that adds a visible entry. So
//! the app writes that same file visible, with its icon, before the plugin
//! registers it: the plugin changes only `Exec` and `MimeType` of an
//! existing file, and leaves this one as it is.

use std::fs;
use std::io;
use std::path::Path;

use crate::auth::DEEP_LINK_SCHEME;

/// The icon, copied out of the AppImage, whose mount path changes at every
/// start.
const ICON: &[u8] = include_bytes!("../icons/128x128@2x.png");

/// The name launchers show, as the release assets spell it.
const NAME: &str = "Exilence-CE-NG";

/// Writes the menu entry and its icon under `data_home` (`$XDG_DATA_HOME`)
/// for the AppImage at `appimage`, whose binary is `binary`. Files that
/// already hold the same content stay untouched.
pub fn install(data_home: &Path, appimage: &Path, binary: &str) -> io::Result<()> {
    let icon = data_home
        .join("icons/hicolor/256x256/apps")
        .join(format!("{binary}.png"));
    write_if_changed(&icon, ICON)?;
    // GTK's program class, the binary with its first letter in upper case,
    // is the window's class (X11) and app ID (Wayland), so docks match the
    // window to this entry.
    let mut chars = binary.chars();
    let class: String = chars
        .next()
        .map(|first| first.to_ascii_uppercase())
        .into_iter()
        .chain(chars)
        .collect();
    // `Exec` as the plugin writes it, so it finds nothing to change.
    let entry = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name={NAME}\n\
         Exec=\"{exec}\" %u\n\
         Icon={icon}\n\
         Categories=Game;\n\
         StartupWMClass={class}\n\
         Terminal=false\n\
         MimeType=x-scheme-handler/{DEEP_LINK_SCHEME};\n",
        exec = appimage.display(),
        icon = icon.display(),
    );
    let file = data_home
        .join("applications")
        .join(format!("{binary}-handler.desktop"));
    write_if_changed(&file, entry.as_bytes())
}

fn write_if_changed(path: &Path, content: &[u8]) -> io::Result<()> {
    if fs::read(path).is_ok_and(|current| current == content) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What `tauri-plugin-deep-link` 2.6.3 writes on its own: hidden.
    const PLUGIN_ENTRY: &str = "[Desktop Entry]\nType=Application\nName=Exilence CE NG\n\
        Exec=\"/old/Exilence.AppImage\" %u\nTerminal=false\n\
        MimeType=x-scheme-handler/exilence-ce-ng\nNoDisplay=true\n";

    #[test]
    fn the_plugins_hidden_handler_becomes_a_visible_entry_with_an_icon() {
        let home = tempfile::tempdir().unwrap();
        let applications = home.path().join("applications");
        fs::create_dir_all(&applications).unwrap();
        let file = applications.join("exilence-ce-ng-handler.desktop");
        fs::write(&file, PLUGIN_ENTRY).unwrap();

        let appimage = Path::new("/home/me/Apps/Exilence CE NG.AppImage");
        install(home.path(), appimage, "exilence-ce-ng").unwrap();

        let icon = home
            .path()
            .join("icons/hicolor/256x256/apps/exilence-ce-ng.png");
        assert_eq!(fs::read(&icon).unwrap(), ICON);
        let entry = fs::read_to_string(&file).unwrap();
        assert!(!entry.contains("NoDisplay"), "{entry}");
        for line in [
            "Name=Exilence-CE-NG",
            "Exec=\"/home/me/Apps/Exilence CE NG.AppImage\" %u",
            &format!("Icon={}", icon.display()),
            "Categories=Game;",
            "StartupWMClass=Exilence-ce-ng",
            "MimeType=x-scheme-handler/exilence-ce-ng;",
        ] {
            assert!(entry.lines().any(|l| l == line), "{line} missing:\n{entry}");
        }
    }
}
