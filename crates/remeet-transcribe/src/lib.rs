//! Whisper transcription for Remeet.
//!
//! Turns captured audio — any channel count, any sample rate — into timestamped
//! text segments. Loads a GGML Whisper model once into a [`Transcriber`] and runs
//! it on the M-series GPU via whisper.cpp's Metal backend.
//!
//! The audio conditioning (downmix to mono, resample to 16 kHz) lives in [`audio`]
//! and runs before every transcription, so callers hand over frames in whatever
//! shape the capture layer produced them.
//!
//! ```no_run
//! use std::path::Path;
//! use remeet_transcribe::Transcriber;
//!
//! let transcriber = Transcriber::load(Path::new("models/ggml-large-v3-turbo.bin"))?;
//! // 48 kHz stereo system audio, straight from capture.
//! # let samples: Vec<f32> = vec![];
//! let segments = transcriber.transcribe(&samples, 2, 48_000, None)?;
//! for seg in &segments {
//!     println!("[{:.1}s] {}", seg.start.as_secs_f64(), seg.text);
//! }
//! # Ok::<(), remeet_transcribe::TranscribeError>(())
//! ```

mod audio;
mod error;

use std::path::Path;
use std::time::Duration;

use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters, WhisperSegment,
};

pub use audio::{WHISPER_SAMPLE_RATE, downmix_to_mono, prepare_for_whisper};
pub use error::{Result, TranscribeError};

/// One transcribed span of speech.
#[derive(Debug, Clone)]
pub struct Segment {
    /// Offset from the start of the audio to the start of this span.
    pub start: Duration,
    /// Offset from the start of the audio to the end of this span.
    pub end: Duration,
    pub text: String,
    /// Mean token probability over the segment, in `[0, 1]`. A proxy for how clearly
    /// the model heard this span: clean speech scores high, degraded audio (an
    /// acoustic echo bleeding across tracks, say) scores lower.
    pub confidence: f32,
}

/// A loaded Whisper model, ready to transcribe.
///
/// Loading is the expensive step (the model is ~1.5 GB), so a `Transcriber` is meant
/// to be built once and reused. Each [`transcribe`](Self::transcribe) call runs on a
/// fresh decoding state, so calls do not leak context into one another.
pub struct Transcriber {
    ctx: WhisperContext,
}

impl Transcriber {
    /// Loads a GGML model from disk with GPU (Metal) acceleration enabled.
    pub fn load(model_path: &Path) -> Result<Self> {
        let mut params = WhisperContextParameters::default();
        params.use_gpu(true);

        let path = model_path.to_string_lossy().into_owned();
        let ctx = WhisperContext::new_with_params(&path, params)
            .map_err(|source| TranscribeError::ModelLoad { path, source })?;

        Ok(Self { ctx })
    }

    /// Transcribes one buffer of audio.
    ///
    /// `samples` are interleaved f32 at `sample_rate` with `channels` channels — the
    /// shape the capture layer produces. Conditioning to Whisper's 16 kHz mono is
    /// handled internally.
    ///
    /// `language` is an ISO code (`"en"`, `"id"`); `None` lets Whisper detect it,
    /// which is the right default for a meeting that may switch languages.
    pub fn transcribe(
        &self,
        samples: &[f32],
        channels: u16,
        sample_rate: u32,
        language: Option<&str>,
    ) -> Result<Vec<Segment>> {
        let audio = prepare_for_whisper(samples, channels, sample_rate)?;

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(language);
        // whisper.cpp otherwise prints every segment to stdout as it decodes; this
        // crate returns them instead.
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        let mut state = self.ctx.create_state()?;
        state.full(params, &audio)?;

        let count = state.full_n_segments();
        let mut segments = Vec::with_capacity(count.max(0) as usize);
        for i in 0..count {
            let Some(segment) = state.get_segment(i) else {
                continue;
            };
            segments.push(Segment {
                start: centiseconds(segment.start_timestamp()),
                end: centiseconds(segment.end_timestamp()),
                text: segment.to_str_lossy()?.into_owned(),
                confidence: mean_token_confidence(&segment),
            });
        }

        Ok(segments)
    }
}

/// Averages a segment's per-token probabilities. Empty segments score 0.
fn mean_token_confidence(segment: &WhisperSegment<'_>) -> f32 {
    let count = segment.n_tokens();
    if count <= 0 {
        return 0.0;
    }

    let sum: f32 = (0..count)
        .filter_map(|t| segment.get_token(t))
        .map(|token| token.token_probability())
        .sum();

    sum / count as f32
}

/// Whisper reports timestamps in centiseconds (hundredths of a second).
fn centiseconds(cs: i64) -> Duration {
    Duration::from_millis(cs.max(0) as u64 * 10)
}
