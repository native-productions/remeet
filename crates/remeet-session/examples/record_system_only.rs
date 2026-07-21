//! Isolation test: capture system audio with `SystemCapture` ALONE — no VPIO mic
//! engine running — to confirm whether the all-silent `system.wav` seen when VPIO is
//! active is caused by the voice-processing unit reconfiguring the audio HAL, or by a
//! bug in the system-only capture itself.
//!
//! Play audio out the speakers during the run. If `system.wav` here has audio, SCK
//! system capture is fine on its own and VPIO is the conflict; if it is still silent,
//! the bug is in the capture path.
//!
//! Run: `cargo run -p remeet-session --example record_system_only -- /tmp/sys_only 8`

use std::path::Path;
use std::time::Duration;

use crossbeam_channel::unbounded;
use remeet_audio::{AudioFrame, SystemCapture, Track, WavSink};

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let dir = args.next().unwrap_or_else(|| "/tmp/sys_only".to_owned());
    let secs: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(8);
    std::fs::create_dir_all(&dir).unwrap();

    let (tx, rx) = unbounded::<AudioFrame>();
    let capture = match SystemCapture::start(tx).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("start failed: {e}");
            std::process::exit(1);
        }
    };

    // Drain to system.wav on a plain thread. The sender lives in the capture delegate
    // and is dropped by `stop()`, which disconnects the receiver and ends this loop.
    let out = Path::new(&dir).join("system.wav");
    let collector = std::thread::spawn(move || {
        let mut sink: Option<WavSink> = None;
        while let Ok(frame) = rx.recv() {
            if frame.track != Track::System {
                continue;
            }
            let s = sink.get_or_insert_with(|| {
                WavSink::create(&out, frame.sample_rate, frame.channels).unwrap()
            });
            if s.accepts(&frame) {
                s.write(&frame).unwrap();
            }
        }
        let secs = sink.as_ref().map_or(0.0, WavSink::seconds_written);
        if let Some(s) = sink {
            s.finalize().unwrap();
        }
        secs
    });

    println!("capturing system audio {secs}s into {dir} — play audio on the speakers…");
    tokio::time::sleep(Duration::from_secs(secs)).await;
    capture.stop().await.unwrap();

    let written = collector.join().unwrap();
    println!("wrote {written:.1}s to {dir}/system.wav");
}
