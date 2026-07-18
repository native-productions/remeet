//! Remeet app: a menu-bar popover plus a main window.
//!
//! Two surfaces, one binary. The tray icon toggles a borderless popover for the
//! capture flow — start, stop, glance — and the main window is the workspace for
//! everything that needs room. Both are rendered by the React frontend under
//! `../ui`, which picks its shell from the window label, and both talk to
//! [`commands`] over Tauri's IPC, which in turn drives `remeet-session`.
//!
//! The app idles as a macOS `Accessory` (menu-bar only, no dock icon) and switches
//! to `Regular` while the main window is open, because a real window without a dock
//! icon or app menu behaves like a bug.

mod commands;
mod settings;
mod spaces;
mod store;

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WindowEvent};
use tauri_plugin_positioner::{Position, WindowExt};

use commands::AppState;

/// Window labels; these match `tauri.conf.json` and the frontend's shell switch.
const POPOVER: &str = "popover";
const MAIN: &str = "main";

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_positioner::init())
        .invoke_handler(tauri::generate_handler![
            commands::get_status,
            commands::list_recordings,
            commands::start_recording,
            commands::stop_recording,
            commands::get_transcript,
            commands::transcribe,
            commands::prepare_audio,
            commands::delete_recording,
            commands::open_main_window,
            commands::get_settings,
            commands::save_settings,
            commands::settings_path,
            commands::probe_provider,
            commands::test_provider,
            commands::get_summary,
            commands::summarize,
            commands::list_spaces,
            commands::create_space,
            commands::rename_space,
            commands::delete_space,
            commands::set_active_space,
            commands::move_recording,
        ])
        .setup(|app| {
            // State is built here rather than in the builder chain because the app
            // config directory is only resolvable from a handle.
            let config_dir = app
                .path()
                .app_config_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."));
            app.manage(AppState::new(config_dir));

            // Idles as a menu-bar utility: no dock icon, no app-switcher presence.
            #[cfg(target_os = "macos")]
            app.handle()
                .set_activation_policy(tauri::ActivationPolicy::Accessory)?;

            build_tray(app.handle())?;
            hide_popover_on_blur(app.handle())?;
            accessory_when_main_closes(app.handle())?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Remeet");
}

/// Builds the menu-bar tray: a template glyph, a left-click that toggles the popover,
/// and a right-click menu holding Quit.
fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Remeet", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Remeet", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;

    // A monochrome glyph flagged as a template image, so the menu bar tints it for
    // the current appearance instead of showing the raw pixels.
    let icon = Image::from_bytes(include_bytes!("../icons/tray.png"))?;

    TrayIconBuilder::with_id("remeet")
        .icon(icon)
        .icon_as_template(true)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_main_window(app),
            "quit" => app.exit(0),
            _ => {}
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

/// Shows the main window, bringing the app forward as a regular app while it is up.
///
/// The window is declared hidden in `tauri.conf.json` and shown on demand, so the
/// workspace only exists once it has been asked for.
pub fn show_main_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window(MAIN) else {
        return;
    };

    // A window with no dock icon and no app menu cannot be cmd-tabbed back to once
    // it falls behind, so the policy follows the window rather than the process.
    #[cfg(target_os = "macos")]
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);

    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();

    // The popover is a transient surface; it should not linger over the window.
    if let Some(popover) = app.get_webview_window(POPOVER) {
        let _ = popover.hide();
    }
}

/// Drops back to a menu-bar-only app when the main window closes, so a dock icon
/// never outlives the window that justified it.
fn accessory_when_main_closes(app: &AppHandle) -> tauri::Result<()> {
    let Some(window) = app.get_webview_window(MAIN) else {
        return Ok(());
    };

    let handle = app.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::Destroyed | WindowEvent::CloseRequested { .. } = event {
            #[cfg(target_os = "macos")]
            let _ = handle.set_activation_policy(tauri::ActivationPolicy::Accessory);
            let _ = &handle;
        }
    });

    Ok(())
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
