//! End-to-end check of the production capture + echo-cancellation path: records through
//! `Recorder` (ScreenCaptureKit dual-capture) for a few seconds, then the stop() runs
//! the AEC post-pass. Afterwards the directory should hold:
//!   system.wav          — the remote / system audio
//!   microphone_raw.wav  — the raw mic (bleed present)
//!   microphone.wav      — the cleaned mic (bleed cancelled)
//!
//! Play audio out the speakers and talk over it during the run.
//! Run: `cargo run -p remeet-session --example record -- /tmp/rec_aec 12`

use std::time::Duration;

use remeet_session::Recorder;

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let dir = args.next().unwrap_or_else(|| "/tmp/rec_aec".to_owned());
    let secs: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(12);

    let recorder = match Recorder::start(&dir).await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("could not start recording: {e}");
            std::process::exit(1);
        }
    };

    println!("recording {secs}s into {dir} — play audio on the speakers and talk over it…");
    tokio::time::sleep(Duration::from_secs(secs)).await;

    match recorder.stop().await {
        Ok(recording) => {
            println!("done — {} track(s):", recording.tracks.len());
            for t in &recording.tracks {
                println!("  {} ({:.1}s)", t.path.display(), t.duration.as_secs_f64());
            }
            println!("(microphone.wav is now AEC-cleaned; microphone_raw.wav is the original)");
        }
        Err(e) => {
            eprintln!("stop failed: {e}");
            std::process::exit(1);
        }
    }
}
