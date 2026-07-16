use std::fmt;
use std::path::Path;
use std::time::Duration;

use remeet_audio::Track;
use remeet_transcribe::Transcriber;

use crate::Recording;
use crate::error::{Result, SessionError};

/// Which side of the conversation a line came from.
///
/// This is [`remeet_audio::Track`] restated in the vocabulary of a transcript: the
/// system track is the remote participants, the microphone track is the local user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Speaker {
    /// The local user (microphone track).
    Me,
    /// The remote participants (system-audio track).
    Them,
}

impl Speaker {
    fn from_track(track: Track) -> Self {
        match track {
            Track::Microphone => Self::Me,
            Track::System => Self::Them,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Me => "me",
            Self::Them => "them",
        }
    }
}

/// One attributed line of transcript.
#[derive(Debug, Clone)]
pub struct TranscriptLine {
    pub speaker: Speaker,
    pub start: Duration,
    pub end: Duration,
    pub text: String,
}

/// A full meeting transcript: both tracks' lines merged onto one timeline.
#[derive(Debug, Clone, Default)]
pub struct Transcript {
    pub lines: Vec<TranscriptLine>,
}

impl Transcript {
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

/// Renders as `[m:ss.d - m:ss.d] speaker: text`, one line per segment.
impl fmt::Display for Transcript {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for line in &self.lines {
            writeln!(
                f,
                "[{} - {}] {:<5} {}",
                timestamp(line.start),
                timestamp(line.end),
                format!("{}:", line.speaker.label()),
                line.text.trim()
            )?;
        }
        Ok(())
    }
}

/// Transcribes every track of a recording and merges them into one transcript.
///
/// Each track is transcribed independently, then the segments are interleaved by
/// start time. Because both tracks were captured on one clock, that ordering is a
/// faithful reconstruction of who spoke when — without any speaker diarization.
///
/// `language` is an ISO code (`"en"`, `"id"`) or `None` to auto-detect per track,
/// which is the right default for a meeting that switches languages.
pub fn transcribe_recording(
    transcriber: &Transcriber,
    recording: &Recording,
    language: Option<&str>,
) -> Result<Transcript> {
    let mut lines = Vec::new();

    for track in &recording.tracks {
        let speaker = Speaker::from_track(track.track);
        let (samples, channels, sample_rate) = read_wav(&track.path)?;

        let segments = transcriber.transcribe(&samples, channels, sample_rate, language)?;
        lines.extend(segments.into_iter().map(|segment| TranscriptLine {
            speaker,
            start: segment.start,
            end: segment.end,
            text: segment.text,
        }));
    }

    // Stable sort by start time keeps same-timestamp lines in track order, which is
    // deterministic because the recording's tracks are ordered.
    lines.sort_by_key(|line| line.start);

    Ok(Transcript { lines })
}

/// Reads a 16-bit PCM WAV into interleaved f32 in [-1, 1], with its format.
fn read_wav(path: &Path) -> Result<(Vec<f32>, u16, u32)> {
    let mut reader = hound::WavReader::open(path).map_err(|source| SessionError::WavRead {
        path: path.display().to_string(),
        source,
    })?;
    let spec = reader.spec();

    let samples = reader
        .samples::<i16>()
        .map(|s| s.map(|v| v as f32 / i16::MAX as f32))
        .collect::<std::result::Result<Vec<f32>, _>>()
        .map_err(|source| SessionError::WavRead {
            path: path.display().to_string(),
            source,
        })?;

    Ok((samples, spec.channels, spec.sample_rate))
}

/// Formats a timestamp as `m:ss.d`.
fn timestamp(d: Duration) -> String {
    let secs = d.as_secs_f64();
    format!("{}:{:04.1}", secs as u64 / 60, secs % 60.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(speaker: Speaker, start_ms: u64, text: &str) -> TranscriptLine {
        TranscriptLine {
            speaker,
            start: Duration::from_millis(start_ms),
            end: Duration::from_millis(start_ms + 1000),
            text: text.to_owned(),
        }
    }

    #[test]
    fn display_renders_speaker_and_time() {
        let transcript = Transcript {
            lines: vec![line(Speaker::Them, 14_000, "can you deploy?")],
        };
        let rendered = transcript.to_string();
        assert!(rendered.contains("them:"), "{rendered}");
        assert!(rendered.contains("0:14.0"), "{rendered}");
        assert!(rendered.contains("can you deploy?"), "{rendered}");
    }

    #[test]
    fn speaker_maps_from_track() {
        assert_eq!(Speaker::from_track(Track::Microphone), Speaker::Me);
        assert_eq!(Speaker::from_track(Track::System), Speaker::Them);
    }
}
