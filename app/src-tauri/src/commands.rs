//! Tauri commands: the bridge between the windows and the Rust core.
//!
//! Recording is stateful (one session at a time), held behind an async mutex so a
//! command can hold the lock across the `await` on capture start/stop. Transcription
//! is CPU/GPU heavy and synchronous, and the AI providers are subprocesses, so both
//! run on blocking threads.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use remeet_ai::{Probe, ProviderId, Summary};
use remeet_session::{
    LiveSegment, Recorder, Recording, Speaker, transcribe_recording_streaming,
};
use remeet_transcribe::{DecodeOptions, Transcriber};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::Mutex;

use crate::settings::{self, Settings};
use crate::spaces::{self, Space};
use crate::store::{self, LineDto, RecordingDto};

/// Shared application state, managed by Tauri.
pub struct AppState {
    session: Mutex<Option<Active>>,
    root: PathBuf,
    model_path: PathBuf,
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
    /// Builds state with the default store (`~/Remeet/recordings`) and the full
    /// `large-v3` model, which transcribes non-English speech far more accurately
    /// than the distilled `turbo` variant.
    ///
    /// Both are overridable: `REMEET_MODEL` points at a different GGML model, and
    /// `REMEET_LANG` forces a language (e.g. `id`) instead of auto-detection.
    pub fn new(config_dir: PathBuf) -> Self {
        let home = home_dir();
        let model_path = std::env::var_os("REMEET_MODEL")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                home.join("whisper")
                    .join("models")
                    .join("ggml-large-v3.bin")
            });

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
            root: home.join("Remeet").join("recordings"),
            model_path,
            language,
            vad_model_path,
            config_dir,
            recording: Arc::new(AtomicBool::new(false)),
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
    let _ = spaces::save_meta(&dir, &spaces::RecordingMeta { space });

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

    Ok(RecordingDto {
        id,
        duration_secs,
        created: now_secs(),
        transcribed: false,
        summarized: false,
        space: spaces::load_meta(&recording.dir).space,
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
    let model_path = state.model_path.clone();

    if !model_path.exists() {
        return Err(format!("model not found at {}", model_path.display()));
    }

    let settings = settings::load(&state.config_dir);

    // Language: the user's choice wins, then the `REMEET_LANG` override, then
    // auto-detect. Forcing it matters — Whisper's auto-detect samples only the opening
    // seconds and readily locks a code-switching Indonesian meeting to English.
    let language = settings
        .transcribe_language
        .clone()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| state.language.clone());

    // Speed/accuracy mode comes from settings; VAD engages only if the model is
    // actually present, so it costs nothing to leave wired when the file is absent.
    let options = DecodeOptions {
        beam_size: settings.transcribe_speed.beam_size(),
        vad_model: state.vad_model_path.clone().filter(|p| p.exists()),
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

        let transcript = transcribe_recording_streaming(
            &transcriber,
            &recording,
            language.as_deref(),
            &options,
            on_segment,
        )
        .map_err(|e| e.to_string())?;

        store::save_transcript(&dir, &transcript).map_err(|e| e.to_string())?;
        Ok(store::to_dtos(&transcript))
    })
    .await
    .map_err(|_| "transcription task panicked".to_string())?
}

/// Returns the path to the recording's playback audio for the player.
///
/// The microphone track, not a mixdown of both tracks. On speakers the mic already
/// captures the whole call — the local voice plus the remote coming back through the
/// speakers — so it plays as one coherent take. The mixdown instead overlays the mic
/// and system tracks, and since the remote is present on both, it doubles into an
/// echo, slightly out of sync. The mic is a single existing WAV, so there is nothing
/// to build.
///
/// A path rather than the bytes themselves: WKWebView loads media over range
/// requests, which the asset protocol serves and a blob URL does not — handing the
/// frontend audio inline leaves `<audio>` unable to play or seek it.
#[tauri::command]
pub async fn prepare_audio(state: State<'_, AppState>, id: String) -> Result<String, String> {
    let dir = state.root.join(sanitize(&id)?);
    let recording = Recording::from_dir(&dir).map_err(|e| e.to_string())?;

    let track = recording
        .tracks
        .iter()
        .find(|t| t.track.as_str() == "microphone")
        // A recording with no mic track (should not happen) still gets something to
        // play rather than a dead player.
        .or_else(|| recording.tracks.first())
        .ok_or("recording has no audio")?;

    Ok(track.path.display().to_string())
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

/// Re-files an existing recording.
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
    spaces::save_meta(&dir, &spaces::RecordingMeta { space }).map_err(|e| e.to_string())
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
