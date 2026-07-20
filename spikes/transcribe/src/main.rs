//! Spike: prove that the two captured tracks transcribe into a readable,
//! attributed timeline.
//!
//! Reads `recordings/system.wav` and `recordings/microphone.wav` (produced by the
//! dual-capture spike), transcribes each, and interleaves the segments by timestamp
//! into one "me vs. them" transcript. Success is judged by reading it: the words
//! should match what was said, and each line should be attributed to the right side.
//!
//! Throwaway — the reusable half lives in `remeet-transcribe`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use remeet_transcribe::{Segment, Transcriber};

const RECORDINGS_DIR: &str = "recordings";
const DEFAULT_MODEL: &str = "models/ggml-large-v3-turbo.bin";

/// A speaker label plus a transcribed segment, ready to sort onto one timeline.
struct Line {
    speaker: &'static str,
    segment: Segment,
}

fn main() -> Result<()> {
    // Model path is the sole argument so it can point at ~/whisper/models without
    // copying the 1.5 GB file into the repo.
    let model_path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_MODEL));

    println!("Loading model: {}", model_path.display());
    let load_start = Instant::now();
    let transcriber = Transcriber::load(&model_path)
        .with_context(|| format!("loading model at {}", model_path.display()))?;
    println!("Loaded in {:.1}s\n", load_start.elapsed().as_secs_f64());

    let dir = Path::new(RECORDINGS_DIR);
    let mut lines = Vec::new();
    // "them" first so that when a system line and a mic line share a timestamp, the
    // remote prompt sorts ahead of the local reply — which reads in the right order.
    lines.extend(transcribe_track(
        &transcriber,
        &dir.join("system.wav"),
        "them",
    )?);
    lines.extend(transcribe_track(
        &transcriber,
        &dir.join("microphone.wav"),
        "me",
    )?);

    lines.sort_by_key(|line| line.segment.start);

    println!("\n=== Transcript ===");
    if lines.is_empty() {
        anyhow::bail!("no speech transcribed from either track");
    }
    for line in &lines {
        println!(
            "[{:>7} - {:>7}] {:<4} {}",
            fmt(line.segment.start),
            fmt(line.segment.end),
            format!("{}:", line.speaker),
            line.segment.text.trim()
        );
    }

    Ok(())
}

/// Transcribes one WAV file and tags every segment with a speaker label.
fn transcribe_track(
    transcriber: &Transcriber,
    path: &Path,
    speaker: &'static str,
) -> Result<Vec<Line>> {
    let (samples, channels, sample_rate) =
        read_wav(path).with_context(|| format!("reading {}", path.display()))?;

    println!(
        "Transcribing {} ({:.1}s, {} Hz, {}ch)...",
        path.display(),
        samples.len() as f64 / (sample_rate as f64 * channels as f64),
        sample_rate,
        channels
    );

    let start = Instant::now();
    let segments = transcriber
        .transcribe(&samples, channels, sample_rate, None, &Default::default())
        .with_context(|| format!("transcribing {}", path.display()))?;
    println!(
        "  {} segments in {:.1}s",
        segments.len(),
        start.elapsed().as_secs_f64()
    );

    Ok(segments
        .into_iter()
        .map(|segment| Line { speaker, segment })
        .collect())
}

/// Reads a 16-bit PCM WAV into interleaved f32 in [-1, 1].
fn read_wav(path: &Path) -> Result<(Vec<f32>, u16, u32)> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();

    let samples = reader
        .samples::<i16>()
        .map(|s| s.map(|v| v as f32 / i16::MAX as f32))
        .collect::<std::result::Result<Vec<f32>, _>>()?;

    Ok((samples, spec.channels, spec.sample_rate))
}

/// Formats a timestamp as `m:ss.d`, the resolution Whisper actually reports.
fn fmt(d: Duration) -> String {
    let secs = d.as_secs_f64();
    format!("{}:{:04.1}", secs as u64 / 60, secs % 60.0)
}
