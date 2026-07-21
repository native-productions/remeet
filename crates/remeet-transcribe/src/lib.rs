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
//! use remeet_transcribe::{DecodeOptions, Transcriber};
//!
//! let transcriber = Transcriber::load(Path::new("models/ggml-large-v3-turbo.bin"))?;
//! // 48 kHz stereo system audio, straight from capture.
//! # let samples: Vec<f32> = vec![];
//! let segments = transcriber.transcribe(&samples, 2, 48_000, None, &DecodeOptions::default())?;
//! for seg in &segments {
//!     println!("[{:.1}s] {}", seg.start.as_secs_f64(), seg.text);
//! }
//! # Ok::<(), remeet_transcribe::TranscribeError>(())
//! ```

mod audio;
mod denoise;
mod error;
mod isolate;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use whisper_rs::{
    FullParams, SamplingStrategy, SegmentCallbackData, WhisperContext, WhisperContextParameters,
    WhisperSegment, WhisperVadParams,
};

pub use audio::{WHISPER_SAMPLE_RATE, downmix_to_mono, prepare_for_whisper};
pub use denoise::denoise;
pub use isolate::isolate_local;
pub use error::{Result, TranscribeError};

/// How to trade transcription speed against accuracy.
#[derive(Debug, Clone)]
pub struct DecodeOptions {
    /// Beam width. A width above 1 runs beam search — slower, and markedly more
    /// accurate on accented and non-English speech. 1 (or 0) selects greedy sampling,
    /// several times faster with a real accuracy cost.
    pub beam_size: usize,
    /// A ggml Silero VAD model. When set, whisper.cpp skips silent spans before
    /// decoding — a large win on meeting tracks, which are mostly silence on each
    /// side. `None` transcribes the whole track.
    pub vad_model: Option<PathBuf>,
    /// Suppress background noise on the microphone track before transcribing — for
    /// recording in a café or other noisy place.
    pub denoise_mic: bool,
    /// Flips to `true` to abort the in-flight decode. whisper.cpp polls this between
    /// compute steps and bails when it flips, so a cancelled run stops promptly instead
    /// of grinding through the rest of the track. `None` never cancels.
    pub cancel: Option<Arc<AtomicBool>>,
}

impl Default for DecodeOptions {
    /// The accurate default: beam search, no VAD, no mic noise suppression (it can
    /// clip a quiet voice, so it is opt-in for noisy places).
    fn default() -> Self {
        Self {
            beam_size: 5,
            vad_model: None,
            denoise_mic: false,
            cancel: None,
        }
    }
}

/// A segment reported live, mid-decode, before the full result is assembled.
///
/// Lighter than [`Segment`]: no confidence, because Whisper only knows the token
/// probabilities once the segment is finalised. This is for showing progress, not for
/// the bleed-suppression decisions that run on the final [`Segment`]s.
#[derive(Debug, Clone)]
pub struct LiveSegment {
    pub start: Duration,
    pub end: Duration,
    pub text: String,
}

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
        // Flash attention is a faster, numerically-equivalent attention kernel on the
        // Metal backend — a free speedup for a large model. It disables DTW token
        // timestamps, which this crate does not use (segment timestamps come from
        // Whisper directly).
        params.flash_attn(true);

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
        options: &DecodeOptions,
    ) -> Result<Vec<Segment>> {
        self.transcribe_with(samples, channels, sample_rate, language, options, |_| {})
    }

    /// Like [`transcribe`](Self::transcribe), but calls `on_segment` for each segment
    /// as Whisper finalises it, before the whole track is done.
    ///
    /// The callback runs on the calling thread, synchronously inside the decode, so it
    /// should be cheap — forwarding to a channel or a UI event, not blocking work.
    pub fn transcribe_with<F>(
        &self,
        samples: &[f32],
        channels: u16,
        sample_rate: u32,
        language: Option<&str>,
        options: &DecodeOptions,
        on_segment: F,
    ) -> Result<Vec<Segment>>
    where
        F: FnMut(LiveSegment) + 'static,
    {
        let audio = prepare_for_whisper(samples, channels, sample_rate)?;
        self.transcribe_prepared(&audio, language, options, on_segment)
    }

    /// Transcribes audio already conditioned to 16 kHz mono f32 (see
    /// [`prepare_for_whisper`]), skipping the conditioning step. Used when the caller
    /// already holds the prepared buffer — e.g. after running detection on it.
    pub fn transcribe_prepared<F>(
        &self,
        audio: &[f32],
        language: Option<&str>,
        options: &DecodeOptions,
        mut on_segment: F,
    ) -> Result<Vec<Segment>>
    where
        F: FnMut(LiveSegment) + 'static,
    {
        // Beam search explores several hypotheses per step and picks the best overall,
        // which meaningfully lifts accuracy on accented and non-English speech; greedy
        // is several times faster for the speed-first mode. `patience = -1.0` uses
        // whisper.cpp's default.
        let strategy = if options.beam_size <= 1 {
            SamplingStrategy::Greedy { best_of: 1 }
        } else {
            SamplingStrategy::BeamSearch {
                beam_size: options.beam_size as i32,
                patience: -1.0,
            }
        };
        let mut params = FullParams::new(strategy);
        params.set_language(language);

        // Decode each 30 s window on its own instead of conditioning on the previous
        // window's text. Whisper otherwise feeds its prior output forward, so a
        // hallucination on a silent stretch — common here, since each track is silent
        // while the other side talks — snowballs into the same phrase repeating for
        // pages ("yes thank you / yes thank you / ..."). Costs a little cross-segment
        // coherence; worth it to stop the runaway loop.
        params.set_no_context(true);
        // Bias against emitting non-speech tokens, the other hallucination source on
        // the quiet stretches.
        params.set_suppress_nst(true);

        // Skip silence when a VAD model is available. On a meeting track each side is
        // quiet most of the time, so this cuts a large fraction of the decode work and
        // removes the silence that drives the hallucinations above. The model path
        // must be set before enabling VAD (whisper-rs panics otherwise).
        if let Some(vad_model) = &options.vad_model {
            params.set_vad_model_path(Some(&vad_model.to_string_lossy()));
            params.set_vad_params(WhisperVadParams::default());
            params.enable_vad(true);
        }
        // Default is min(4, cores); the encoder runs on the GPU but sampling and the
        // non-GPU ops still gain from more threads. Capped so Apple Silicon's slower
        // efficiency cores are not oversubscribed.
        params.set_n_threads(decode_threads());
        // whisper.cpp otherwise prints every segment to stdout as it decodes; this
        // crate returns them instead.
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        // Let a caller abort a long decode. whisper.cpp calls this between compute
        // steps; returning true stops the run, which surfaces here as an error the
        // caller maps to a cancellation.
        if let Some(cancel) = options.cancel.clone() {
            params.set_abort_callback_safe(move || cancel.load(Ordering::Relaxed));
        }

        // Fires once per segment as it is finalised, so a caller can show the
        // transcript filling in rather than a single blocking wait.
        params.set_segment_callback_safe(move |data: SegmentCallbackData| {
            on_segment(LiveSegment {
                start: centiseconds(data.start_timestamp),
                end: centiseconds(data.end_timestamp),
                text: data.text,
            });
        });

        let mut state = self.ctx.create_state()?;
        state.full(params, audio)?;

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

    /// Picks the most likely language of `audio` (16 kHz mono) from `candidates`.
    ///
    /// whisper.cpp's own auto-detect reads only the opening seconds and locks the whole
    /// track, so a meeting that opens with silence or English small talk gets guessed
    /// wrong. This instead decodes the passed window under each candidate and keeps the
    /// one Whisper heard most confidently — the wrong language yields garbled,
    /// low-probability tokens. `None` if nothing decoded.
    ///
    /// Meant to run on a short, speech-dense window, not a whole track: it decodes once
    /// per candidate.
    pub fn detect_language(&self, audio: &[f32], candidates: &[&str]) -> Option<String> {
        let mut best: Option<(String, f32)> = None;
        for &language in candidates {
            let Ok(confidence) = self.probe_confidence(audio, language) else {
                continue;
            };
            if best.as_ref().is_none_or(|(_, top)| confidence > *top) {
                best = Some((language.to_owned(), confidence));
            }
        }
        best.map(|(language, _)| language)
    }

    /// Mean token probability from a fast greedy decode of `audio` forced to `language`
    /// — how clearly Whisper heard the audio as that language.
    fn probe_confidence(&self, audio: &[f32], language: &str) -> Result<f32> {
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some(language));
        params.set_n_threads(decode_threads());
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);

        let mut state = self.ctx.create_state()?;
        state.full(params, audio)?;

        let count = state.full_n_segments();
        let mut sum = 0.0;
        let mut tokens = 0usize;
        for i in 0..count {
            let Some(segment) = state.get_segment(i) else {
                continue;
            };
            let n = segment.n_tokens();
            for t in 0..n {
                if let Some(token) = segment.get_token(t) {
                    sum += token.token_probability();
                    tokens += 1;
                }
            }
        }
        Ok(if tokens == 0 { 0.0 } else { sum / tokens as f32 })
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

/// Thread count for the non-GPU decode work. The available parallelism on Apple
/// Silicon counts performance and efficiency cores together; capping at 8 keeps the
/// work on the fast cores without paying scheduling overhead on the slow ones.
fn decode_threads() -> std::os::raw::c_int {
    std::thread::available_parallelism()
        .map(|n| n.get().min(8))
        .unwrap_or(4) as std::os::raw::c_int
}
