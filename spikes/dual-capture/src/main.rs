//! Spike: prove that a meeting's two sides can be captured as separate tracks.
//!
//! Records for a fixed window and writes `recordings/system.wav` (the remote
//! participants) and `recordings/microphone.wav` (the local user). Success is judged
//! by ear, not by exit code: play both back and confirm each holds the voice it
//! should, and none of the other.
//!
//! Throwaway by design — the reusable half lives in `remeet-audio`.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossbeam_channel::RecvTimeoutError;
use remeet_audio::{AudioFrame, DualCapture, Track, WavSink};

const RECORD_FOR: Duration = Duration::from_secs(30);
const OUTPUT_DIR: &str = "recordings";

/// How long to keep draining after the stream stops, to collect buffers already in
/// flight on the dispatch queue.
const DRAIN_GRACE: Duration = Duration::from_millis(500);

#[tokio::main]
async fn main() -> Result<()> {
    let output_dir = PathBuf::from(OUTPUT_DIR);
    fs::create_dir_all(&output_dir)
        .with_context(|| format!("creating {}", output_dir.display()))?;

    let (tx, rx) = crossbeam_channel::unbounded::<AudioFrame>();

    println!("Requesting capture permissions (Screen Recording + Microphone)...");
    let capture = DualCapture::start(tx).await.context("starting capture")?;

    println!(
        "Recording for {}s. Play some audio and talk into the mic.",
        RECORD_FOR.as_secs()
    );

    // The receive loop blocks, so it runs off the async runtime's worker threads.
    let collector = tokio::task::spawn_blocking(move || collect(rx, &output_dir));

    let started = Instant::now();
    tokio::time::sleep(RECORD_FOR).await;
    capture.stop().await.context("stopping capture")?;
    let elapsed = started.elapsed();

    let (sinks, output_dir) = collector.await.context("collector panicked")??;
    report(sinks, &output_dir, elapsed)
}

/// Drains frames until the stream stops and the grace period lapses.
fn collect(
    rx: crossbeam_channel::Receiver<AudioFrame>,
    output_dir: &Path,
) -> Result<(HashMap<Track, WavSink>, PathBuf)> {
    let mut sinks: HashMap<Track, WavSink> = HashMap::new();

    loop {
        match rx.recv_timeout(DRAIN_GRACE) {
            Ok(frame) => route(&mut sinks, output_dir, frame)?,
            // Silence for the whole grace period means capture has stopped. Both
            // tracks emit continuously while running, even over silence.
            Err(RecvTimeoutError::Timeout) => break,
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }

    Ok((sinks, output_dir.to_path_buf()))
}

/// Writes a frame to its track's file, opening the file on first sight of the track.
///
/// The WAV header needs a sample rate and channel count up front, and those are only
/// known once a real frame arrives — so files are created lazily rather than guessed.
fn route(sinks: &mut HashMap<Track, WavSink>, dir: &Path, frame: AudioFrame) -> Result<()> {
    let sink = match sinks.entry(frame.track) {
        Entry::Occupied(entry) => entry.into_mut(),
        Entry::Vacant(entry) => {
            let path = dir.join(format!("{}.wav", frame.track.as_str()));
            println!(
                "  {} -> {} ({} Hz, {}ch)",
                frame.track.as_str(),
                path.display(),
                frame.sample_rate,
                frame.channels
            );
            let sink = WavSink::create(&path, frame.sample_rate, frame.channels)
                .with_context(|| format!("creating {}", path.display()))?;
            entry.insert(sink)
        }
    };

    anyhow::ensure!(
        sink.accepts(&frame),
        "{} changed format mid-stream ({} Hz, {}ch)",
        frame.track.as_str(),
        frame.sample_rate,
        frame.channels
    );

    sink.write(&frame).context("writing samples")
}

/// Tolerance on the audio-duration vs. wall-clock check. Generous, because it is
/// meant to catch a misread format (which is off by a whole ratio), not jitter.
const DRIFT_TOLERANCE: f64 = 0.15;

/// Prints what landed, and fails on either failure this spike exists to catch.
///
/// Neither is visible from the exit code alone: an empty WAV is still a valid WAV,
/// and a WAV whose header disagrees with its samples still plays — just at the
/// wrong speed. So a track must produce audio, and the audio's duration must match
/// how long we actually recorded. A mismatch means the sample rate or channel count
/// was misread, which would silently pitch-shift everything downstream.
fn report(mut sinks: HashMap<Track, WavSink>, dir: &Path, elapsed: Duration) -> Result<()> {
    println!(
        "\nResult (recorded {:.1}s wall clock):",
        elapsed.as_secs_f64()
    );
    let mut problems = Vec::new();

    for track in [Track::System, Track::Microphone] {
        let Some(sink) = sinks.remove(&track) else {
            println!("  {:<11} NO AUDIO", track.as_str());
            problems.push(format!("{}: no audio", track.as_str()));
            continue;
        };

        let seconds = sink.seconds_written();
        let drift = (seconds - elapsed.as_secs_f64()).abs() / elapsed.as_secs_f64();

        println!(
            "  {:<11} {:>6.1}s  ({} frames, {:.0}% off wall clock)",
            track.as_str(),
            seconds,
            sink.frames_written(),
            drift * 100.0
        );

        if drift > DRIFT_TOLERANCE {
            problems.push(format!(
                "{}: {:.1}s of audio over {:.1}s of recording — format misread",
                track.as_str(),
                seconds,
                elapsed.as_secs_f64()
            ));
        }

        sink.finalize()
            .with_context(|| format!("finalizing {} wav", track.as_str()))?;
    }

    if !problems.is_empty() {
        anyhow::bail!("{}", problems.join("; "));
    }

    println!("\nVerify by ear:");
    println!("  afplay {}/system.wav", dir.display());
    println!("  afplay {}/microphone.wav", dir.display());

    Ok(())
}
