//! Spike: capture the microphone through macOS Voice Processing (VPIO) and write it
//! to a WAV, to answer one question before committing to a full capture rewrite —
//! does macOS's acoustic echo cancellation remove the speaker bleed from *another*
//! app's output, or only from audio this process itself renders?
//!
//! How to read the result:
//!   1. Play music or a video out the Mac **speakers** (not headphones).
//!   2. Run this, and talk over it for ~15 s.
//!   3. Open the written WAV. If your voice is there but the speaker audio is gone,
//!      VPIO references the hardware output and is the right tool. If the speaker
//!      audio is still there, VPIO can't help a passive recorder and we pivot to a
//!      software AEC over the two already-synced ScreenCaptureKit tracks.
//!
//! Run: `cargo run -p remeet-audio --example vpio_mic -- /tmp/vpio.wav 15`

use std::sync::{Arc, Mutex};
use std::time::Duration;

use cidre::av;

fn main() {
    let mut args = std::env::args().skip(1);
    let out = args.next().unwrap_or_else(|| "/tmp/vpio.wav".to_owned());
    let secs: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(15);

    // Captured samples (channel 0) plus the format they arrived in, filled from the
    // tap's real-time thread.
    let captured: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let rate: Arc<Mutex<f64>> = Arc::new(Mutex::new(48_000.0));

    let mut engine = av::AudioEngine::new();
    let mut input = engine.input_node();

    // Turn on voice processing: this is what enables the echo canceller and noise
    // suppressor on the mic input.
    if let Err(err) = input.set_vp_enabled(true) {
        eprintln!("could not enable voice processing: {err:?}");
        return;
    }

    // The node's output format after VPIO — that is the shape the AEC'd samples arrive
    // in, so tap in the same format.
    let fmt = input.output_format_for_bus(0);
    let sample_rate = fmt.absd().sample_rate;
    *rate.lock().unwrap() = sample_rate;
    println!(
        "VPIO on. tap format: {} Hz, {} ch",
        sample_rate,
        fmt.channel_count()
    );

    let sink = captured.clone();
    let tap = move |buf: &av::AudioPcmBuf, _when: &av::AudioTime| {
        let frames = buf.frame_len() as usize;
        if let Some(channel) = buf.data_f32_at(0) {
            sink.lock().unwrap().extend_from_slice(&channel[..frames.min(channel.len())]);
        }
    };

    if let Err(err) = input.install_tap_on_bus(0, 4096, Some(&fmt), tap) {
        eprintln!("could not install tap: {err:?}");
        return;
    }

    engine.prepare();
    if let Err(err) = engine.start() {
        eprintln!("could not start engine: {err:?}");
        return;
    }

    println!("recording {secs}s — play audio on the speakers and talk over it…");
    std::thread::sleep(Duration::from_secs(secs));

    engine.stop();
    unsafe { input.remove_tap_on_bus_throws(0) };

    let samples = captured.lock().unwrap();
    let sample_rate = *rate.lock().unwrap();
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: sample_rate as u32,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(&out, spec).expect("create wav");
    for &s in samples.iter() {
        let clamped = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        writer.write_sample(clamped).expect("write sample");
    }
    writer.finalize().expect("finalize wav");

    println!("wrote {} samples to {out}", samples.len());
}
