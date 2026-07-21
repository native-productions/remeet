//! Acoustic echo cancellation for the microphone track.
//!
//! Remeet records the mic raw, so the remote's voice — played out the speakers —
//! bleeds into it. This wraps WebRTC's AudioProcessing (AEC3) to subtract that bleed
//! using the system track (what was played) as the reference. It is the same canceller
//! Chrome and Google Meet use; unlike a plain adaptive filter it has a nonlinear
//! residual-echo suppressor, which is what makes it work on the nonlinear speaker→mic
//! path a linear filter can barely touch.
//!
//! Why here and not at capture: capturing the mic through macOS voice processing would
//! cancel the bleed at the source, but that silences ScreenCaptureKit's system capture
//! — the two cannot run together. Software AEC over the two already-captured tracks is
//! the way that keeps both.
//!
//! ## Alignment
//!
//! AEC3 estimates the render→capture delay itself, but only within its search window.
//! The two tracks must already be roughly aligned — which the recorder guarantees by
//! padding each track's lead-in to a shared start. Feeding a badly offset reference
//! just leaves the bleed uncancelled; it does not corrupt the voice.

use webrtc_audio_processing::config::EchoCanceller;
use webrtc_audio_processing::{Config, Processor};

pub use webrtc_audio_processing::Error;

/// Cancels the speaker bleed in `mic` using `reference` (the system audio) as the
/// render signal. Both are mono at `sample_rate`. Returns the cleaned mic, the same
/// length as `mic`.
///
/// The reference may be shorter or longer than the mic; it is read positionally and
/// zero-padded past its end, which is correct because both tracks were aligned to a
/// shared start before this runs.
pub fn cancel(mic: &[f32], reference: &[f32], sample_rate: u32) -> Result<Vec<f32>, Error> {
    let ap = Processor::new(sample_rate)?;
    ap.set_config(Config {
        // Default echo canceller is Full (AEC3) with delay estimation.
        echo_canceller: Some(EchoCanceller::default()),
        ..Default::default()
    });

    let frame = ap.num_samples_per_frame();
    let mut out = Vec::with_capacity(mic.len());

    let mut i = 0;
    while i + frame <= mic.len() {
        // The render frame must be processed before the capture frame it echoes into.
        let mut render = vec![reference_frame(reference, i, frame)];
        ap.process_render_frame(&mut render)?;

        let mut capture = vec![mic[i..i + frame].to_vec()];
        ap.process_capture_frame(&mut capture)?;
        out.extend_from_slice(&capture[0]);

        i += frame;
    }

    // The trailing partial frame is shorter than the processor accepts; pass it through
    // untouched rather than dropping audio.
    out.extend_from_slice(&mic[i..]);
    Ok(out)
}

/// One reference frame at `start`, zero-padded if the reference ends early.
fn reference_frame(reference: &[f32], start: usize, len: usize) -> Vec<f32> {
    let mut frame = vec![0.0f32; len];
    if start < reference.len() {
        let available = &reference[start..reference.len().min(start + len)];
        frame[..available.len()].copy_from_slice(available);
    }
    frame
}
