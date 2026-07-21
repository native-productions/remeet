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

mod call_detect;
mod commands;
mod settings;
mod spaces;
mod store;
mod whisper_cli;

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, RunEvent, WindowEvent};
use tauri_plugin_positioner::{Position, WindowExt};

use commands::AppState;

/// Window labels; these match `tauri.conf.json` and the frontend's shell switch.
const POPOVER: &str = "popover";
const MAIN: &str = "main";

/// Set once the app has begun exiting, so the abort guard can tell the known
/// teardown bug apart from a genuine runtime fault. See [`install_exit_guard`].
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_positioner::init())
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![
            commands::get_status,
            commands::list_recordings,
            commands::start_recording,
            commands::pause_recording,
            commands::resume_recording,
            commands::stop_recording,
            commands::get_transcript,
            commands::transcribe,
            commands::cancel_transcribe,
            commands::app_info,
            commands::prepare_audio,
            commands::delete_recording,
            commands::reveal_recording,
            commands::open_main_window,
            commands::get_settings,
            commands::save_settings,
            commands::settings_path,
            commands::detect_whisper,
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
            commands::rename_recording,
        ])
        .setup(|app| {
            // State is built here rather than in the builder chain because the app
            // config directory is only resolvable from a handle.
            let mut config_dir = app
                .path()
                .app_config_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."));
            // Dev keeps its settings and spaces out of the installed app's config, so a
            // terminal run can be reconfigured freely without disturbing the real one.
            if tauri::is_dev() {
                config_dir.push("dev");
            }
            app.manage(AppState::new(config_dir));

            // Idles as a menu-bar utility: no dock icon, no app-switcher presence.
            #[cfg(target_os = "macos")]
            app.handle()
                .set_activation_policy(tauri::ActivationPolicy::Accessory)?;

            build_tray(app.handle())?;
            hide_popover_on_blur(app.handle())?;
            accessory_when_main_closes(app.handle())?;
            make_main_movable_by_background(app.handle());

            // Watches for a call on the mic + speakers so a forgotten recording gets
            // a nudge. Managed so it outlives `setup` and unregisters on shutdown;
            // it reads state, so it starts after the state is managed.
            app.manage(call_detect::start(app.handle()));

            install_exit_guard();
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Remeet")
        .run(|_app, event| {
            if let RunEvent::ExitRequested { .. } = event {
                // From here on, a SIGABRT is the ggml Metal teardown bug rather than a
                // live fault, so the guard is allowed to swallow it. Set before the
                // C++ static destructors run at `exit()`.
                SHUTTING_DOWN.store(true, Ordering::SeqCst);
            }
        });
}

/// Installs a SIGABRT handler that turns the whisper.cpp/ggml Metal teardown abort
/// into a clean exit.
///
/// whisper.cpp keeps a global Metal device that is torn down from a C++ static
/// destructor at `exit()`. On this machine `ggml_metal_rsets_free` fails a
/// `GGML_ASSERT` there and calls `abort()`, so every quit *after* a transcription
/// crashes with SIGABRT — nothing the Rust side does per call can prevent a free that
/// runs during process finalization.
///
/// The guard only acts once [`SHUTTING_DOWN`] is set, so a genuine runtime abort
/// still crashes and produces a report; only the teardown abort is swallowed.
fn install_exit_guard() {
    extern "C" fn on_abort(_signal: libc::c_int) {
        if SHUTTING_DOWN.load(Ordering::SeqCst) {
            // Async-signal-safe: end the process without running further finalizers.
            unsafe { libc::_exit(0) };
        }
        // A real abort: restore the default disposition and re-raise so it crashes
        // and reports exactly as it would have without this guard.
        unsafe {
            libc::signal(libc::SIGABRT, libc::SIG_DFL);
            libc::raise(libc::SIGABRT);
        }
    }

    // SAFETY: registers a minimal, async-signal-safe handler for one signal.
    unsafe {
        libc::signal(libc::SIGABRT, on_abort as *const () as libc::sighandler_t);
    }
}

/// Makes the main window movable by dragging its background.
///
/// The window has a transparent, overlaid title bar and the webview fills under it, so
/// the only thing that moves the window is the JS drag region. That handles click-drag,
/// but the macOS three-finger-drag gesture is a WindowServer-level event that never
/// reaches the webview — the native window itself has to be movable by its background
/// for the gesture to work. Setting that one AppKit flag enables it; controls still
/// take their own clicks, so nothing else changes.
///
/// The popover is deliberately left out: it is positioned under the tray icon and must
/// not wander.
fn make_main_movable_by_background(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    {
        use objc2::msg_send;
        use objc2::runtime::AnyObject;

        let Some(window) = app.get_webview_window(MAIN) else {
            return;
        };
        let Ok(ns_window) = window.ns_window() else {
            return;
        };
        let ns_window = ns_window as *mut AnyObject;
        if ns_window.is_null() {
            return;
        }
        // SAFETY: `ns_window` is the live `NSWindow` for the main window, and this runs
        // on the main thread inside `setup`. The setter takes a `BOOL` and returns void.
        unsafe {
            let _: () = msg_send![ns_window, setMovableByWindowBackground: true];
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = app;
}

/// Builds the menu-bar tray: a template glyph, a left-click that toggles the popover,
/// and a right-click menu holding Quit.
fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    // A greyed line naming the build, so the version — and whether this is a dev run —
    // is legible straight from the menu bar, where two identical icons would otherwise
    // be indistinguishable.
    let version = app.package_info().version.to_string();
    let build_label = if tauri::is_dev() {
        format!("Remeet v{version} · dev")
    } else {
        format!("Remeet v{version}")
    };
    let version_item = MenuItem::with_id(app, "version", &build_label, false, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", "Open Remeet", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Remeet", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&version_item, &open, &quit])?;

    // A monochrome glyph flagged as a template image, so the menu bar tints it for
    // the current appearance instead of showing the raw pixels.
    let icon = Image::from_bytes(include_bytes!("../icons/tray.png"))?;

    TrayIconBuilder::with_id("remeet")
        .icon(icon)
        .icon_as_template(true)
        .tooltip(&build_label)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_main_window(app),
            "quit" => {
                SHUTTING_DOWN.store(true, Ordering::SeqCst);
                app.exit(0);
            }
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
