//! Spike: prove the capture -> transcribe orchestration end to end.
//!
//! Records a meeting to a fresh directory (one WAV per track), then transcribes the
//! recording into an attributed transcript and saves it alongside the audio. This is
//! the core "record a meeting, get a transcript" flow — todo extraction is left out
//! on purpose; that is the caller's to wire up later.
//!
//! ```sh
//! # record until Enter, then transcribe:
//! cargo run --release -p session -- models/ggml-large-v3-turbo.bin
//!
//! # or record a fixed number of seconds (handy for scripted runs):
//! cargo run --release -p session -- models/ggml-large-v3-turbo.bin 30
//!
//! # or transcribe a recording captured earlier, skipping capture entirely:
//! cargo run --release -p session -- models/ggml-large-v3-turbo.bin --dir recordings
//! ```
//!
//! Throwaway — the reusable half lives in `remeet-session`.

use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use remeet_session::{Recorder, Recording, transcribe_recording};
use remeet_transcribe::Transcriber;

const RECORDINGS_ROOT: &str = "recordings";
const TRANSCRIPT_FILE: &str = "transcript.txt";

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let model_path = PathBuf::from(
        args.next()
            .context("usage: session <model-path> [record-seconds | --dir <recording>]")?,
    );

    // Either capture a new recording, or reopen one captured earlier.
    let recording = match args.next().as_deref() {
        Some("--dir") => {
            let dir = args.next().context("--dir needs a path")?;
            println!("Loading recording from {dir} ...");
            Recording::from_dir(&dir).with_context(|| format!("loading recording at {dir}"))?
        }
        other => {
            let record_secs: Option<u64> = other.and_then(|s| s.parse().ok());
            capture(record_secs).await?
        }
    };
    report_recording(&recording);

    // Transcribe. Separate step: everything above could run without a model present.
    println!("\nLoading model: {} ...", model_path.display());
    let load_start = Instant::now();
    let transcriber = Transcriber::load(&model_path)
        .with_context(|| format!("loading model at {}", model_path.display()))?;
    println!(
        "Loaded in {:.1}s. Transcribing...",
        load_start.elapsed().as_secs_f64()
    );

    let transcribe_start = Instant::now();
    let transcript =
        transcribe_recording(&transcriber, &recording, None).context("transcribing recording")?;
    println!(
        "Transcribed in {:.1}s.",
        transcribe_start.elapsed().as_secs_f64()
    );

    if transcript.is_empty() {
        anyhow::bail!("no speech transcribed from the recording");
    }

    let transcript_path = recording.dir.join(TRANSCRIPT_FILE);
    std::fs::write(&transcript_path, transcript.to_string())
        .with_context(|| format!("writing {}", transcript_path.display()))?;

    println!("\n=== Transcript ===");
    print!("{transcript}");
    println!("\nSaved to {}", transcript_path.display());

    Ok(())
}

/// Captures a new recording, either for a fixed duration or until Enter.
async fn capture(record_secs: Option<u64>) -> Result<Recording> {
    let dir = PathBuf::from(RECORDINGS_ROOT).join(format!("session-{}", unix_secs()));

    println!("Recording to {} ...", dir.display());
    let recorder = Recorder::start(&dir).await.context("starting recorder")?;

    match record_secs {
        Some(secs) => {
            println!("Recording for {secs}s.");
            tokio::time::sleep(Duration::from_secs(secs)).await;
        }
        None => {
            println!("Press Enter to stop.");
            wait_for_enter().await?;
        }
    }

    recorder.stop().await.context("stopping recorder")
}

/// Prints what each track captured.
fn report_recording(recording: &Recording) {
    println!("\nCaptured:");
    for track in &recording.tracks {
        println!(
            "  {:<11} {:>5.1}s  {}",
            track.track.as_str(),
            track.duration.as_secs_f64(),
            track.path.display()
        );
    }
}

/// Blocks on a single line from stdin without stalling the async runtime.
async fn wait_for_enter() -> Result<()> {
    tokio::task::spawn_blocking(|| {
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)
    })
    .await
    .context("stdin task panicked")?
    .context("reading stdin")?;
    Ok(())
}

/// Seconds since the Unix epoch, for a unique-enough recording directory name.
fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
