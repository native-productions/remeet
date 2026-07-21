//! Session orchestration for Remeet.
//!
//! Ties capture ([`remeet_audio`]) and transcription ([`remeet_transcribe`]) into the
//! core flow: record a meeting to disk, then turn the recording into an attributed
//! transcript.
//!
//! The two steps are intentionally decoupled through the WAV files on disk. Recording
//! is live and needs the microphone; transcription is offline and can run at any time
//! afterward — or be skipped, or driven into some other workflow. Nothing here forces
//! them into a single pipeline.
//!
//! ```no_run
//! use std::path::Path;
//! use remeet_session::{Recorder, transcribe_recording};
//! use remeet_transcribe::Transcriber;
//!
//! # async fn run() -> Result<(), remeet_session::SessionError> {
//! // Record.
//! let recorder = Recorder::start("recordings/2026-07-16").await?;
//! // ... meeting happens ...
//! let recording = recorder.stop().await?;
//!
//! // Transcribe, whenever.
//! let transcriber = Transcriber::load(Path::new("models/ggml-large-v3-turbo.bin"))?;
//! let transcript = transcribe_recording(&transcriber, &recording, None, &Default::default())?;
//! print!("{transcript}");
//! # Ok(())
//! # }
//! ```

mod aec_pass;
mod error;
mod mixdown;
mod recorder;
mod transcript;

use std::path::{Path, PathBuf};
use std::time::Duration;

use remeet_audio::Track;

pub use aec_pass::apply as apply_echo_cancellation;
pub use error::{Result, SessionError};
pub use mixdown::{MIXDOWN_WAV, mixdown};
pub use recorder::Recorder;
pub use remeet_transcribe::LiveSegment;
pub use transcript::{
    Speaker, Transcript, TranscriptLine, transcribe_recording, transcribe_recording_streaming,
};

/// Every track a recording can hold, in a stable order.
const KNOWN_TRACKS: [Track; 2] = [Track::System, Track::Microphone];

/// A finished recording: one WAV file per captured track.
#[derive(Debug, Clone)]
pub struct Recording {
    /// Directory holding the track files.
    pub dir: PathBuf,
    /// One entry per track that produced audio.
    pub tracks: Vec<TrackRecording>,
}

impl Recording {
    /// Loads a recording previously written to `dir` by a [`Recorder`].
    ///
    /// This is the other half of the record/transcribe split: a recording saved now
    /// can be reopened and transcribed later, in a separate process, from the WAV
    /// files alone. Only the known track files are picked up; anything else in the
    /// directory (a saved transcript, say) is ignored.
    pub fn from_dir(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref().to_path_buf();

        let mut tracks = Vec::new();
        for track in KNOWN_TRACKS {
            let path = dir.join(format!("{}.wav", track.as_str()));
            if path.exists() {
                let duration = wav_duration(&path)?;
                tracks.push(TrackRecording {
                    track,
                    path,
                    duration,
                });
            }
        }

        if tracks.is_empty() {
            return Err(SessionError::NothingCaptured);
        }

        Ok(Self { dir, tracks })
    }
}

/// Reads a WAV's playing time from its header, without decoding samples.
fn wav_duration(path: &Path) -> Result<Duration> {
    let reader = hound::WavReader::open(path).map_err(|source| SessionError::WavRead {
        path: path.display().to_string(),
        source,
    })?;
    let spec = reader.spec();
    let seconds = reader.duration() as f64 / spec.sample_rate as f64;
    Ok(Duration::from_secs_f64(seconds))
}

/// One track's WAV file and how long it ran.
#[derive(Debug, Clone)]
pub struct TrackRecording {
    pub track: Track,
    pub path: PathBuf,
    pub duration: Duration,
}
