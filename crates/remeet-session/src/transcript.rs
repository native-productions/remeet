use std::collections::HashSet;
use std::fmt;
use std::path::Path;
use std::time::Duration;

use remeet_audio::Track;
use remeet_transcribe::Transcriber;

use crate::Recording;
use crate::error::{Result, SessionError};

/// Cross-track bleed suppression thresholds.
///
/// On speakers, one side's audio leaks into the other side's track: the remote
/// through the microphone, and the local voice back into the system mix. That
/// produces a near-duplicate segment on both tracks at the same moment. The leaked
/// copy is acoustically degraded (room, speaker, re-capture), so Whisper transcribes
/// it with lower confidence than the clean, direct source. These thresholds decide
/// when a segment is that leak rather than genuine simultaneous speech.
///
/// Energy was tried first and rejected: on loud speakers the leak is not much quieter
/// than the source (a near-constant gain offset between the two capture paths), so
/// level does not tell the speaker apart. Confidence does.
mod bleed {
    /// Minimum fraction of the shorter of two cross-track segments that must overlap
    /// in time before they are considered the same moment.
    pub const OVERLAP_MIN: f64 = 0.5;

    /// The two texts must be at least this similar (Jaccard over words) before either
    /// is treated as a copy of the other, so two people genuinely talking over each
    /// other with different words are never collapsed.
    pub const TEXT_SIMILARITY: f32 = 0.4;

    /// The leaked copy's confidence must be at least this much below the source's.
    /// A small margin avoids dropping either of two equally clean, coincidentally
    /// similar segments.
    pub const CONFIDENCE_MARGIN: f32 = 0.02;
}

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
    let mut candidates = Vec::new();

    for track in &recording.tracks {
        let speaker = Speaker::from_track(track.track);
        let (samples, channels, sample_rate) = read_wav(&track.path)?;

        let segments = transcriber.transcribe(&samples, channels, sample_rate, language)?;
        for segment in segments {
            candidates.push(Candidate {
                speaker,
                start: segment.start,
                end: segment.end,
                text: segment.text,
                confidence: segment.confidence,
            });
        }
    }

    Ok(Transcript {
        lines: suppress_bleed(candidates),
    })
}

/// A transcribed segment plus the confidence it was heard with, before bleed
/// suppression decides whether to keep it.
#[derive(Debug, Clone)]
struct Candidate {
    speaker: Speaker,
    start: Duration,
    end: Duration,
    text: String,
    /// Mean token probability, from Whisper. The clean source scores higher than its
    /// leaked echo on the other track.
    confidence: f32,
}

/// Drops segments that are one track's audio leaking into the other, then returns the
/// survivors as an ordered transcript.
///
/// A candidate is dropped when it overlaps an opposite-track candidate in time, their
/// texts are similar, and it was heard with lower confidence. All three must hold:
/// overlap alone is backchannel, similar text alone is two people saying "yeah", low
/// confidence alone is just a mumbled line. Together they are an echo of the clearer
/// copy on the other track.
fn suppress_bleed(candidates: Vec<Candidate>) -> Vec<TranscriptLine> {
    let mut keep = vec![true; candidates.len()];

    for i in 0..candidates.len() {
        for j in 0..candidates.len() {
            if i == j || !keep[i] {
                continue;
            }
            let (echo, source) = (&candidates[i], &candidates[j]);
            if echo.speaker == source.speaker {
                continue;
            }
            if is_leak_of(echo, source) {
                keep[i] = false;
            }
        }
    }

    let mut lines: Vec<TranscriptLine> = candidates
        .into_iter()
        .zip(keep)
        .filter(|(_, keep)| *keep)
        .map(|(c, _)| TranscriptLine {
            speaker: c.speaker,
            start: c.start,
            end: c.end,
            text: c.text,
        })
        .collect();

    lines.sort_by_key(|line| line.start);
    lines
}

/// Whether `echo` is a leaked copy of `source`.
fn is_leak_of(echo: &Candidate, source: &Candidate) -> bool {
    overlap_fraction(echo, source) >= bleed::OVERLAP_MIN
        && text_similarity(&echo.text, &source.text) >= bleed::TEXT_SIMILARITY
        && echo.confidence < source.confidence - bleed::CONFIDENCE_MARGIN
}

/// Fraction of the shorter segment that overlaps the other in time.
fn overlap_fraction(a: &Candidate, b: &Candidate) -> f64 {
    let start = a.start.max(b.start);
    let end = a.end.min(b.end);
    let overlap = end.saturating_sub(start).as_secs_f64();
    if overlap <= 0.0 {
        return 0.0;
    }
    let shorter = a
        .end
        .saturating_sub(a.start)
        .min(b.end.saturating_sub(b.start))
        .as_secs_f64();
    if shorter <= 0.0 {
        return 0.0;
    }
    overlap / shorter
}

/// Jaccard similarity over the two texts' word sets, case- and punctuation-insensitive.
fn text_similarity(a: &str, b: &str) -> f32 {
    let wa = word_set(a);
    let wb = word_set(b);
    if wa.is_empty() || wb.is_empty() {
        return 0.0;
    }
    let intersection = wa.intersection(&wb).count() as f32;
    let union = wa.union(&wb).count() as f32;
    intersection / union
}

fn word_set(text: &str) -> HashSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect()
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

    fn candidate(
        speaker: Speaker,
        start_ms: u64,
        end_ms: u64,
        confidence: f32,
        text: &str,
    ) -> Candidate {
        Candidate {
            speaker,
            start: Duration::from_millis(start_ms),
            end: Duration::from_millis(end_ms),
            text: text.to_owned(),
            confidence,
        }
    }

    #[test]
    fn drops_lower_confidence_overlapping_duplicate() {
        // The reported case: the same phrase on both tracks at the same time. The
        // clean digital source (system) scores higher than its acoustic echo (mic).
        let source = candidate(
            Speaker::Them,
            20_000,
            26_000,
            0.86,
            "based on the experience we have integrating with others",
        );
        let echo = candidate(
            Speaker::Me,
            20_000,
            26_000,
            0.71,
            "based on the experience we have integrating",
        );

        let lines = suppress_bleed(vec![source, echo]);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].speaker, Speaker::Them);
    }

    #[test]
    fn keeps_both_when_confidence_is_close() {
        // Two people genuinely talking over each other: overlapping, similar-ish, but
        // both heard clearly, so neither is an echo of the other.
        let a = candidate(
            Speaker::Them,
            20_000,
            26_000,
            0.88,
            "the deploy is ready to go now",
        );
        let b = candidate(Speaker::Me, 21_000, 25_000, 0.87, "the deploy is ready");

        assert_eq!(suppress_bleed(vec![a, b]).len(), 2);
    }

    #[test]
    fn keeps_backchannel_with_different_words() {
        // A soft "yeah exactly" over the remote's sentence overlaps and is lower
        // confidence, but different words, so it is real speech, not an echo.
        let source = candidate(
            Speaker::Them,
            20_000,
            26_000,
            0.88,
            "so the migration has to run first before anything",
        );
        let back = candidate(Speaker::Me, 22_000, 23_000, 0.6, "yeah exactly");

        assert_eq!(suppress_bleed(vec![source, back]).len(), 2);
    }

    #[test]
    fn keeps_duplicate_that_does_not_overlap() {
        // Same words, lower confidence, but a different moment: really said twice.
        let source = candidate(Speaker::Them, 20_000, 24_000, 0.88, "sounds good to me");
        let later = candidate(Speaker::Me, 40_000, 44_000, 0.7, "sounds good to me");

        assert_eq!(suppress_bleed(vec![source, later]).len(), 2);
    }

    #[test]
    fn suppressed_lines_stay_time_ordered() {
        let first = candidate(Speaker::Me, 5_000, 7_000, 0.85, "hello there");
        let second = candidate(Speaker::Them, 10_000, 12_000, 0.85, "how are you");
        let lines = suppress_bleed(vec![second, first]);
        assert!(lines[0].start < lines[1].start);
    }

    #[test]
    fn text_similarity_is_symmetric_and_bounded() {
        assert_eq!(text_similarity("deploy the app", "deploy the app"), 1.0);
        assert_eq!(text_similarity("deploy", "migration"), 0.0);
        let s = text_similarity("Deploy the app now!", "deploy the app");
        assert!(s > 0.5 && s < 1.0, "partial overlap: {s}");
    }
}
