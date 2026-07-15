# Remeet

Meeting capture, transcription, and action items for macOS.

Status: **spike**. Audio capture is proven; nothing else is built yet.

## Layout

```
crates/
  remeet-audio/      Capture + WAV sink. The only crate that touches Objective-C.
spikes/
  dual-capture/      Throwaway: proves both meeting sides record to separate tracks.
```

## Design

**Two tracks, one stream.** A meeting has two sides: what the machine plays back
(the remote participants) and what the microphone hears (you). ScreenCaptureKit
delivers both from a single `SCStream` as separate output types, so `remeet-audio`
keeps them separate rather than mixing.

This matters for action items. A mixed track gives you

> "I'll handle the deploy, you take the migration"

with no way to tell whose task is whose. Two tracks answer that without any speaker
diarization — not *who* said it by name, but *me vs. them*, which is the part that
decides whose todo list a line lands on.

Using one stream for both is also why the tracks stay aligned: they share a clock,
so presentation timestamps are directly comparable. Capturing the mic through a
second API (`cpal`, say) would introduce a second clock and drift to correct for.

## Building

Requires the Rust toolchain pinned in `rust-toolchain.toml`. No other setup —
`cidre` binds Objective-C directly, so there is no Swift bridge and no Xcode
version floor beyond the macOS 15 SDK.

```sh
cargo build
cargo test
```

### Why cidre and not the `screencapturekit` crate

The `screencapturekit` crate pulls in `apple-metal`, whose Swift bridge needs the
macOS 26 SDK → Xcode 26 → macOS Tahoe. It does not build on macOS 15. Its own
build script also gates Swift features on the SDK *major* version only, so a 15.1
SDK gets told it has 15.2 APIs and fails to compile.

`cidre` has no Swift bridge, builds in seconds, and exposes the same
ScreenCaptureKit surface including `setCaptureMicrophone:` (macOS 15+).

## Running the spike

```sh
cargo run --release -p dual-capture
```

Records 30 seconds to `recordings/system.wav` and `recordings/microphone.wav`.

It fails loudly on the two things that are invisible from an exit code: a track
that produced no audio, and audio whose duration disagrees with the wall clock
(which means the sample rate or channel count was misread — the WAV would still
play, just at the wrong speed).

Verify the rest by ear:

```sh
afplay recordings/system.wav
afplay recordings/microphone.wav
```

### Permissions

Needs **Screen Recording** (for system audio, even though no video is captured)
and **Microphone**. Both prompt on first run and are attributed to the enclosing
app bundle — for a bare binary that means the terminal you launch it from, not the
binary itself.

### Wear headphones

Verified formats: system audio arrives as 48 kHz stereo, the built-in microphone as
48 kHz mono.

The track split is only as clean as your acoustic isolation. On speakers, the
microphone also picks up the remote participants coming out of them, and the "me
vs. them" split degrades — measured at roughly -35 dBFS of bleed in testing. On
headphones the microphone hears only you.
