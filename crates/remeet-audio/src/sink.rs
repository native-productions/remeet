use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use hound::{SampleFormat, WavSpec, WavWriter};

use crate::error::Result;
use crate::frame::AudioFrame;

/// Writes one track's frames to a 16-bit PCM WAV file.
///
/// 16-bit rather than float32 so the output plays in QuickTime and `afplay`
/// without conversion — this exists to be listened to. The transcription path
/// will consume [`AudioFrame`] directly and keep full float precision.
pub struct WavSink {
    writer: WavWriter<BufWriter<File>>,
    channels: u16,
    sample_rate: u32,
    frames_written: u64,
}

impl WavSink {
    /// Creates a WAV file whose header matches the first frame seen.
    pub fn create(path: &Path, sample_rate: u32, channels: u16) -> Result<Self> {
        let spec = WavSpec {
            channels,
            sample_rate,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };

        Ok(Self {
            writer: WavWriter::create(path, spec)?,
            channels,
            sample_rate,
            frames_written: 0,
        })
    }

    /// Whether a frame's format matches this file's header.
    ///
    /// A mid-stream format change would silently play back at the wrong pitch,
    /// so callers check rather than let it through.
    pub fn accepts(&self, frame: &AudioFrame) -> bool {
        frame.channels == self.channels && frame.sample_rate == self.sample_rate
    }

    pub fn write(&mut self, frame: &AudioFrame) -> Result<()> {
        for &sample in &frame.samples {
            self.writer.write_sample(to_i16(sample))?;
        }
        self.frames_written += frame.frame_count() as u64;
        Ok(())
    }

    /// Writes `frames` sample-frames of silence.
    ///
    /// Used as a track's lead-in: the two capture engines (ScreenCaptureKit for
    /// system, voice processing for mic) do not start on the same instant, so each
    /// track is padded by the gap between recording start and its own first frame.
    /// That puts both files on one shared timeline, which is what lets the mixdown
    /// and per-speaker transcript line the sides up.
    pub fn write_silence(&mut self, frames: usize) -> Result<()> {
        for _ in 0..frames * self.channels as usize {
            self.writer.write_sample(0i16)?;
        }
        self.frames_written += frames as u64;
        Ok(())
    }

    pub fn frames_written(&self) -> u64 {
        self.frames_written
    }

    pub fn seconds_written(&self) -> f64 {
        self.frames_written as f64 / self.sample_rate as f64
    }

    /// Flushes and patches the WAV header with the final length.
    pub fn finalize(self) -> Result<()> {
        self.writer.finalize()?;
        Ok(())
    }
}

/// Clamps before scaling so inter-sample peaks above 0 dBFS wrap to the rail
/// instead of overflowing into the opposite sign.
fn to_i16(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_i16_maps_full_scale() {
        assert_eq!(to_i16(0.0), 0);
        assert_eq!(to_i16(1.0), i16::MAX);
        assert_eq!(to_i16(-1.0), -i16::MAX);
    }

    #[test]
    fn to_i16_clamps_out_of_range_input() {
        assert_eq!(to_i16(9.0), i16::MAX);
        assert_eq!(to_i16(-9.0), -i16::MAX);
    }
}
