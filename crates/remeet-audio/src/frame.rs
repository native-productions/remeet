use std::time::Duration;

use cidre::{cat, cm, sc};

use crate::error::{AudioError, Result};

/// Sample rate assumed when a buffer carries no usable format description.
/// Matches what [`crate::capture`] requests.
const FALLBACK_SAMPLE_RATE: u32 = 48_000;

/// The host clock's current time as a `Duration`.
///
/// This is the one time base both capture engines agree on: ScreenCaptureKit stamps
/// its audio PTS with it, and an AVAudioTime host time converts onto it. Captured
/// once at the start of a recording, it is the origin every track's lead-in is
/// measured from.
pub fn host_now() -> Duration {
    Duration::from_secs_f64(cm::Clock::host_time_clock().time().as_secs())
}

/// Which side of the conversation a frame came from.
///
/// Kept as two tracks rather than one mix: the split is free (ScreenCaptureKit
/// delivers them separately) and downstream it is what lets us tell the user's own
/// action items apart from everyone else's without speaker diarization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Track {
    /// Everything the machine plays back — i.e. the remote participants.
    System,
    /// The local microphone — i.e. the user.
    Microphone,
}

impl Track {
    /// Maps a stream output type onto a track, ignoring video.
    pub(crate) fn from_output_type(kind: sc::OutputType) -> Option<Self> {
        match kind {
            sc::OutputType::Audio => Some(Self::System),
            sc::OutputType::Mic => Some(Self::Microphone),
            sc::OutputType::Screen => None,
        }
    }

    /// Stable identifier, used for filenames.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Microphone => "microphone",
        }
    }
}

/// One buffer of PCM audio, copied out of Core Media into owned memory.
///
/// Samples are interleaved, which is what both WAV and most resamplers expect.
#[derive(Debug, Clone)]
pub struct AudioFrame {
    pub track: Track,
    pub samples: Vec<f32>,
    pub channels: u16,
    pub sample_rate: u32,
    /// Presentation timestamp on the stream clock. Both tracks come from the same
    /// stream, so timestamps are comparable across tracks without drift correction.
    pub pts: Duration,
}

impl AudioFrame {
    /// Number of sample frames (a frame being one sample per channel).
    pub fn frame_count(&self) -> usize {
        self.samples.len() / self.channels.max(1) as usize
    }

    /// Wall-clock length of this buffer.
    pub fn duration(&self) -> Duration {
        Duration::from_secs_f64(self.frame_count() as f64 / self.sample_rate as f64)
    }
}

/// Reusable destination for Core Media's audio buffer list.
///
/// The list is sized from the sample buffer at runtime rather than guessed: the
/// microphone and the system mixer do not agree on channel counts, and a fixed
/// guess earns `kCMSampleBufferError_ArrayTooSmall` from whichever one it got wrong.
/// Held across callbacks so the steady state does not allocate.
pub(crate) struct BufListScratch(cat::AudioBufListN);

impl BufListScratch {
    pub(crate) fn new() -> Self {
        // Any size is fine; `audio_buf_list_n` resizes to what the buffer reports.
        Self(cat::AudioBufListN::new(std::mem::size_of::<u32>()))
    }
}

/// Copies a Core Media audio sample buffer into an owned [`AudioFrame`].
///
/// The copy is deliberate: the block buffer is only valid for the length of the
/// ScreenCaptureKit callback, and holding it would stall the capture queue.
pub(crate) fn frame_from_sample_buffer(
    track: Track,
    sample: &mut cm::SampleBuf,
    scratch: &mut BufListScratch,
) -> Result<AudioFrame> {
    let sample_rate = sample
        .format_desc()
        .and_then(|desc| desc.stream_basic_desc())
        .map(|asbd| asbd.sample_rate as u32)
        .filter(|rate| *rate > 0)
        .unwrap_or(FALLBACK_SAMPLE_RATE);

    let pts = sample.pts();
    let pts = if pts.is_valid() && pts.as_secs() > 0.0 {
        Duration::from_secs_f64(pts.as_secs())
    } else {
        Duration::ZERO
    };

    let block = sample
        .audio_buf_list_n(&mut scratch.0)
        .map_err(|err| AudioError::AudioBufferList(format!("{err:?}")))?;

    let buffer_count = block.list.number_buffers();
    let buffers = block.list.buffers();
    let buffers = buffers.get(..buffer_count).ok_or(AudioError::NoAudioData)?;

    let (samples, channels) = interleave(buffers)?;

    Ok(AudioFrame {
        track,
        samples,
        channels,
        sample_rate,
        pts,
    })
}

/// Flattens a list of `AudioBuffer`s into interleaved f32 samples.
///
/// ScreenCaptureKit delivers non-interleaved float32 — one buffer per channel, each
/// declaring a single channel. A single buffer declaring N channels (already
/// interleaved) is also handled, since the layout is a property of the format
/// description rather than a guarantee of the API.
fn interleave(buffers: &[cat::audio::Buf]) -> Result<(Vec<f32>, u16)> {
    if buffers.is_empty() {
        return Err(AudioError::NoAudioData);
    }

    // Already interleaved: one buffer carrying every channel.
    if buffers.len() == 1 {
        let channels = buffers[0].number_channels.max(1) as u16;
        return Ok((bytes_to_f32(buffer_bytes(&buffers[0])), channels));
    }

    // Planar: one buffer per channel.
    let planes: Vec<Vec<f32>> = buffers
        .iter()
        .map(|buf| bytes_to_f32(buffer_bytes(buf)))
        .collect();

    let frame_count = planes[0].len();
    if planes.iter().any(|plane| plane.len() != frame_count) {
        return Err(AudioError::RaggedPlanes(
            planes.iter().map(Vec::len).collect(),
        ));
    }

    let channels = u16::try_from(planes.len()).unwrap_or(u16::MAX);
    let mut samples = Vec::with_capacity(frame_count * planes.len());
    for i in 0..frame_count {
        for plane in &planes {
            samples.push(plane[i]);
        }
    }

    Ok((samples, channels))
}

/// Views one `AudioBuffer`'s data as bytes.
///
/// Returns empty rather than dereferencing null: Core Media leaves `data` null on an
/// empty buffer, and a silent stretch is not an error worth dropping a frame for.
fn buffer_bytes(buf: &cat::audio::Buf) -> &[u8] {
    if buf.data.is_null() || buf.data_bytes_size == 0 {
        return &[];
    }
    // SAFETY: Core Media guarantees `data` points to `data_bytes_size` readable bytes
    // owned by the retained block buffer, which outlives this borrow.
    unsafe { std::slice::from_raw_parts(buf.data, buf.data_bytes_size as usize) }
}

/// Reinterprets a little-endian f32 byte slice.
///
/// Goes through `from_le_bytes` rather than a pointer cast to sidestep alignment
/// entirely; the compiler turns this into the same loads.
fn bytes_to_f32(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf(bytes: &mut [u8], channels: u32) -> cat::audio::Buf {
        cat::audio::Buf {
            number_channels: channels,
            data_bytes_size: bytes.len() as u32,
            data: bytes.as_mut_ptr(),
        }
    }

    fn to_bytes(samples: &[f32]) -> Vec<u8> {
        samples.iter().flat_map(|s| s.to_le_bytes()).collect()
    }

    #[test]
    fn bytes_to_f32_round_trips() {
        let source = [0.0f32, 1.0, -1.0, 0.5];
        assert_eq!(bytes_to_f32(&to_bytes(&source)), source);
    }

    #[test]
    fn bytes_to_f32_ignores_trailing_partial_sample() {
        assert_eq!(bytes_to_f32(&[0u8, 0, 0, 0, 0xFF]), vec![0.0]);
    }

    #[test]
    fn interleave_zips_planar_channels() {
        let mut left = to_bytes(&[1.0, 3.0]);
        let mut right = to_bytes(&[2.0, 4.0]);
        let buffers = [buf(&mut left, 1), buf(&mut right, 1)];

        let (samples, channels) = interleave(&buffers).expect("interleave");
        assert_eq!(channels, 2);
        assert_eq!(samples, vec![1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn interleave_zips_more_than_two_planes() {
        let mut a = to_bytes(&[1.0, 4.0]);
        let mut b = to_bytes(&[2.0, 5.0]);
        let mut c = to_bytes(&[3.0, 6.0]);
        let buffers = [buf(&mut a, 1), buf(&mut b, 1), buf(&mut c, 1)];

        let (samples, channels) = interleave(&buffers).expect("interleave");
        assert_eq!(channels, 3);
        assert_eq!(samples, vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
    }

    #[test]
    fn interleave_passes_through_single_buffer() {
        let interleaved = [1.0f32, 2.0, 3.0, 4.0];
        let mut bytes = to_bytes(&interleaved);
        let buffers = [buf(&mut bytes, 2)];

        let (samples, channels) = interleave(&buffers).expect("interleave");
        assert_eq!(channels, 2);
        assert_eq!(samples, interleaved);
    }

    #[test]
    fn interleave_rejects_ragged_planes() {
        let mut long = to_bytes(&[1.0, 2.0]);
        let mut short = to_bytes(&[3.0]);
        let buffers = [buf(&mut long, 1), buf(&mut short, 1)];

        assert!(matches!(
            interleave(&buffers),
            Err(AudioError::RaggedPlanes(_))
        ));
    }

    #[test]
    fn interleave_rejects_empty_list() {
        assert!(matches!(interleave(&[]), Err(AudioError::NoAudioData)));
    }
}
