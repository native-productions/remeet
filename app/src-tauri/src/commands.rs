//! Tauri commands: the bridge between the popover UI and `remeet-session`.
//!
//! Recording is stateful (one session at a time), held behind an async mutex so a
//! command can hold the lock across the `await` on capture start/stop. Transcription
//! is CPU/GPU heavy and synchronous, so it runs on a blocking thread.

use std::path::PathBuf;
use std::time::Instant;

use remeet_session::{Recorder, Recording, mixdown, transcribe_recording};
use remeet_transcribe::Transcriber;
use serde::Serialize;
use tauri::State;
use tokio::sync::Mutex;

use crate::store::{self, LineDto, RecordingDto};

/// Shared application state, managed by Tauri.
pub struct AppState {
    session: Mutex<Option<Active>>,
    root: PathBuf,
    model_path: PathBuf,
    /// Transcription language as an ISO code, or `None` to auto-detect.
    language: Option<String>,
}

/// The in-progress recording, if any.
struct Active {
    recorder: Recorder,
    started: Instant,
}

impl AppState {
    /// Builds state with the default store (`~/Remeet/recordings`) and the full
    /// `large-v3` model, which transcribes non-English speech far more accurately
    /// than the distilled `turbo` variant.
    ///
    /// Both are overridable: `REMEET_MODEL` points at a different GGML model, and
    /// `REMEET_LANG` forces a language (e.g. `id`) instead of auto-detection.
    pub fn new() -> Self {
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

        Self {
            session: Mutex::new(None),
            root: home.join("Remeet").join("recordings"),
            model_path,
            language,
        }
    }
}

/// Whether a recording is in progress, and for how long.
#[derive(Serialize)]
pub struct Status {
    recording: bool,
    elapsed_secs: u64,
}

#[tauri::command]
pub async fn get_status(state: State<'_, AppState>) -> Result<Status, String> {
    let session = state.session.lock().await;
    Ok(match session.as_ref() {
        Some(active) => Status {
            recording: true,
            elapsed_secs: active.started.elapsed().as_secs(),
        },
        None => Status {
            recording: false,
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

    *session = Some(Active {
        recorder,
        started: Instant::now(),
    });
    Ok(())
}

#[tauri::command]
pub async fn stop_recording(state: State<'_, AppState>) -> Result<RecordingDto, String> {
    let mut session = state.session.lock().await;
    let active = session.take().ok_or("not recording")?;

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

#[tauri::command]
pub async fn transcribe(state: State<'_, AppState>, id: String) -> Result<Vec<LineDto>, String> {
    let dir = state.root.join(sanitize(&id)?);
    let model_path = state.model_path.clone();
    let language = state.language.clone();

    if !model_path.exists() {
        return Err(format!("model not found at {}", model_path.display()));
    }

    // Model load and inference are synchronous and heavy; keep them off the async
    // runtime's worker threads.
    tokio::task::spawn_blocking(move || {
        let recording = Recording::from_dir(&dir).map_err(|e| e.to_string())?;
        let transcriber = Transcriber::load(&model_path).map_err(|e| e.to_string())?;
        let transcript = transcribe_recording(&transcriber, &recording, language.as_deref())
            .map_err(|e| e.to_string())?;

        store::save_transcript(&dir, &transcript).map_err(|e| e.to_string())?;
        Ok(store::to_dtos(&transcript))
    })
    .await
    .map_err(|_| "transcription task panicked".to_string())?
}

/// Mixes the recording's tracks into one playable file and returns its path.
///
/// A path rather than the bytes themselves: WKWebView loads media over range
/// requests, which the asset protocol serves and a blob URL does not — handing the
/// frontend audio inline leaves `<audio>` unable to play or seek it.
#[tauri::command]
pub async fn prepare_audio(state: State<'_, AppState>, id: String) -> Result<String, String> {
    let dir = state.root.join(sanitize(&id)?);

    // Decoding and resampling both tracks blocks; keep it off the async workers.
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
