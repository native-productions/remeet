//! Mixes a recording's separate track files into one playable WAV.
//!
//! The tracks are kept apart on disk because transcription needs them apart — that is
//! what attributes a line to a speaker. Playback wants the opposite: one file holding
//! both sides of the conversation on a single timeline.
//!
//! The mix is mono 16 kHz, the same shape transcription already normalizes to. That
//! is telephone quality, which is all a spoken meeting needs, and it reuses the
//! band-limited resampler instead of introducing a second audio path.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use hound::{SampleFormat, WavSpec, WavWriter};
use remeet_transcribe::{WHISPER_SAMPLE_RATE, prepare_for_whisper};

use crate::Recording;
use crate::error::Result;
use crate::transcript::read_wav;

/// File name of the cached mixdown inside a recording's directory.
pub const MIXDOWN_WAV: &str = "mixdown.wav";

/// Mixes the recording's tracks into a single 16-bit mono WAV and returns its path.
///
/// The file is written as [`MIXDOWN_WAV`] next to the tracks and reused on later
/// calls, so replaying a recording does not re-decode and re-resample the audio.
pub fn mixdown(recording: &Recording) -> Result<PathBuf> {
    let path = recording.dir.join(MIXDOWN_WAV);
    if path.exists() {
        return Ok(path);
    }

    std::fs::write(&path, encode(&mix(recording)?)?)?;
    Ok(path)
}

/// Sums every track to one mono 16 kHz signal.
///
/// Tracks are summed at half gain rather than averaged over however many tracks
/// exist, so a one-sided stretch — the common case, only one person talking — does
/// not drop in level when the other track is silent. Clipping is handled at encode
/// time, where samples are clamped to the rail.
fn mix(recording: &Recording) -> Result<Vec<f32>> {
    let mut mixed: Vec<f32> = Vec::new();

    for track in &recording.tracks {
        let samples = mono_16k(&track.path)?;
        if samples.len() > mixed.len() {
            mixed.resize(samples.len(), 0.0);
        }
        for (out, sample) in mixed.iter_mut().zip(samples) {
            *out += sample * 0.5;
        }
    }

    Ok(mixed)
}

fn mono_16k(path: &Path) -> Result<Vec<f32>> {
    let (samples, channels, sample_rate) = read_wav(path)?;
    Ok(prepare_for_whisper(&samples, channels, sample_rate)?)
}

/// Writes samples to an in-memory 16-bit PCM WAV.
fn encode(samples: &[f32]) -> Result<Vec<u8>> {
    let spec = WavSpec {
        channels: 1,
        sample_rate: WHISPER_SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };

    let mut buffer = Cursor::new(Vec::new());
    let mut writer = WavWriter::new(&mut buffer, spec)?;
    for &sample in samples {
        writer.write_sample((sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)?;
    }
    writer.finalize()?;

    Ok(buffer.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_writes_a_readable_wav() {
        let bytes = encode(&[0.0, 0.5, -0.5]).expect("encode");
        let reader = hound::WavReader::new(Cursor::new(bytes)).expect("read back");
        assert_eq!(reader.spec().sample_rate, WHISPER_SAMPLE_RATE);
        assert_eq!(reader.spec().channels, 1);
        assert_eq!(reader.duration(), 3);
    }
}
