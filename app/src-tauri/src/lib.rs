//! Remeet menu-bar app.
//!
//! A tray icon that toggles a borderless cream popover. The popover's UI is the
//! static frontend under `../ui`; it talks to [`commands`] over Tauri's IPC, which in
//! turn drives `remeet-session`. There is no dock icon and no main window: this is a
//! menu-bar utility, so it runs with the macOS `Accessory` activation policy.

mod commands;
mod store;

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WindowEvent};
use tauri_plugin_positioner::{Position, WindowExt};

use commands::AppState;

/// The single popover window's label; matches `tauri.conf.json`.
const POPOVER: &str = "popover";

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_positioner::init())
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            commands::get_status,
            commands::list_recordings,
            commands::start_recording,
            commands::stop_recording,
            commands::get_transcript,
            commands::transcribe,
        ])
        .setup(|app| {
            // Menu-bar utility: no dock icon, no app-switcher presence.
            #[cfg(target_os = "macos")]
            app.handle()
                .set_activation_policy(tauri::ActivationPolicy::Accessory)?;

            build_tray(app.handle())?;
            hide_popover_on_blur(app.handle())?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Remeet");
}

/// Builds the menu-bar tray: a template glyph, a left-click that toggles the popover,
/// and a right-click menu holding Quit.
fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let quit = MenuItem::with_id(app, "quit", "Quit Remeet", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&quit])?;

    // A monochrome glyph flagged as a template image, so the menu bar tints it for
    // the current appearance instead of showing the raw pixels.
    let icon = Image::from_bytes(include_bytes!("../icons/tray.png"))?;

    TrayIconBuilder::with_id("remeet")
        .icon(icon)
        .icon_as_template(true)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            if event.id() == "quit" {
                app.exit(0);
            }
        })
        .on_tray_icon_event(|tray, event| {
            let app = tray.app_handle();
            // Record the tray's location so the positioner can place the popover.
            tauri_plugin_positioner::on_tray_event(app, &event);

            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_popover(app);
            }
        })
        .build(app)?;

    Ok(())
}

/// Shows the popover just under the tray icon, or hides it if already open.
fn toggle_popover(app: &AppHandle) {
    let Some(window) = app.get_webview_window(POPOVER) else {
        return;
    };

    if window.is_visible().unwrap_or(false) {
        let _ = window.hide();
    } else {
        let _ = window.move_window(Position::TrayCenter);
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Dismisses the popover when it loses focus, the way a native menu-bar popover does.
fn hide_popover_on_blur(app: &AppHandle) -> tauri::Result<()> {
    let Some(window) = app.get_webview_window(POPOVER) else {
        return Ok(());
    };

    let handle = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::Focused(false) = event {
            let _ = handle.hide();
        }
    });

    Ok(())
}
