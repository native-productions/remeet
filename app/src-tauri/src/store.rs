//! On-disk recording store and the DTOs the frontend sees.
//!
//! Recordings live one-directory-per-session under a root in the user's home. Each
//! directory holds the track WAVs plus, once transcribed, `transcript.json` (the
//! structured form the UI renders) and `transcript.txt` (a plain-text copy).

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use remeet_ai::Summary;
use remeet_session::{Recording, Transcript};
use serde::{Deserialize, Serialize};

const TRANSCRIPT_JSON: &str = "transcript.json";
pub const TRANSCRIPT_TXT: &str = "transcript.txt";
const SUMMARY_JSON: &str = "summary.json";

/// A recording as the frontend needs it: an id, how long it ran, and whether it has
/// been transcribed yet.
#[derive(Debug, Clone, Serialize)]
pub struct RecordingDto {
    /// Directory name, used as the stable id in later commands.
    pub id: String,
    /// Longest track's duration, in whole seconds.
    pub duration_secs: u64,
    /// Seconds since the Unix epoch when the directory was created, for sorting.
    pub created: u64,
    pub transcribed: bool,
    pub summarized: bool,
    /// Space id this recording is filed under, or `None` for the default space.
    pub space: Option<String>,
    /// A user-given label, or `None` to fall back to the recorded-at timestamp.
    pub name: Option<String>,
}

/// A transcribed line, flattened for the frontend. Round-trips through
/// `transcript.json`, so it is both serialized and deserialized.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineDto {
    /// "me" or "them".
    pub speaker: String,
    pub start_secs: f64,
    pub text: String,
}

impl RecordingDto {
    fn from_dir(dir: &Path) -> Option<Self> {
        let id = dir.file_name()?.to_str()?.to_owned();
        let recording = Recording::from_dir(dir).ok()?;
        let duration_secs = recording
            .tracks
            .iter()
            .map(|t| t.duration.as_secs())
            .max()
            .unwrap_or(0);

        let meta = crate::spaces::load_meta(dir);
        Some(Self {
            id,
            duration_secs,
            created: dir_created(dir),
            transcribed: dir.join(TRANSCRIPT_JSON).exists(),
            summarized: dir.join(SUMMARY_JSON).exists(),
            space: meta.space,
            name: meta.name,
        })
    }
}

/// Lists recordings under `root`, newest first. A missing root is simply empty.
pub fn list(root: &Path) -> Vec<RecordingDto> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };

    let mut recordings: Vec<RecordingDto> = entries
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| RecordingDto::from_dir(&e.path()))
        .collect();

    recordings.sort_by_key(|r| std::cmp::Reverse(r.created));
    recordings
}

/// Serializes a transcript to the two on-disk forms next to the audio.
pub fn save_transcript(dir: &Path, transcript: &Transcript) -> std::io::Result<()> {
    let lines = to_dtos(transcript);
    let json = serde_json::to_string_pretty(&lines).unwrap_or_else(|_| "[]".to_owned());
    std::fs::write(dir.join(TRANSCRIPT_JSON), json)?;
    std::fs::write(dir.join(TRANSCRIPT_TXT), transcript.to_string())?;
    Ok(())
}

/// Loads a previously saved transcript, if one exists.
pub fn load_transcript(dir: &Path) -> Option<Vec<LineDto>> {
    let json = std::fs::read_to_string(dir.join(TRANSCRIPT_JSON)).ok()?;
    serde_json::from_str(&json).ok()
}

/// Flattens a [`Transcript`] into frontend line DTOs.
pub fn to_dtos(transcript: &Transcript) -> Vec<LineDto> {
    transcript
        .lines
        .iter()
        .map(|line| LineDto {
            speaker: match line.speaker {
                remeet_session::Speaker::Me => "me".to_owned(),
                remeet_session::Speaker::Them => "them".to_owned(),
            },
            start_secs: line.start.as_secs_f64(),
            text: line.text.trim().to_owned(),
        })
        .collect()
}

/// Saves a summary next to the audio it describes.
///
/// Cached on disk because each summary costs a full CLI invocation — re-opening a
/// recording must not silently spend tokens again.
pub fn save_summary(dir: &Path, summary: &Summary) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(summary).unwrap_or_else(|_| "{}".to_owned());
    std::fs::write(dir.join(SUMMARY_JSON), json)
}

pub fn load_summary(dir: &Path) -> Option<Summary> {
    let json = std::fs::read_to_string(dir.join(SUMMARY_JSON)).ok()?;
    serde_json::from_str(&json).ok()
}

/// Renders a transcript the way the AI prompts expect: `[speaker] text`, one line
/// each, which is also the format `remeet-todo` reads.
pub fn to_prompt_text(lines: &[LineDto]) -> String {
    lines
        .iter()
        .map(|line| format!("[{}] {}", line.speaker, line.text))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A fresh session directory name, `session-<unix-seconds>`.
pub fn new_session_dir(root: &Path) -> PathBuf {
    root.join(format!("session-{}", now_secs()))
}

fn dir_created(dir: &Path) -> u64 {
    dir.metadata()
        .and_then(|m| m.created())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
