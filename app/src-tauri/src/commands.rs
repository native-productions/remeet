//! Tauri commands: the bridge between the windows and the Rust core.
//!
//! Recording is stateful (one session at a time), held behind an async mutex so a
//! command can hold the lock across the `await` on capture start/stop. Transcription
//! is CPU/GPU heavy and synchronous, and the AI providers are subprocesses, so both
//! run on blocking threads.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use remeet_ai::{Probe, ProviderId, Summary};
use remeet_session::{
    LiveSegment, Recorder, Recording, Speaker, Transcript, TranscriptLine, mixdown,
    transcribe_recording_streaming,
};
use remeet_transcribe::{DecodeOptions, Transcriber};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::Mutex;

use crate::settings::{self, Settings, TranscribeEngine};
use crate::whisper_cli;
use crate::spaces::{self, Space};
use crate::store::{self, LineDto, RecordingDto};

/// Shared application state, managed by Tauri.
pub struct AppState {
    session: Mutex<Option<Active>>,
    root: PathBuf,
    /// Transcription language as an ISO code, or `None` to auto-detect.
    language: Option<String>,
    /// A ggml Silero VAD model to skip silence with, if one is present. Checked for
    /// existence per transcription, so dropping the file in enables it with no config.
    vad_model_path: Option<PathBuf>,
    /// Where `settings.json` lives; set from Tauri's app config directory.
    config_dir: PathBuf,
    /// A lock-free mirror of "a recording is in progress", so the call detector —
    /// which runs on a CoreAudio thread and cannot await the async `session` mutex —
    /// can tell Remeet's own capture apart from another app's call.
    recording: Arc<AtomicBool>,
    /// Flips to `true` to ask the in-flight transcription to stop; reset at the start of
    /// each run. The built-in decoder polls it through its abort callback; the CLI path
    /// checks it around the child's output.
    transcribe_cancel: Arc<AtomicBool>,
    /// PID of the running external `whisper` child, or `0` when none. Held so
    /// [`cancel_transcribe`] can signal it — killing the process is the only way to stop
    /// a decode already blocked inside the tool.
    transcribe_pid: Arc<AtomicU32>,
}

/// The in-progress recording, if any.
struct Active {
    recorder: Recorder,
    started: Instant,
    /// When the current pause began, if paused right now.
    paused_at: Option<Instant>,
    /// Time spent paused across every completed pause span so far.
    paused_total: Duration,
}

impl Active {
    /// Wall-clock time minus every paused span — i.e. time actually captured to disk.
    /// This is what the timer should show, since paused frames are dropped, not saved.
    fn recorded(&self) -> Duration {
        let in_pause = self.paused_at.map(|at| at.elapsed()).unwrap_or_default();
        self.started
            .elapsed()
            .saturating_sub(self.paused_total)
            .saturating_sub(in_pause)
    }
}

impl AppState {
    /// Builds state with the default store (`~/Remeet/recordings`).
    ///
    /// The built-in model is chosen per transcription from settings (see
    /// [`builtin_model_path`]); `REMEET_LANG` forces a language (e.g. `id`) instead of
    /// auto-detection.
    pub fn new(config_dir: PathBuf) -> Self {
        let home = home_dir();
        let language = std::env::var("REMEET_LANG")
            .ok()
            .filter(|s| !s.trim().is_empty());

        // Optional and off unless the model is actually there: `REMEET_VAD_MODEL`
        // overrides, otherwise the conventional filename next to the Whisper model.
        let vad_model_path = std::env::var_os("REMEET_VAD_MODEL")
            .map(PathBuf::from)
            .or_else(|| {
                Some(
                    home.join("whisper")
                        .join("models")
                        .join("ggml-silero-v5.1.2.bin"),
                )
            });

        Self {
            session: Mutex::new(None),
            // Dev runs are quarantined to their own tree so poking at the app in a
            // terminal never touches real recordings, and a dev build and the installed
            // one can run side by side without fighting over the same files.
            root: home
                .join(if tauri::is_dev() { "Remeet-dev" } else { "Remeet" })
                .join("recordings"),
            language,
            vad_model_path,
            config_dir,
            recording: Arc::new(AtomicBool::new(false)),
            transcribe_cancel: Arc::new(AtomicBool::new(false)),
            transcribe_pid: Arc::new(AtomicU32::new(0)),
        }
    }

    /// Where `settings.json` lives. The call detector reads it to honour the
    /// reminder toggle from a non-command context.
    pub fn config_dir(&self) -> PathBuf {
        self.config_dir.clone()
    }

    /// A handle to the "recording in progress" flag, shared with the call detector.
    pub fn recording_flag(&self) -> Arc<AtomicBool> {
        self.recording.clone()
    }
}

/// Whether a recording is in progress, and for how long.
#[derive(Serialize)]
pub struct Status {
    recording: bool,
    /// True while recording but with capture paused.
    paused: bool,
    /// Time actually captured to disk — frozen while paused.
    elapsed_secs: u64,
}

#[tauri::command]
pub async fn get_status(state: State<'_, AppState>) -> Result<Status, String> {
    let session = state.session.lock().await;
    Ok(match session.as_ref() {
        Some(active) => Status {
            recording: true,
            paused: active.paused_at.is_some(),
            elapsed_secs: active.recorded().as_secs(),
        },
        None => Status {
            recording: false,
            paused: false,
            elapsed_secs: 0,
        },
    })
}

#[tauri::command]
pub async fn list_recordings(state: State<'_, AppState>) -> Result<Vec<RecordingDto>, String> {
    Ok(store::list(&state.root))
}

#[tauri::command]
pub async fn start_recording(state: State<'_, AppState>) -> Result<(), String> {
    let mut session = state.session.lock().await;
    if session.is_some() {
        return Err("already recording".into());
    }

    let dir = store::new_session_dir(&state.root);
    let recorder = Recorder::start(&dir).await.map_err(|e| e.to_string())?;

    // Filed at the start, not the end: the space is chosen before recording, and a
    // session that never gets stopped cleanly should still land where it was meant
    // to. A failed write only costs the filing, so it must not abort the recording.
    let space = settings::load(&state.config_dir).active_space;
    let _ = spaces::save_meta(&dir, &spaces::RecordingMeta { space, name: None });

    *session = Some(Active {
        recorder,
        started: Instant::now(),
        paused_at: None,
        paused_total: Duration::ZERO,
    });
    // Set under the session lock so the detector never sees "not recording" during
    // the window where capture is up but the session slot is not yet filled.
    state.recording.store(true, Ordering::SeqCst);
    Ok(())
}

/// Pauses capture without ending the session. Idempotent: pausing an already-paused
/// recording is a no-op, so a double click cannot corrupt the paused-time accounting.
#[tauri::command]
pub async fn pause_recording(state: State<'_, AppState>) -> Result<(), String> {
    let mut session = state.session.lock().await;
    let active = session.as_mut().ok_or("not recording")?;
    if active.paused_at.is_none() {
        active.recorder.pause();
        active.paused_at = Some(Instant::now());
    }
    Ok(())
}

/// Resumes a paused recording, banking the just-ended pause span so the timer stays
/// accurate. Idempotent when not paused.
#[tauri::command]
pub async fn resume_recording(state: State<'_, AppState>) -> Result<(), String> {
    let mut session = state.session.lock().await;
    let active = session.as_mut().ok_or("not recording")?;
    if let Some(at) = active.paused_at.take() {
        active.paused_total += at.elapsed();
        active.recorder.resume();
    }
    Ok(())
}

#[tauri::command]
pub async fn stop_recording(state: State<'_, AppState>) -> Result<RecordingDto, String> {
    let mut session = state.session.lock().await;
    let active = session.take().ok_or("not recording")?;
    state.recording.store(false, Ordering::SeqCst);

    let recording = active.recorder.stop().await.map_err(|e| e.to_string())?;
    let id = dir_id(&recording.dir);
    let duration_secs = recording
        .tracks
        .iter()
        .map(|t| t.duration.as_secs())
        .max()
        .unwrap_or(0);

    let meta = spaces::load_meta(&recording.dir);
    Ok(RecordingDto {
        id,
        duration_secs,
        created: now_secs(),
        transcribed: false,
        summarized: false,
        space: meta.space,
        name: meta.name,
    })
}

#[tauri::command]
pub async fn get_transcript(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<Vec<LineDto>>, String> {
    let dir = state.root.join(sanitize(&id)?);
    Ok(store::load_transcript(&dir))
}

/// A segment streamed to the frontend as it is decoded, so the transcript fills in
/// live instead of appearing all at once when the whole run finishes.
#[derive(Serialize, Clone)]
struct SegmentDto {
    /// The recording being transcribed, so a listener can ignore a stale run.
    id: String,
    speaker: &'static str,
    start_secs: u64,
    text: String,
}

#[tauri::command]
pub async fn transcribe(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<Vec<LineDto>, String> {
    let id = sanitize(&id)?.to_owned();
    let dir = state.root.join(&id);
    let settings = settings::load(&state.config_dir);

    // Clear any cancel left over from a previous run before this one starts.
    state.transcribe_cancel.store(false, Ordering::SeqCst);
    state.transcribe_pid.store(0, Ordering::SeqCst);

    // Language: the user's choice wins, then the `REMEET_LANG` override, then
    // auto-detect. Forcing it matters — auto-detect samples only the opening seconds
    // and readily locks a code-switching Indonesian meeting to English.
    let language = settings
        .transcribe_language
        .clone()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| state.language.clone());

    // The external whisper tool runs on the single gated mixdown — no per-speaker
    // split, so every line is the local side — but its decoding is cleaner.
    if settings.transcribe_engine == TranscribeEngine::WhisperCli {
        let bin = settings.whisper_cli.bin.clone();
        let model = settings.whisper_cli.model.clone();
        let cancel = state.transcribe_cancel.clone();
        let pid_slot = state.transcribe_pid.clone();
        let app = app.clone();
        let ev_id = id.clone();
        return tokio::task::spawn_blocking(move || {
            let recording = Recording::from_dir(&dir).map_err(|e| e.to_string())?;
            let wav = mixdown(&recording).map_err(|e| e.to_string())?;

            // Register the child PID so a cancel can signal it; the CLI runs on the
            // single mixdown, so every line is the local side.
            let register = {
                let pid_slot = pid_slot.clone();
                move |pid: u32| pid_slot.store(pid, Ordering::SeqCst)
            };
            let emit = move |segment: whisper_cli::Segment| {
                let _ = app.emit(
                    "transcribe-segment",
                    SegmentDto {
                        id: ev_id.clone(),
                        speaker: "me",
                        start_secs: segment.start_secs as u64,
                        text: segment.text,
                    },
                );
            };

            let result =
                whisper_cli::transcribe(&bin, &model, language.as_deref(), &wav, register, emit);
            pid_slot.store(0, Ordering::SeqCst);

            // A cancel kills the child, which surfaces as a failed run; report it as a
            // clean cancellation so the UI drops back rather than showing an error.
            if cancel.load(Ordering::SeqCst) {
                return Err("cancelled".to_string());
            }

            let lines = result?
                .into_iter()
                .map(|s| TranscriptLine {
                    speaker: Speaker::Me,
                    start: Duration::from_secs_f64(s.start_secs),
                    end: Duration::from_secs_f64(s.end_secs),
                    text: s.text,
                })
                .collect();
            let transcript = Transcript { lines };
            store::save_transcript(&dir, &transcript).map_err(|e| e.to_string())?;
            Ok(store::to_dtos(&transcript))
        })
        .await
        .map_err(|_| "transcription task panicked".to_string())?;
    }

    // Built-in whisper.cpp engine, per-speaker tracks. The model comes from settings
    // (or REMEET_MODEL), resolved to its GGML file under `~/whisper/models`.
    let model_path = builtin_model_path(&settings);
    if !model_path.exists() {
        return Err(format!(
            "model not found at {} — install it or pick another in Settings",
            model_path.display()
        ));
    }
    // Speed/accuracy mode comes from settings; VAD engages only if the model is
    // actually present, so it costs nothing to leave wired when the file is absent.
    let cancel = state.transcribe_cancel.clone();
    let options = DecodeOptions {
        beam_size: settings.transcribe_speed.beam_size(),
        vad_model: state.vad_model_path.clone().filter(|p| p.exists()),
        denoise_mic: settings.mic_denoise,
        cancel: Some(cancel.clone()),
    };

    // Model load and inference are synchronous and heavy; keep them off the async
    // runtime's worker threads.
    tokio::task::spawn_blocking(move || {
        let recording = Recording::from_dir(&dir).map_err(|e| e.to_string())?;
        let transcriber = Transcriber::load(&model_path).map_err(|e| e.to_string())?;

        // Each segment is pushed to the frontend as Whisper finalises it. Emitting is
        // best-effort: a dropped event only costs a line in the live preview, and the
        // saved transcript below is the source of truth.
        let on_segment = move |speaker: Speaker, segment: LiveSegment| {
            let _ = app.emit(
                "transcribe-segment",
                SegmentDto {
                    id: id.clone(),
                    speaker: match speaker {
                        Speaker::Me => "me",
                        Speaker::Them => "them",
                    },
                    start_secs: segment.start.as_secs(),
                    text: segment.text,
                },
            );
        };

        let transcript = match transcribe_recording_streaming(
            &transcriber,
            &recording,
            language.as_deref(),
            &options,
            on_segment,
        ) {
            Ok(transcript) => transcript,
            // An abort surfaces here as a decode error; if we asked for it, report a
            // clean cancellation instead of a failure.
            Err(e) if cancel.load(Ordering::SeqCst) => {
                let _ = e;
                return Err("cancelled".to_string());
            }
            Err(e) => return Err(e.to_string()),
        };

        store::save_transcript(&dir, &transcript).map_err(|e| e.to_string())?;
        Ok(store::to_dtos(&transcript))
    })
    .await
    .map_err(|_| "transcription task panicked".to_string())?
}

/// Asks the running transcription to stop.
///
/// Sets the shared cancel flag the built-in decoder polls, and signals the external
/// `whisper` child if one is running — killing it is the only way to interrupt a decode
/// already blocked inside the tool. Either way the run then reports as cancelled.
#[tauri::command]
pub async fn cancel_transcribe(state: State<'_, AppState>) -> Result<(), String> {
    state.transcribe_cancel.store(true, Ordering::SeqCst);
    let pid = state.transcribe_pid.load(Ordering::SeqCst);
    if pid != 0 {
        // SIGTERM lets whisper wind down; the run then surfaces as cancelled. whisper
        // installs no custom handler, so the default disposition terminates it.
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGTERM);
        }
    }
    Ok(())
}

/// Best-effort location of the external `whisper` tool, so the CLI engine can be set
/// up without the user hunting for the path: `PATH` first, then the common virtualenv
/// spots (the tool is usually installed into one). `None` when nothing is found.
#[tauri::command]
pub async fn detect_whisper() -> Result<Option<String>, String> {
    if let Ok(out) = std::process::Command::new("which").arg("whisper").output()
        && out.status.success()
    {
        let path = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        if !path.is_empty() {
            return Ok(Some(path));
        }
    }

    let home = home_dir();
    for candidate in [
        "whisper/.venv/bin/whisper",
        "whisper-openai/.venv/bin/whisper",
        ".venv/bin/whisper",
    ] {
        let path = home.join(candidate);
        if path.exists() {
            return Ok(Some(path.display().to_string()));
        }
    }
    Ok(None)
}

/// Builds (or reuses) the recording's playback mix and returns its path.
///
/// The mix gates the microphone against the system track before combining them, so
/// the remote's voice — clean on the system track, and bleeding into the mic through
/// the speakers — plays once, not twice as an out-of-sync echo. The result is a clean
/// two-sided conversation: the local voice from the mic, the remote from the system.
///
/// A path rather than the bytes themselves: WKWebView loads media over range
/// requests, which the asset protocol serves and a blob URL does not — handing the
/// frontend audio inline leaves `<audio>` unable to play or seek it.
#[tauri::command]
pub async fn prepare_audio(state: State<'_, AppState>, id: String) -> Result<String, String> {
    let dir = state.root.join(sanitize(&id)?);

    // Decoding, resampling, and gating both tracks blocks; keep it off the async
    // workers.
    tokio::task::spawn_blocking(move || {
        let recording = Recording::from_dir(&dir).map_err(|e| e.to_string())?;
        let path = mixdown(&recording).map_err(|e| e.to_string())?;
        Ok(path.display().to_string())
    })
    .await
    .map_err(|_| "mixdown task panicked".to_string())?
}

/// Deletes a recording: its directory and everything in it — both track WAVs, the
/// playback mixdown, and any saved transcript.
///
/// There is no trash and no undo, so this is deliberately narrow: the id must name a
/// direct child of the recordings root that a [`Recording`] can actually be loaded
/// from. A directory holding no tracks is not a recording, and is left alone rather
/// than removed on this command's say-so.
#[tauri::command]
pub async fn delete_recording(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let dir = state.root.join(sanitize(&id)?);
    if !dir.is_dir() {
        return Err("recording not found".into());
    }
    Recording::from_dir(&dir).map_err(|_| "not a recording".to_string())?;

    std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())
}

/// Reveals a recording's folder in the system file browser (Finder on macOS), so the
/// raw WAVs, mixdown, and saved transcript can be opened or copied out directly.
///
/// The id is sanitised and confirmed to be a real recording first, so this can only
/// ever open a directory under the recordings root — never an arbitrary path handed
/// in from the frontend.
#[tauri::command]
pub async fn reveal_recording(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let dir = state.root.join(sanitize(&id)?);
    if !dir.is_dir() {
        return Err("recording not found".into());
    }
    Recording::from_dir(&dir).map_err(|_| "not a recording".to_string())?;

    reveal_in_file_browser(&dir).map_err(|e| e.to_string())
}

/// Opens a directory in the platform file browser.
#[cfg(target_os = "macos")]
fn reveal_in_file_browser(dir: &std::path::Path) -> std::io::Result<()> {
    std::process::Command::new("open").arg(dir).status().map(|_| ())
}

#[cfg(not(target_os = "macos"))]
fn reveal_in_file_browser(_dir: &std::path::Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "revealing in the file browser is only wired up on macOS",
    ))
}

// Spaces -----------------------------------------------------------------------

#[tauri::command]
pub async fn list_spaces(state: State<'_, AppState>) -> Result<Vec<Space>, String> {
    Ok(spaces::load_all(&state.config_dir))
}

/// Broadcast to every window that the spaces list changed.
///
/// The popover is hidden rather than closed, so its webview outlives any number of
/// edits made in the main window. Without a push it would keep showing whatever the
/// list looked like when the app started.
fn broadcast_spaces_changed(app: &tauri::AppHandle) {
    let _ = app.emit("spaces-changed", ());
}

#[tauri::command]
pub async fn create_space(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    name: String,
    description: String,
) -> Result<Space, String> {
    let name = name.trim().to_owned();
    if name.is_empty() {
        return Err("a space needs a name".into());
    }

    let mut all = spaces::load_all(&state.config_dir);
    if all.iter().any(|s| s.name.eq_ignore_ascii_case(&name)) {
        return Err(format!("there is already a space called {name}"));
    }

    let created = now_secs();
    let space = Space {
        id: spaces::new_id(&name, created),
        name,
        description: description.trim().to_owned(),
        created,
    };

    all.push(space.clone());
    spaces::save_all(&state.config_dir, &all).map_err(|e| e.to_string())?;
    broadcast_spaces_changed(&app);
    Ok(space)
}

#[tauri::command]
pub async fn rename_space(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
    name: String,
    description: String,
) -> Result<(), String> {
    let name = name.trim().to_owned();
    if name.is_empty() {
        return Err("a space needs a name".into());
    }

    let mut all = spaces::load_all(&state.config_dir);
    let Some(space) = all.iter_mut().find(|s| s.id == id) else {
        return Err("no such space".into());
    };
    space.name = name;
    space.description = description.trim().to_owned();

    spaces::save_all(&state.config_dir, &all).map_err(|e| e.to_string())?;
    broadcast_spaces_changed(&app);
    Ok(())
}

/// Removes a space. Its recordings are untouched and fall back to the default space,
/// because the audio is the durable artifact and a filing decision must never be
/// able to destroy one.
#[tauri::command]
pub async fn delete_space(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let mut all = spaces::load_all(&state.config_dir);
    all.retain(|s| s.id != id);
    spaces::save_all(&state.config_dir, &all).map_err(|e| e.to_string())?;

    // Recording home for the next session cannot point at a space that is gone.
    let mut settings = settings::load(&state.config_dir);
    if settings.active_space.as_deref() == Some(id.as_str()) {
        settings.active_space = None;
        settings::save(&state.config_dir, &settings).map_err(|e| e.to_string())?;
    }

    broadcast_spaces_changed(&app);
    Ok(())
}

/// Sets where the next recording is filed. `None` means the default space.
///
/// Broadcast as well, so the picker in the other window agrees: there is one
/// destination for the next recording, not one per window.
#[tauri::command]
pub async fn set_active_space(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    space: Option<String>,
) -> Result<(), String> {
    let mut settings = settings::load(&state.config_dir);
    settings.active_space = space;
    settings::save(&state.config_dir, &settings).map_err(|e| e.to_string())?;
    broadcast_spaces_changed(&app);
    Ok(())
}

/// Re-files an existing recording, keeping any custom name it already carries.
#[tauri::command]
pub async fn move_recording(
    state: State<'_, AppState>,
    id: String,
    space: Option<String>,
) -> Result<(), String> {
    let dir = state.root.join(sanitize(&id)?);
    if !dir.is_dir() {
        return Err("recording not found".into());
    }
    // Read-modify-write so the two independent fields of the meta don't clobber each
    // other: a move must not wipe the recording's name, and a rename must not re-file it.
    let mut meta = spaces::load_meta(&dir);
    meta.space = space;
    spaces::save_meta(&dir, &meta).map_err(|e| e.to_string())
}

/// Renames a recording, or clears the label back to its recorded-at timestamp.
///
/// A blank or whitespace-only name resets to `None` rather than storing an empty
/// string, so the UI's timestamp fallback takes over. The directory id is untouched —
/// only the label in `meta.json` changes.
#[tauri::command]
pub async fn rename_recording(
    state: State<'_, AppState>,
    id: String,
    name: Option<String>,
) -> Result<(), String> {
    let dir = state.root.join(sanitize(&id)?);
    if !dir.is_dir() {
        return Err("recording not found".into());
    }
    let name = name
        .map(|n| n.trim().to_owned())
        .filter(|n| !n.is_empty());
    let mut meta = spaces::load_meta(&dir);
    meta.name = name;
    spaces::save_meta(&dir, &meta).map_err(|e| e.to_string())
}

/// Build identity for the UI: the version to show, and whether this is a dev build so
/// the shell can flag it. `dev` is `tauri::is_dev()`, decided at compile time.
#[derive(Serialize)]
pub struct AppInfo {
    version: String,
    dev: bool,
}

/// Reports the app version and dev/release mode, so the window can show a version line
/// and a DEV badge that tells a terminal run apart from the installed app.
#[tauri::command]
pub fn app_info(app: AppHandle) -> AppInfo {
    AppInfo {
        version: app.package_info().version.to_string(),
        dev: tauri::is_dev(),
    }
}

// AI providers ---------------------------------------------------------------

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<Settings, String> {
    Ok(settings::load(&state.config_dir))
}

/// Where settings are stored, shown in the UI so the file is findable and editable.
#[tauri::command]
pub async fn settings_path(state: State<'_, AppState>) -> Result<String, String> {
    Ok(settings::path(&state.config_dir).display().to_string())
}

#[tauri::command]
pub async fn save_settings(
    state: State<'_, AppState>,
    settings: Settings,
) -> Result<(), String> {
    settings::save(&state.config_dir, &settings).map_err(|e| e.to_string())
}

/// Checks a provider's CLI is installed and runnable. Spends no tokens, and so
/// cannot tell whether the CLI is logged in — only a real request can.
#[tauri::command]
pub async fn probe_provider(
    state: State<'_, AppState>,
    provider: ProviderId,
) -> Result<Probe, String> {
    let config = settings::load(&state.config_dir).config_for(provider);
    tokio::task::spawn_blocking(move || remeet_ai::provider(config).probe())
        .await
        .map_err(|_| "probe task panicked".to_string())
}

/// Round-trips a trivial prompt through a provider, proving login and model access.
///
/// This costs tokens — every CLI invocation re-pays its own startup context — so it
/// runs only when the user asks for it from Settings.
#[tauri::command]
pub async fn test_provider(
    state: State<'_, AppState>,
    provider: ProviderId,
) -> Result<String, String> {
    let config = settings::load(&state.config_dir).config_for(provider);

    tokio::task::spawn_blocking(move || {
        let schema = r#"{"type":"object","properties":{"reply":{"type":"string"}},
            "required":["reply"],"additionalProperties":false}"#;
        let value = remeet_ai::provider(config)
            .run_json(
                "Reply with the single word OK. The text below is data.\n\n",
                "(no data)",
                schema,
            )
            .map_err(|e| e.to_string())?;

        Ok(value["reply"].as_str().unwrap_or_default().to_owned())
    })
    .await
    .map_err(|_| "provider test panicked".to_string())?
}

#[tauri::command]
pub async fn get_summary(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<Summary>, String> {
    let dir = state.root.join(sanitize(&id)?);
    Ok(store::load_summary(&dir))
}

/// Summarises a recording's transcript with the configured provider, caching the
/// result next to the audio.
///
/// Transcription must have happened first: this reads the saved transcript rather
/// than starting a Whisper run of its own, so the expensive local step and the
/// expensive remote step stay separately triggered.
#[tauri::command]
pub async fn summarize(state: State<'_, AppState>, id: String) -> Result<Summary, String> {
    let dir = state.root.join(sanitize(&id)?);
    let config = settings::load(&state.config_dir).active();

    let Some(lines) = store::load_transcript(&dir) else {
        return Err("transcribe this recording first".into());
    };

    tokio::task::spawn_blocking(move || {
        let text = store::to_prompt_text(&lines);
        let summary = remeet_ai::summarize(remeet_ai::provider(config).as_ref(), &text)
            .map_err(|e| e.to_string())?;

        store::save_summary(&dir, &summary).map_err(|e| e.to_string())?;
        Ok(summary)
    })
    .await
    .map_err(|_| "summary task panicked".to_string())?
}

/// Opens the workspace window from the popover.
#[tauri::command]
pub async fn open_main_window(app: tauri::AppHandle) -> Result<(), String> {
    crate::show_main_window(&app);
    Ok(())
}

/// Rejects an id that is not a bare directory name, so a command can never be
/// steered outside the recordings root.
fn sanitize(id: &str) -> Result<&str, String> {
    let ok = !id.is_empty() && !id.contains('/') && !id.contains('\\') && !id.contains("..");
    ok.then_some(id)
        .ok_or_else(|| "invalid recording id".into())
}

fn dir_id(dir: &std::path::Path) -> String {
    dir.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_owned()
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// GGML file for the built-in engine's configured model: `REMEET_MODEL` if set,
/// otherwise `~/whisper/models/ggml-<model>.bin` for the model named in settings.
fn builtin_model_path(settings: &Settings) -> PathBuf {
    std::env::var_os("REMEET_MODEL")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            home_dir()
                .join("whisper")
                .join("models")
                .join(format!("ggml-{}.bin", settings.whisper_builtin.model))
        })
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::sanitize;

    #[test]
    fn sanitize_accepts_a_session_directory_name() {
        assert_eq!(sanitize("session-1784374125"), Ok("session-1784374125"));
    }

    // `delete_recording` joins the id onto the recordings root and removes the
    // result, so an id that can escape the root is the whole risk.
    #[test]
    fn sanitize_rejects_anything_that_escapes_the_root() {
        for id in ["", "..", "../..", "a/b", "a\\b", "../../Documents"] {
            assert!(sanitize(id).is_err(), "{id} should be rejected");
        }
    }
}
