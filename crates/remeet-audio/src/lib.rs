//! Audio capture for Remeet.
//!
//! Captures a meeting as two separate tracks — system audio (the remote
//! participants) and microphone (the local user) — from a single ScreenCaptureKit
//! stream.
//!
//! Keeping the tracks separate is deliberate. ScreenCaptureKit hands them over
//! already split, so preserving that costs nothing, and it recovers "who said this"
//! at the granularity that matters (me vs. them) without any speaker diarization.
//! Mixing to a single track would throw that away.
//!
//! This crate is also where the raw Objective-C binding surface stops. Everything
//! above it sees [`AudioFrame`] and plain Rust types.

mod activity;
mod capture;
mod error;
mod frame;
mod sink;

pub use activity::CallWatcher;
pub use capture::DualCapture;
pub use error::{AudioError, Result};
pub use frame::{AudioFrame, Track};
pub use sink::WavSink;
