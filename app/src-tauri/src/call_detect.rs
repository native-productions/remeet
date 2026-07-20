//! Turns "another app has a call live" into a reminder to record it.
//!
//! [`remeet_audio::CallWatcher`] does the CoreAudio detection; this module decides
//! whether the signal is worth surfacing, then emits [`CALL_DETECTED`] for the
//! frontend to act on — an in-window prompt plus an actionable notification, both
//! owned by the popover so a tap can start recording. Three gates keep it from
//! nagging:
//!
//! - not while Remeet is itself recording — its own capture lights up the same
//!   devices, and reminding someone to record what they are already recording is
//!   noise;
//! - not when the user has turned the reminder off in settings;
//! - not more than once per [`COOLDOWN`], so a call whose devices flap does not
//!   produce a burst.
//!
//! The detection is edge-triggered upstream, so a single call yields a single
//! `true`; the cooldown is a backstop against device churn within one call.
//!
//! The reminder is emitted, not shown, from here on purpose: the notification is
//! posted by the popover's (always-alive) webview so the OS click can call straight
//! into `start_recording`, which a notification posted from Rust could not do.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};

use remeet_audio::CallWatcher;

use crate::commands::AppState;

/// Shortest gap between two reminders. Long enough that a call which briefly drops
/// and re-acquires a device does not fire twice.
const COOLDOWN: Duration = Duration::from_secs(120);

/// Event the popover listens for to raise the record prompt and notification.
const CALL_DETECTED: &str = "call-detected";

/// Starts the detector. The returned watcher must be kept alive — it is managed by
/// Tauri so it lives for the app's lifetime and unregisters on shutdown.
pub fn start(app: &AppHandle) -> CallWatcher {
    let state = app.state::<AppState>();
    let recording = state.recording_flag();
    let config_dir = state.config_dir();

    let handle = app.clone();
    let last_fired: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));

    CallWatcher::start(move |active| {
        if !active {
            return;
        }
        maybe_remind(&handle, &recording, &config_dir, &last_fired);
    })
}

fn maybe_remind(
    app: &AppHandle,
    recording: &AtomicBool,
    config_dir: &std::path::Path,
    last_fired: &Mutex<Option<Instant>>,
) {
    // Remeet's own capture trips the same devices; never remind about that.
    if recording.load(Ordering::SeqCst) {
        return;
    }
    if !crate::settings::load(config_dir).call_reminder {
        return;
    }

    {
        let mut guard = match last_fired.lock() {
            Ok(guard) => guard,
            Err(_) => return,
        };
        if let Some(at) = *guard
            && at.elapsed() < COOLDOWN
        {
            return;
        }
        *guard = Some(Instant::now());
    }

    // The popover owns the reminder UI; it is alive from launch even while hidden.
    let _ = app.emit(CALL_DETECTED, ());
}
