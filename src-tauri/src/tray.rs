//! The tray icon and its menu (F15). Its text comes from the shared locale
//! resources; the update sublabel follows the `update` slice (D13).

use ipc::slices::{Update, UpdateState};
use ipc::{Event, Locale, Message, MessageKey, SliceName, links};
use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Runtime};

use crate::shell::show_main_window;
use crate::state::{self, Store};
use crate::{slot, updater};

const SHOW: &str = "show";
const CHECK_FOR_UPDATES: &str = "check_for_updates";
const UPDATE_SUBLABEL: &str = "update_sublabel";
const DONATE: &str = "donate";
const DISCORD_HELP: &str = "discord_help";
const QUIT: &str = "quit";

/// Creates the tray icon.
pub fn create<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let locale = Locale::english();
    let item = |id: &str, key: MessageKey, enabled: bool| {
        MenuItem::with_id(app, id, locale.key(key), enabled, None::<&str>)
    };
    let update = app.state::<Store>().read(|u: &Update| u.state.clone());
    // Menus have no sublabels on Windows and Linux: the state is a disabled
    // item right below "Check for Updates…".
    let sublabel = MenuItem::with_id(
        app,
        UPDATE_SUBLABEL,
        locale.text(&update_sublabel(&update)),
        false,
        None::<&str>,
    )?;
    let menu = Menu::with_items(
        app,
        &[
            &item(SHOW, MessageKey::TrayShow, true)?,
            // Only builds that check for updates can check on demand.
            &item(
                CHECK_FOR_UPDATES,
                MessageKey::TrayCheckForUpdates,
                updater::ENABLED,
            )?,
            &sublabel,
            &PredefinedMenuItem::separator(app)?,
            &item(DONATE, MessageKey::TrayDonate, true)?,
            &item(DISCORD_HELP, MessageKey::TrayDiscordHelp, true)?,
            &PredefinedMenuItem::separator(app)?,
            &item(QUIT, MessageKey::TrayQuit, true)?,
        ],
    )?;
    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip(locale.key(MessageKey::AppTitle))
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(on_menu_event)
        .on_tray_icon_event(on_tray_icon_event);
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;

    // Handlers run while the event goes out: the menu, which waits for the
    // main thread, changes there later.
    let main = app.clone();
    state::listen(app, SliceName::Update.event_name(), move |event| {
        let Ok(Event::Update(update)) = serde_json::from_str(event.payload()) else {
            return;
        };
        let sublabel = sublabel.clone();
        let text = locale.text(&update_sublabel(&update.state));
        let changed = main.run_on_main_thread(move || {
            if let Err(error) = sublabel.set_text(text) {
                log::error!("cannot update the tray's update sublabel: {error}");
            }
        });
        if let Err(error) = changed {
            log::error!("cannot update the tray's update sublabel: {error}");
        }
    });
    Ok(())
}

/// The tray's update sublabel: what the updater actually has (D13).
pub fn update_sublabel(state: &UpdateState) -> Message {
    match state {
        UpdateState::None => MessageKey::TrayUpdateNone.into(),
        UpdateState::Available { version } => {
            Message::new(MessageKey::TrayUpdateAvailable).arg("version", version.as_str())
        }
        UpdateState::Downloading { version, percent } => {
            Message::new(MessageKey::TrayUpdateDownloading)
                .arg("version", version.as_str())
                .arg("percent", u32::from(*percent))
        }
        UpdateState::Ready { version } => {
            Message::new(MessageKey::TrayUpdateReady).arg("version", version.as_str())
        }
    }
}

fn on_menu_event<R: Runtime>(app: &AppHandle<R>, event: MenuEvent) {
    let opened = match event.id().as_ref() {
        SHOW => {
            show_main_window(app);
            Ok(())
        }
        CHECK_FOR_UPDATES => {
            updater::check_now(app);
            Ok(())
        }
        DONATE => crate::links::open(links::KOFI),
        DISCORD_HELP => crate::links::open(links::DISCORD),
        QUIT => {
            // A running Snapshot is cancelled first, so it never ends half
            // done (client-background-work.md, operation slot).
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let _held = slot::cancel_and_hold(&app).await;
                app.exit(0);
            });
            Ok(())
        }
        _ => Ok(()),
    };
    if let Err(error) = opened {
        log::error!("{error}");
    }
}

/// A left click shows the main window. Linux reports no clicks: there every
/// click opens the menu.
fn on_tray_icon_event<R: Runtime>(tray: &TrayIcon<R>, event: TrayIconEvent) {
    if let TrayIconEvent::Click {
        button: MouseButton::Left,
        button_state: MouseButtonState::Up,
        ..
    } = event
    {
        show_main_window(tray.app_handle());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn d13_tray_update_sublabel_reflects_the_update_state() {
        let locale = Locale::english();
        let text = |state: UpdateState| locale.text(&update_sublabel(&state));
        assert_eq!(text(UpdateState::None), "No updates available");
        assert_eq!(
            text(UpdateState::Available {
                version: "2.0.1".to_owned()
            }),
            "Update 2.0.1 available"
        );
        assert_eq!(
            text(UpdateState::Downloading {
                version: "2.0.1".to_owned(),
                percent: 40
            }),
            "Downloading update 2.0.1: 40 %"
        );
        assert_eq!(
            text(UpdateState::Ready {
                version: "2.0.1".to_owned()
            }),
            "Update 2.0.1 ready to install"
        );
    }
}
