use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crossbeam_channel::{Receiver, RecvTimeoutError};
use remeet_audio::{AudioFrame, DualCapture, Track, WavSink, host_now};
use tokio::task::JoinHandle;

use crate::error::{Result, SessionError};
use crate::{Recording, TrackRecording};

/// How often the collector wakes to re-check the stop flag while no frames arrive.
/// Only relevant after capture stops; during recording, frames arrive far faster.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// A track's WAV file while it is being written.
struct OpenTrack {
    path: PathBuf,
    sink: WavSink,
}

/// A live recording session writing each captured track to its own WAV file.
///
/// [`start`](Self::start) begins capturing immediately; [`stop`](Self::stop)
/// finalizes the files and returns a [`Recording`]. Recording and transcription are
/// deliberately separate steps — the WAVs on disk are the hand-off, so a recording
/// can be transcribed later, or not at all.
pub struct Recorder {
    dir: PathBuf,
    capture: DualCapture,
    running: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    collector: JoinHandle<Result<HashMap<Track, OpenTrack>>>,
}

impl Recorder {
    /// Starts capturing into `dir`, one WAV per track. Creates `dir` if needed.
    ///
    /// Requires Screen Recording and Microphone permission (see [`remeet_audio`]).
    pub async fn start(dir: impl AsRef<Path>) -> Result<Self> {
        let dir = dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir)?;

        let (tx, rx) = crossbeam_channel::unbounded::<AudioFrame>();

        // Origin for every track's lead-in, captured before capture starts so both
        // tracks' first frames land at or after it. The two tracks share the stream
        // clock, so the difference between their first pts is the real start offset —
        // the mic's first frame commonly arrives a beat after the system's.
        let t0 = host_now();
        let capture = DualCapture::start(tx).await?;

        let running = Arc::new(AtomicBool::new(true));
        let paused = Arc::new(AtomicBool::new(false));
        let collector = {
            let running = Arc::clone(&running);
            let paused = Arc::clone(&paused);
            let dir = dir.clone();
            // Writing WAVs blocks, so the collector runs off the async worker pool.
            tokio::task::spawn_blocking(move || collect(rx, &dir, &running, &paused, t0))
        };

        Ok(Self {
            dir,
            capture,
            running,
            paused,
            collector,
        })
    }

    /// The directory the tracks are being written to.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Stops writing captured frames to disk without tearing down the stream.
    ///
    /// The capture stream stays live — only the collector stops appending. Because
    /// each track's WAV is built frame by frame, the dropped frames leave no silent
    /// gap: resumed audio concatenates straight onto the paused audio. Keeping the
    /// stream up (rather than [`stop`](Self::stop)/[`start`](Self::start)) avoids a
    /// second Screen Recording permission prompt and the multi-second ScreenCaptureKit
    /// spin-up on every resume.
    pub fn pause(&self) {
        self.paused.store(true, Ordering::Release);
    }

    /// Resumes writing captured frames after a [`pause`](Self::pause).
    pub fn resume(&self) {
        self.paused.store(false, Ordering::Release);
    }

    /// Stops capturing, finalizes the WAV files, and returns the [`Recording`].
    pub async fn stop(self) -> Result<Recording> {
        // Stop the stream first: once this returns, ScreenCaptureKit delivers no
        // further frames, so the collector can safely drain what is already queued.
        self.capture.stop().await?;
        self.running.store(false, Ordering::Release);

        let open = self
            .collector
            .await
            .map_err(|_| SessionError::CollectorPanicked)??;

        if open.is_empty() {
            return Err(SessionError::NothingCaptured);
        }

        let mut tracks = Vec::with_capacity(open.len());
        for (track, OpenTrack { path, sink }) in open {
            let duration = Duration::from_secs_f64(sink.seconds_written());
            sink.finalize()?;
            tracks.push(TrackRecording {
                track,
                path,
                duration,
            });
        }
        // Deterministic order regardless of which track produced its first frame first.
        tracks.sort_by_key(|t| t.track.as_str());

        // The mic's echo cancellation is deliberately NOT run here: it processes the
        // whole recording and would stall `stop` (and this call holds the session lock),
        // making the Stop button appear dead. It runs off the stop path instead — see
        // [`apply_echo_cancellation`](crate::apply_echo_cancellation), which the app
        // kicks off in the background and which the transcript/mixdown paths also call
        // (idempotently) before they read the mic.
        Ok(Recording {
            dir: self.dir,
            tracks,
        })
    }
}

/// Drains frames into per-track WAV files until stopped, then flushes what's queued.
///
/// Termination is driven by the `running` flag, not by a receive timeout: a timeout
/// cannot tell a slow start (a permission prompt can delay the first frame for a long
/// time) apart from a finished recording. The flag is unambiguous.
fn collect(
    rx: Receiver<AudioFrame>,
    dir: &Path,
    running: &AtomicBool,
    paused: &AtomicBool,
    t0: Duration,
) -> Result<HashMap<Track, OpenTrack>> {
    let mut open: HashMap<Track, OpenTrack> = HashMap::new();

    while running.load(Ordering::Acquire) {
        match rx.recv_timeout(POLL_INTERVAL) {
            // While paused the frame is still received and then dropped, so the
            // channel keeps draining rather than backing up until resume.
            Ok(frame) if paused.load(Ordering::Acquire) => drop(frame),
            Ok(frame) => route(&mut open, dir, frame, t0)?,
            Err(RecvTimeoutError::Timeout) => continue,
            // The capture side dropped its sender — nothing more will ever arrive.
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }

    // Flush frames that were queued between the stop signal and this point. A pause
    // in effect at stop still discards them: the user asked for this tail to be off.
    while let Ok(frame) = rx.try_recv() {
        if paused.load(Ordering::Acquire) {
            continue;
        }
        route(&mut open, dir, frame, t0)?;
    }

    Ok(open)
}

/// Longest lead-in silence a track may be padded with. A frame timestamp far ahead of
/// the recording start is a bad clock reading, not a real offset — cap it so one bogus
/// pts cannot write gigabytes of silence.
const MAX_LEAD: Duration = Duration::from_secs(10);

/// Appends a frame to its track's file, opening the file on first sight of the track.
///
/// The WAV header needs a sample rate and channel count up front, known only once a
/// real frame arrives, so files open lazily. A mid-stream format change (never
/// observed in practice) skips the odd frame rather than aborting the whole
/// recording — losing a meeting to one bad buffer would be the worse failure.
fn route(
    open: &mut HashMap<Track, OpenTrack>,
    dir: &Path,
    frame: AudioFrame,
    t0: Duration,
) -> Result<()> {
    let track = match open.entry(frame.track) {
        Entry::Occupied(entry) => entry.into_mut(),
        Entry::Vacant(entry) => {
            let path = dir.join(format!("{}.wav", frame.track.as_str()));
            let mut sink = WavSink::create(&path, frame.sample_rate, frame.channels)?;

            // Pad the gap between recording start and this track's first frame, so all
            // tracks share one timeline. An invalid pts subtracts to zero (no lead);
            // an implausibly large one is ignored rather than trusted.
            let lead = frame.pts.checked_sub(t0).unwrap_or_default();
            if lead > Duration::ZERO && lead <= MAX_LEAD {
                let silent_frames = (lead.as_secs_f64() * frame.sample_rate as f64).round() as usize;
                sink.write_silence(silent_frames)?;
            }

            entry.insert(OpenTrack { path, sink })
        }
    };

    if track.sink.accepts(&frame) {
        track.sink.write(&frame)?;
    }

    Ok(())
}
