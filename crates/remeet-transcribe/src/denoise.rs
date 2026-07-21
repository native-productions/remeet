//! Background-noise suppression for the microphone track.
//!
//! In a noisy place — a café, a busy room — the mic picks up clatter, hum, and general
//! din alongside the local voice, and Whisper will happily transcribe the noise as
//! words. This runs the mic through RNNoise (via the pure-Rust `nnnoiseless`, so no
//! native dependency), which is trained to keep speech and strip non-speech noise.
//!
//! It removes *noise*, not other *people*: another voice in the room is speech, so it
//! survives. Suppressing a specific talker needs to know who the owner is — a separate,
//! much harder problem — and is left to the level gate in [`crate::isolate`], which at
//! least drops far, quiet voices while the owner is silent.

use nnnoiseless::DenoiseState;

/// RNNoise is defined at 48 kHz; anything else is passed through untouched.
const RATE: u32 = 48_000;

/// Returns `samples` (mono, 48 kHz, normalised `[-1, 1]`) with background noise
/// suppressed. Passes the input through unchanged when it is not 48 kHz mono or is
/// shorter than one frame — there is nothing to gain and the model expects 48 kHz.
pub fn denoise(samples: &[f32], sample_rate: u32) -> Vec<f32> {
    let frame = DenoiseState::FRAME_SIZE;
    if sample_rate != RATE || samples.len() < frame {
        return samples.to_vec();
    }

    let mut state = DenoiseState::new();
    let mut input = vec![0.0f32; frame];
    let mut output = vec![0.0f32; frame];
    let mut out = Vec::with_capacity(samples.len());

    let mut i = 0;
    while i + frame <= samples.len() {
        // RNNoise works in i16 amplitude range, not the normalised floats stored in the
        // WAV, so scale on the way in and back out.
        for (dst, &src) in input.iter_mut().zip(&samples[i..i + frame]) {
            *dst = src * 32768.0;
        }
        state.process_frame(&mut output, &input);
        out.extend(output.iter().map(|&s| s / 32768.0));
        i += frame;
    }
    // A trailing partial frame is left as-is; a few milliseconds not worth padding.
    out.extend_from_slice(&samples[i..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn energy(x: &[f32]) -> f64 {
        x.iter().map(|&s| s as f64 * s as f64).sum()
    }

    fn noise(len: usize, seed: u64) -> Vec<f32> {
        let mut state = seed | 1;
        (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state as i64 as f64 / i64::MAX as f64) as f32 * 0.3
            })
            .collect()
    }

    #[test]
    fn suppresses_pure_noise() {
        // No speech, only noise: RNNoise should pull it down. Flat synthetic noise is
        // the hardest case for it — real-world noise drops much further — so this only
        // asserts a clear reduction, not a specific figure.
        let input = noise(48_000 * 3, 0xC0FFEE);
        let output = denoise(&input, 48_000);
        assert_eq!(output.len(), input.len());
        // Judge after the model has settled, past the first second.
        let skip = 48_000;
        let ratio = energy(&output[skip..]) / energy(&input[skip..]);
        assert!(ratio < 0.85, "expected noise pulled down, kept {ratio:.2}");
    }

    #[test]
    fn passes_through_non_48k() {
        let input = vec![0.1, -0.2, 0.3, 0.4];
        assert_eq!(denoise(&input, 16_000), input);
    }
}
