//! Post-record echo-cancellation pass.
//!
//! Runs once when a recording stops, after both tracks are on disk and aligned. It
//! reads `microphone.wav` (raw, with the remote's speaker bleed) and `system.wav` (the
//! reference), cancels the bleed with [`remeet_aec`], and replaces `microphone.wav`
//! with the cleaned track — keeping the original as `microphone_raw.wav` so nothing is
//! lost. Everything downstream (transcript, mixdown) then reads a mic that already has
//! only the local voice, so no per-track gating is needed.

use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::path::Path;
use std::thread::sleep;
use std::time::Duration;

use hound::{SampleFormat, WavReader, WavSpec, WavWriter};

use crate::error::{Result, SessionError};

const MIC: &str = "microphone.wav";
const SYSTEM: &str = "system.wav";
const MIC_RAW: &str = "microphone_raw.wav";
const TMP: &str = "microphone.clean.tmp";
const LOCK: &str = ".aec.lock";

/// Cancels the mic's speaker bleed in `dir`, in place. A no-op when there is nothing to
/// cancel against (a one-sided capture) or when it has already run.
///
/// Safe to call concurrently — from the post-stop background pass, from playback, and
/// from transcription. Exactly one caller does the work; the others wait for it and
/// return once the cleaned mic is in place, so none of them ever reads or renames the
/// mic file out from under another.
pub fn apply(dir: &Path) -> Result<()> {
    let mic_path = dir.join(MIC);
    let sys_path = dir.join(SYSTEM);
    let raw_path = dir.join(MIC_RAW);

    // Fast path: already done, or nothing to reference against.
    if raw_path.exists() || !mic_path.exists() || !sys_path.exists() {
        return Ok(());
    }

    if !acquire_lock(&dir.join(LOCK), &raw_path)? {
        // Another caller finished the cancellation while we waited.
        return Ok(());
    }

    // Holding the lock now. Do the work, then always release the lock.
    let result = cancel_in_place(dir, &mic_path, &sys_path, &raw_path);
    let _ = std::fs::remove_file(dir.join(LOCK));
    result
}

/// Acquires the per-recording AEC lock. Returns `Ok(true)` when this caller holds it and
/// must do the work, `Ok(false)` when another caller finished it while we waited.
fn acquire_lock(lock_path: &Path, raw_path: &Path) -> Result<bool> {
    loop {
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(lock_path)
        {
            Ok(_) => return Ok(true),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {
                // Someone else is cancelling. Wait for the raw file (their success
                // marker); bail out of the wait if the lock disappears without it,
                // which means the holder died mid-run and we should retry.
                for _ in 0..600 {
                    sleep(Duration::from_millis(100));
                    if raw_path.exists() {
                        return Ok(false);
                    }
                    if !lock_path.exists() {
                        break;
                    }
                }
                if raw_path.exists() {
                    return Ok(false);
                }
                // Stale lock from a crashed holder — clear it and try to acquire.
                let _ = std::fs::remove_file(lock_path);
            }
            Err(e) => return Err(SessionError::Io(e)),
        }
    }
}

/// The actual cancellation, run under the lock. Writes the cleaned mic to a temp file
/// first, then swaps: `microphone.wav` → `microphone_raw.wav`, temp → `microphone.wav`,
/// so a crash cannot leave the mic file missing or half-written.
fn cancel_in_place(dir: &Path, mic_path: &Path, sys_path: &Path, raw_path: &Path) -> Result<()> {
    // Re-check under the lock: a prior holder may have finished between our fast-path
    // check and acquiring the lock.
    if raw_path.exists() {
        return Ok(());
    }

    let (mic, mic_rate) = read_mono(mic_path)?;
    let (reference, ref_rate) = read_mono(sys_path)?;
    // AEC needs one common rate. The tracks are captured at the same rate today; if that
    // ever diverges, skip rather than cancel against a mismatched reference.
    if mic_rate != ref_rate {
        return Ok(());
    }

    let clean = remeet_aec::cancel(&mic, &reference, mic_rate)
        .map_err(|e| SessionError::Aec(format!("{e:?}")))?;

    let tmp_path = dir.join(TMP);
    write_mono(&tmp_path, &clean, mic_rate)?;
    std::fs::rename(mic_path, raw_path)?;
    std::fs::rename(&tmp_path, mic_path)?;
    Ok(())
}

/// Reads a 16-bit WAV as mono f32, averaging channels.
fn read_mono(path: &Path) -> Result<(Vec<f32>, u32)> {
    let mut reader = WavReader::open(path).map_err(|source| SessionError::WavRead {
        path: path.display().to_string(),
        source,
    })?;
    let spec = reader.spec();
    let channels = spec.channels.max(1) as usize;

    let samples: Vec<f32> = reader
        .samples::<i16>()
        .map(|s| s.map(|v| v as f32 / 32768.0))
        .collect::<std::result::Result<_, _>>()
        .map_err(|source| SessionError::WavRead {
            path: path.display().to_string(),
            source,
        })?;

    let mono = if channels <= 1 {
        samples
    } else {
        samples
            .chunks(channels)
            .map(|frame| frame.iter().sum::<f32>() / channels as f32)
            .collect()
    };
    Ok((mono, spec.sample_rate))
}

/// Writes mono f32 samples as a 16-bit PCM WAV, matching [`remeet_audio::WavSink`].
fn write_mono(path: &Path, samples: &[f32], sample_rate: u32) -> Result<()> {
    let spec = WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut writer = WavWriter::create(path, spec)?;
    for &s in samples {
        writer.write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)?;
    }
    writer.finalize()?;
    Ok(())
}
