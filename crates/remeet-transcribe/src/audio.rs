use rubato::{FftFixedIn, Resampler};

use crate::error::{Result, TranscribeError};

/// The only sample rate Whisper accepts. Everything is converted to this.
pub const WHISPER_SAMPLE_RATE: u32 = 16_000;

/// Input chunk size fed to the resampler per step. A power of two keeps the
/// internal FFT efficient; the exact value only trades latency for throughput,
/// neither of which matters for whole-file batch conversion.
const RESAMPLE_CHUNK: usize = 1024;

/// Prepares captured audio for Whisper: mono, 16 kHz, f32.
///
/// Downmix happens before resampling so the (more expensive) resampler only runs
/// on one channel instead of two.
pub fn prepare_for_whisper(samples: &[f32], channels: u16, sample_rate: u32) -> Result<Vec<f32>> {
    if samples.is_empty() {
        return Err(TranscribeError::EmptyAudio);
    }

    let mono = downmix_to_mono(samples, channels);

    if sample_rate == WHISPER_SAMPLE_RATE {
        return Ok(mono);
    }

    resample(&mono, sample_rate, WHISPER_SAMPLE_RATE)
}

/// Averages interleaved channels down to a single channel.
///
/// A plain average, not a weighted downmix: the goal is intelligibility for an ASR
/// model, and preserving one voice over another would be actively wrong when either
/// channel might carry the speech.
pub fn downmix_to_mono(samples: &[f32], channels: u16) -> Vec<f32> {
    let channels = channels.max(1) as usize;
    if channels == 1 {
        return samples.to_vec();
    }

    samples
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect()
}

/// Resamples a mono signal with a band-limited FFT resampler.
///
/// FFT rather than naive decimation because 48 kHz carries content above the 8 kHz
/// Nyquist limit of 16 kHz; dropping samples without a low-pass would alias that
/// content down into the speech band as tones that were never spoken. `FftFixedIn`
/// band-limits as part of the transform.
///
/// The resampler adds a fixed startup delay ([`Resampler::output_delay`]) — a few
/// milliseconds of leading silence, identical on every track, so relative timing
/// across tracks is unaffected.
fn resample(mono: &[f32], from_hz: u32, to_hz: u32) -> Result<Vec<f32>> {
    let mut resampler =
        FftFixedIn::<f32>::new(from_hz as usize, to_hz as usize, RESAMPLE_CHUNK, 1, 1)?;

    // Upper bound: the output-to-input ratio plus one full output chunk of slack for
    // the flushed tail. Avoids reallocations during the loop.
    let capacity = mono.len() * to_hz as usize / from_hz as usize + resampler.output_frames_max();
    let mut out = Vec::with_capacity(capacity);

    let mut pos = 0;
    let mut input = [mono]; // reused single-channel wrapper

    // `FftFixedIn` consumes a constant number of input frames per call.
    while pos + resampler.input_frames_next() <= mono.len() {
        let take = resampler.input_frames_next();
        input[0] = &mono[pos..pos + take];
        let resampled = resampler.process(&input, None)?;
        out.extend_from_slice(&resampled[0]);
        pos += take;
    }

    // Flush the final partial chunk; the resampler zero-pads it internally. Skipped
    // when the input divided evenly into chunks, because `process_partial` rejects a
    // zero-length buffer — and there is nothing left to flush in that case anyway.
    if pos < mono.len() {
        let tail = [&mono[pos..]];
        let resampled = resampler.process_partial(Some(&tail), None)?;
        out.extend_from_slice(&resampled[0]);
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downmix_averages_stereo() {
        // L/R interleaved: (1,3) -> 2, (2,4) -> 3
        let stereo = [1.0, 3.0, 2.0, 4.0];
        assert_eq!(downmix_to_mono(&stereo, 2), vec![2.0, 3.0]);
    }

    #[test]
    fn downmix_passes_mono_through() {
        let mono = [1.0, 2.0, 3.0];
        assert_eq!(downmix_to_mono(&mono, 1), mono);
    }

    #[test]
    fn resample_thirds_the_length() {
        // 48 kHz -> 16 kHz is 3:1, so ~1/3 the samples come out (within the
        // resampler's startup/flush slack).
        let input: Vec<f32> = (0..48_000).map(|i| (i as f32 * 0.01).sin()).collect();
        let out = resample(&input, 48_000, 16_000).expect("resample");
        let expected = input.len() / 3;
        let drift = (out.len() as i64 - expected as i64).abs();
        assert!(
            drift < RESAMPLE_CHUNK as i64,
            "got {} samples, expected ~{expected}",
            out.len()
        );
    }

    #[test]
    fn resample_handles_input_that_divides_evenly_into_chunks() {
        // Regression: the real microphone track was exactly 1024 * 1406 samples, so
        // the chunk loop consumed everything and left a zero-length tail, which
        // `process_partial` rejects. Sweep several exact multiples to be sure.
        for chunks in [1, 2, 1406] {
            let n = RESAMPLE_CHUNK * chunks;
            let input: Vec<f32> = (0..n).map(|i| (i as f32 * 0.01).sin()).collect();
            assert!(
                resample(&input, 48_000, 16_000).is_ok(),
                "failed for {n} samples ({chunks} chunks)"
            );
        }
    }

    #[test]
    fn prepare_short_circuits_when_already_16k_mono() {
        let mono = vec![0.1, 0.2, 0.3];
        let out = prepare_for_whisper(&mono, 1, WHISPER_SAMPLE_RATE).expect("prepare");
        assert_eq!(out, mono);
    }

    #[test]
    fn prepare_rejects_empty() {
        assert!(matches!(
            prepare_for_whisper(&[], 1, WHISPER_SAMPLE_RATE),
            Err(TranscribeError::EmptyAudio)
        ));
    }
}
