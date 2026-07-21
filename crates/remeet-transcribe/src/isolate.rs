//! Isolates the local voice on the microphone track by gating out the bleed.
//!
//! On speakers the remote participants play out loud and the mic picks them up, so the
//! mic track carries the local voice *plus* a quieter acoustic copy of the remote.
//! Transcribing it alongside the clean system track then reports the remote twice.
//!
//! Cancelling that bleed with an adaptive filter does not work here — measured on real
//! recordings, the speaker-to-mic path is nonlinear enough that even the best linear
//! filter removes almost nothing. So this does not try to cancel it. It *gates*: the
//! local speaker is right on the mic and comes in loud, while the bleed arrives faint,
//! at roughly a fixed fraction of the system level. A frame is kept as local voice
//! only when the mic rises clearly above that expected bleed; the rest is silenced.
//!
//! Detecting the bleed is far easier than cancelling it — a level comparison, immune
//! to the nonlinearity that defeats a filter. The result feeds the mic ("me") track;
//! the remote ("them") comes from the clean system capture, so each side is
//! transcribed once, from its better source.

use crate::WHISPER_SAMPLE_RATE;

/// Analysis frame, in milliseconds. Long enough to average out speech fine-structure
/// into a stable level, short enough to follow turn-taking.
const FRAME_MS: usize = 30;
/// Largest misalignment searched between the mic and system level envelopes.
const MAX_LAG_FRAMES: isize = 40;
/// How far above the estimated bleed level the mic must rise to count as local speech.
const MARGIN: f32 = 2.5;
/// Frames the gate stays open after speech ends. Long enough (~150 ms) to bridge the
/// quiet gaps between words so a sentence is not chopped into pieces, short enough not
/// to hold open across a long stretch of pure bleed.
const HANGOVER_FRAMES: u32 = 5;
const EPS: f32 = 1e-9;

/// Returns the mic with everything but the local voice silenced, using `reference`
/// (the system track) to know how loud the bleed should be.
///
/// Both are 16 kHz mono. Falls back to the mic unchanged when there is too little
/// audio to estimate levels from.
pub fn isolate_local(mic: &[f32], reference: &[f32]) -> Vec<f32> {
    let hop = WHISPER_SAMPLE_RATE as usize * FRAME_MS / 1000;
    let frames = mic.len().min(reference.len()) / hop;
    if frames < 8 {
        return mic.to_vec();
    }

    let mic_level: Vec<f32> = (0..frames).map(|i| rms(&mic[i * hop..(i + 1) * hop])).collect();
    let ref_level: Vec<f32> =
        (0..frames).map(|i| rms(&reference[i * hop..(i + 1) * hop])).collect();

    // Align the two level envelopes: the bleed lags the system by the acoustic delay,
    // and the tracks can start a little apart.
    let lag = align(&mic_level, &ref_level);
    let ref_level = shift(&ref_level, lag);

    // Bleed level as a fraction of the system level, learned from the frames where the
    // system is active. The low quantile picks the moments that are bleed *without*
    // local speech on top — the floor of the mic/system ratio.
    let active = percentile(&ref_level, 0.60);
    let mut ratios: Vec<f32> = (0..frames)
        .filter(|&i| ref_level[i] > active)
        .map(|i| mic_level[i] / (ref_level[i] + EPS))
        .collect();
    let bleed = if ratios.is_empty() {
        0.0
    } else {
        percentile_owned(&mut ratios, 0.25)
    };

    // A frame is local speech when the mic clears both a quiet-noise floor and the
    // expected bleed level by a margin. The floor sits low — near the silence level,
    // not the median — so quiet syllables of the local voice are kept, not clipped;
    // its only job is to drop true silence.
    let floor = percentile(&mic_level, 0.15);
    let mut keep: Vec<bool> = (0..frames)
        .map(|i| mic_level[i] > floor.max(bleed * ref_level[i] * MARGIN))
        .collect();

    // Hangover: hold the gate open for a couple of frames past the end of speech so a
    // trailing syllable at lower level is not clipped. Bounded and driven off the
    // original decision, not the running one — otherwise each held frame would re-open
    // the next and the gate would never close over a long stretch of bleed.
    let decided = keep.clone();
    let mut hold = 0u32;
    for i in 0..frames {
        if decided[i] {
            hold = HANGOVER_FRAMES;
        } else if hold > 0 {
            keep[i] = true;
            hold -= 1;
        }
    }

    let mut out = mic.to_vec();
    for (i, &keep) in keep.iter().enumerate() {
        if !keep {
            out[i * hop..(i + 1) * hop].fill(0.0);
        }
    }
    out
}

fn rms(frame: &[f32]) -> f32 {
    (frame.iter().map(|&s| s * s).sum::<f32>() / frame.len() as f32).sqrt()
}

/// Lag (in frames) that best aligns envelope `b` under `a`, so `a[i] ~ b[i - lag]`.
fn align(a: &[f32], b: &[f32]) -> isize {
    let n = a.len().min(b.len()) as isize;
    let mut best_lag = 0;
    let mut best = f64::MIN;
    for lag in -MAX_LAG_FRAMES..=MAX_LAG_FRAMES {
        let mut acc = 0.0;
        for i in 0..n {
            let j = i - lag;
            if j >= 0 && j < n {
                acc += a[i as usize] as f64 * b[j as usize] as f64;
            }
        }
        if acc > best {
            best = acc;
            best_lag = lag;
        }
    }
    best_lag
}

/// `env` shifted by `lag` frames: `out[i] = env[i - lag]`, zero outside.
fn shift(env: &[f32], lag: isize) -> Vec<f32> {
    let n = env.len() as isize;
    (0..n)
        .map(|i| {
            let j = i - lag;
            if j >= 0 && j < n {
                env[j as usize]
            } else {
                0.0
            }
        })
        .collect()
}

/// The `p`-quantile of `x` (0.0..=1.0), by copy so the input is untouched.
fn percentile(x: &[f32], p: f32) -> f32 {
    percentile_owned(&mut x.to_vec(), p)
}

fn percentile_owned(x: &mut [f32], p: f32) -> f32 {
    if x.is_empty() {
        return 0.0;
    }
    x.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = (((x.len() - 1) as f32) * p).round() as usize;
    x[idx]
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
                (state as i64 as f64 / i64::MAX as f64) as f32
            })
            .collect()
    }

    #[test]
    fn keeps_local_and_mutes_bleed() {
        let n = WHISPER_SAMPLE_RATE as usize * 6;
        // The remote talks throughout; its faint bleed sits on the mic the whole time.
        let system = noise(n, 0xA5A5);
        let local = noise(n, 0x1357);
        let mut mic = vec![0.0f32; n];
        for i in 0..n {
            mic[i] = 0.4 * system[i]; // bleed everywhere
            if i > n / 2 {
                mic[i] += 3.0 * local[i]; // local speaks, loud, in the second half
            }
        }

        let gated = isolate_local(&mic, &system);

        // The bleed-only first half is mostly silenced; the local second half survives.
        let bleed_kept = energy(&gated[..n / 2]) / energy(&mic[..n / 2]);
        let local_kept = energy(&gated[n / 2..]) / energy(&mic[n / 2..]);
        assert!(bleed_kept < 0.2, "bleed-only should be muted, kept {bleed_kept:.2}");
        assert!(local_kept > 0.5, "local speech should survive, kept {local_kept:.2}");
    }

    #[test]
    fn short_input_passes_through() {
        let mic = vec![0.1, -0.2, 0.3];
        assert_eq!(isolate_local(&mic, &[0.05, 0.05, 0.05]), mic);
    }
}
