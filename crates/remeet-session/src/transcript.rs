use std::collections::HashSet;
use std::fmt;
use std::path::Path;
use std::time::Duration;

use remeet_audio::Track;
use remeet_transcribe::{
    DecodeOptions, LiveSegment, Transcriber, WHISPER_SAMPLE_RATE, denoise,
    prepare_for_whisper,
};

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

    /// How far apart two confidences must be to decide the leak on confidence alone.
    /// Within this margin the two copies are treated as equally clear, and the tie is
    /// broken by completeness (see `is_leak_of`).
    pub const CONFIDENCE_MARGIN: f32 = 0.02;
}

/// Which side of the conversation a line came from.
///
/// This is [`remeet_audio::Track`] restated in the vocabulary of a transcript: the
/// system track is the remote participants, the microphone track is the local user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
    options: &DecodeOptions,
) -> Result<Transcript> {
    transcribe_recording_streaming(transcriber, recording, language, options, |_, _| {})
}

/// Like [`transcribe_recording`], but calls `on_segment` with the speaker and each
/// segment as it is decoded, before the whole recording is done.
///
/// The live segments arrive per track (one track finishes before the next starts) and
/// are neither reordered nor bleed-suppressed — that only happens on the returned
/// [`Transcript`]. This is a progress feed, not the final result.
pub fn transcribe_recording_streaming<F>(
    transcriber: &Transcriber,
    recording: &Recording,
    language: Option<&str>,
    options: &DecodeOptions,
    on_segment: F,
) -> Result<Transcript>
where
    F: Fn(Speaker, LiveSegment) + Clone + 'static,
{
    // Cancel the remote's speaker bleed out of the mic before reading it. Idempotent
    // (a no-op once done) and best-effort: if it fails, transcribe the raw mic rather
    // than nothing — the bleed just risks the remote being transcribed off the mic too,
    // which `suppress_bleed` still trims at the text level below.
    if let Err(err) = crate::aec_pass::apply(&recording.dir) {
        eprintln!("echo cancellation skipped for transcription: {err}");
    }

    // Condition every track once, up front, so language detection and transcription
    // share the prepared audio instead of decoding each WAV twice.
    let mut prepared: Vec<(Speaker, Vec<f32>)> = Vec::with_capacity(recording.tracks.len());
    for track in &recording.tracks {
        let speaker = Speaker::from_track(track.track);
        let (samples, channels, sample_rate) = read_wav(&track.path)?;

        // Suppress room noise on the microphone before anything else, so the café din
        // never reaches Whisper. Only the mic — the system track is a clean digital
        // capture with no acoustic noise. Skipped for multi-channel audio, which the
        // mono noise model does not take.
        let samples = if options.denoise_mic && speaker == Speaker::Me && channels == 1 {
            denoise(&samples, sample_rate)
        } else {
            samples
        };

        prepared.push((speaker, prepare_for_whisper(&samples, channels, sample_rate)?));
    }

    // The remote's speaker bleed is already gone from the mic: it was cancelled off the
    // raw track when the recording stopped (see `aec_pass`, `remeet_aec`). So the mic
    // here is only the local voice, and no per-track gating is applied. `suppress_bleed`
    // below stays as a text-level backstop for any residual the AEC left.

    // Honour an explicit language; otherwise detect one for the whole meeting from the
    // loudest window across every track, and force it on all of them. Whisper's own
    // auto-detect reads only each track's opening seconds and locks it, so a track
    // that starts silent (the local mic while the other side talks) or with English
    // small talk gets mislabelled — and the whole side comes out wrong.
    let detected = match language {
        Some(explicit) => Some(explicit.to_owned()),
        None => detect_meeting_language(transcriber, &prepared),
    };
    let language = detected.as_deref();

    let mut candidates = Vec::new();
    for (speaker, audio) in &prepared {
        let speaker = *speaker;
        // One live callback per track, carrying that track's speaker.
        let live = on_segment.clone();
        let segments =
            transcriber.transcribe_prepared(audio, language, options, move |segment| {
                live(speaker, segment)
            })?;
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

/// Detects the meeting's language between Indonesian and English by decoding the
/// single loudest window across all tracks under each and keeping the more confident.
///
/// One decision for the whole meeting: both tracks are the same conversation, so a
/// quiet track never has to guess from its own thin audio. `None` falls back to
/// Whisper's per-track auto-detect when there is no audio to judge.
fn detect_meeting_language(
    transcriber: &Transcriber,
    prepared: &[(Speaker, Vec<f32>)],
) -> Option<String> {
    // The realistic pair for these meetings; auto-detect's failure is almost always
    // Indonesian speech mislabelled as English.
    const CANDIDATES: [&str; 2] = ["id", "en"];
    let window = loudest_window_across(prepared)?;
    transcriber.detect_language(window, &CANDIDATES)
}

/// The loudest fixed-length window across every track — the best bet for clear speech
/// to judge the language on, rather than a track's silent or small-talk opening.
fn loudest_window_across(prepared: &[(Speaker, Vec<f32>)]) -> Option<&[f32]> {
    const DETECT_SECS: usize = 30;
    let window = WHISPER_SAMPLE_RATE as usize * DETECT_SECS;

    let mut best: Option<(&[f32], f64)> = None;
    for (_, audio) in prepared {
        if audio.is_empty() {
            continue;
        }
        let candidate = loudest_window(audio, window.min(audio.len()));
        let level = energy(candidate);
        if best.as_ref().is_none_or(|(_, top)| level > *top) {
            best = Some((candidate, level));
        }
    }
    best.map(|(window, _)| window)
}

/// The `window`-length slice of `audio` with the most energy, scanned in coarse hops.
fn loudest_window(audio: &[f32], window: usize) -> &[f32] {
    if audio.len() <= window {
        return audio;
    }
    // ~5 s hops for a 30 s window: fine enough to land on real speech, coarse enough
    // to stay cheap on a long track.
    let step = (window / 6).max(1);
    let mut best_start = 0;
    let mut best = f64::MIN;
    let mut start = 0;
    while start + window <= audio.len() {
        let level = energy(&audio[start..start + window]);
        if level > best {
            best = level;
            best_start = start;
        }
        start += step;
    }
    &audio[best_start..best_start + window]
}

/// Sum of squares — relative loudness, enough to compare windows.
fn energy(samples: &[f32]) -> f64 {
    samples.iter().map(|&s| (s as f64) * (s as f64)).sum()
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
    // Drop hallucinations before anything else. Whisper invents a common phrase over a
    // whole non-speech window ("terima kasih", "sambil share design") — a handful of
    // words spread across many seconds. Real speech is dense, so a long segment with
    // very few words per second is not real. High confidence does not save it: Whisper
    // is sure of these, and VAD does not remove them, so this is the reliable signal.
    let candidates: Vec<Candidate> = candidates.into_iter().filter(|c| !is_sparse(c)).collect();

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

    // Collapse repeats: Whisper loops a phrase on silence and reports it many times for
    // the same speaker, scattered across the track ("gitu / gitu / gitu"). Keep the
    // first, drop the rest — a genuine line repeated word-for-word far apart is rare,
    // and losing it costs less than keeping the loop.
    let mut seen: HashSet<(Speaker, String)> = HashSet::new();
    lines.retain(|line| seen.insert((line.speaker, normalize(&line.text))));

    lines
}

/// Whether a segment is too sparse to be real speech — a few words stretched over a
/// long span, the shape of a hallucination on non-speech audio.
fn is_sparse(candidate: &Candidate) -> bool {
    const MIN_SECS: f64 = 4.0;
    const MIN_WORDS_PER_SEC: f64 = 0.4;

    let seconds = candidate.end.saturating_sub(candidate.start).as_secs_f64();
    if seconds <= MIN_SECS {
        return false;
    }
    let words = candidate.text.split_whitespace().count() as f64;
    words / seconds < MIN_WORDS_PER_SEC
}

/// Lowercased, punctuation-stripped words joined by single spaces — a canonical form
/// for comparing whether two lines say the same thing.
fn normalize(text: &str) -> String {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether `echo` is a leaked copy of `source` that should be dropped in its favour.
///
/// The two must overlap in time and read as the same words. Which of the pair is the
/// leak is then decided by confidence; when the two are within the confidence margin
/// (a clean speaker environment leaves both copies clear), the shorter one wins the
/// tie as the truncated echo, and an exact tie falls to a fixed side so the pair never
/// drops both or neither.
fn is_leak_of(echo: &Candidate, source: &Candidate) -> bool {
    if overlap_fraction(echo, source) < bleed::OVERLAP_MIN
        || text_similarity(&echo.text, &source.text) < bleed::TEXT_SIMILARITY
    {
        return false;
    }

    let confidence_gap = source.confidence - echo.confidence;
    if confidence_gap > bleed::CONFIDENCE_MARGIN {
        return true; // echo is clearly less confident
    }
    if confidence_gap < -bleed::CONFIDENCE_MARGIN {
        return false; // echo is clearly more confident; source is the leak
    }

    // Confidences tie: prefer the more complete transcription, then a fixed side.
    let (echo_words, source_words) = (word_count(&echo.text), word_count(&source.text));
    if echo_words != source_words {
        echo_words < source_words
    } else {
        echo.speaker == Speaker::Me
    }
}

fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
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
pub(crate) fn read_wav(path: &Path) -> Result<(Vec<f32>, u16, u32)> {
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
    fn breaks_confidence_tie_by_dropping_the_shorter_copy() {
        // The reported case: the same utterance on both tracks at the same moment, one
        // a truncated copy of the other, confidences too close to separate. The
        // shorter copy is the echo, so it is dropped and the fuller line survives.
        let full = candidate(
            Speaker::Them,
            0,
            6_000,
            0.83,
            "pasti sih si Hermes juga by default dia pakai ini tak whisper",
        );
        let truncated = candidate(
            Speaker::Me,
            0,
            5_000,
            0.82,
            "pasti sih si Hermes juga by default dia pakai ini",
        );

        let lines = suppress_bleed(vec![full, truncated]);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].speaker, Speaker::Them);
    }

    #[test]
    fn keeps_overlapping_speech_with_different_words() {
        // Two people genuinely talking over each other say different things, so the
        // texts are not similar and both lines survive.
        let a = candidate(
            Speaker::Them,
            20_000,
            26_000,
            0.88,
            "so when can we ship the migration",
        );
        let b = candidate(
            Speaker::Me,
            21_000,
            25_000,
            0.85,
            "i think friday works for the deploy",
        );

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
