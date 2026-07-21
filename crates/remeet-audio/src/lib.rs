//! Audio capture for Remeet.
//!
//! Captures a meeting as two separate tracks — system audio (the remote participants)
//! and microphone (the local user) — from a single ScreenCaptureKit stream via
//! [`DualCapture`]. One stream means both tracks share a clock; each frame carries a
//! host-clock [`pts`](AudioFrame::pts), and because the mic's first frame can arrive
//! later than the system's, the recorder aligns the tracks against a common start
//! captured with [`host_now`].
//!
//! Keeping the tracks separate is deliberate: it recovers "who said this" at the
//! granularity that matters (me vs. them) without any speaker diarization. The mic is
//! captured raw here — the remote's speaker bleed is removed afterwards by acoustic
//! echo cancellation (`remeet-aec`), which needs the untouched system track as its
//! reference. (Capturing the mic through macOS voice processing would cancel the bleed
//! at the source, but it silences ScreenCaptureKit's system capture — the two cannot
//! run together.)
//!
//! This crate is also where the raw Objective-C binding surface stops. Everything
//! above it sees [`AudioFrame`] and plain Rust types.

mod activity;
mod capture;
mod error;
mod frame;
mod sink;

pub use activity::CallWatcher;
pub use capture::{DualCapture, SystemCapture};
pub use error::{AudioError, Result};
pub use frame::{AudioFrame, Track, host_now};
pub use sink::WavSink;
