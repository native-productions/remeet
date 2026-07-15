// Fires inside cidre's `define_obj_type!` expansion, which builds the Objective-C
// class pair. Scoped to this file, which contains no hand-written transmutes.
#![allow(clippy::useless_transmute)]

use cidre::sc::stream::{Output, OutputImpl};
use cidre::{arc, cm, define_obj_type, dispatch, ns, objc, sc};
use crossbeam_channel::Sender;

use crate::error::{AudioError, Result};
use crate::frame::{AudioFrame, BufListScratch, Track, frame_from_sample_buffer};

/// 48 kHz is the system mixer's native rate, so asking for it avoids a resample
/// inside ScreenCaptureKit. Whisper's 16 kHz downsample happens later, once.
const SAMPLE_RATE: i64 = 48_000;
const CHANNEL_COUNT: i64 = 2;

/// ScreenCaptureKit requires a display in the content filter even for audio-only
/// capture. No `Screen` output is registered, so no frames are ever produced —
/// this is just the smallest legal size.
const MIN_DIMENSION: usize = 2;

/// Captures system audio and microphone as two independent tracks.
///
/// Both tracks come from a single `SCStream`, which is the point: they share one
/// clock, so presentation timestamps are comparable across tracks without
/// correcting for drift between separate capture APIs.
pub struct DualCapture {
    stream: arc::R<sc::Stream>,
    /// ScreenCaptureKit does not retain the output, so the delegate and its queue
    /// are held here — dropping either mid-capture would leave the stream calling
    /// into freed memory.
    _delegate: arc::R<CaptureDelegate>,
    _queue: arc::R<dispatch::Queue>,
}

impl DualCapture {
    /// Starts capturing. Frames are pushed to `tx` from ScreenCaptureKit's dispatch
    /// queue, so the consumer runs on its own thread and never blocks capture.
    ///
    /// Requires Screen Recording permission (for system audio) and Microphone
    /// permission. Both prompt on first use and are attributed to the enclosing app
    /// bundle — for a bare binary, that is the terminal that launched it.
    pub async fn start(tx: Sender<AudioFrame>) -> Result<Self> {
        let content = sc::ShareableContent::current()
            .await
            .map_err(|err| AudioError::ScreenCaptureKit(format!("{err:?}")))?;

        let displays = content.displays();
        let display = displays.get(0).ok().ok_or(AudioError::NoDisplay)?;

        let mut cfg = sc::StreamCfg::new();
        cfg.set_width(MIN_DIMENSION);
        cfg.set_height(MIN_DIMENSION);
        cfg.set_captures_audio(true);
        cfg.set_capture_mic(true);
        // Without this, anything this process plays would be folded back into the
        // system track and re-recorded.
        cfg.set_excludes_current_process_audio(true);
        cfg.set_sample_rate(SAMPLE_RATE);
        cfg.set_channel_count(CHANNEL_COUNT);

        let windows = ns::Array::new();
        let filter = sc::ContentFilter::with_display_excluding_windows(&display, &windows);
        let stream = sc::Stream::new(&filter, &cfg);

        // Serial queue: the two tracks' callbacks are delivered one at a time, so
        // the delegate's &mut self access stays sound without a lock.
        let queue = dispatch::Queue::serial_with_ar_pool();
        let delegate = CaptureDelegate::with(DelegateInner {
            tx,
            scratch: BufListScratch::new(),
        });

        for kind in [sc::OutputType::Audio, sc::OutputType::Mic] {
            stream
                .add_stream_output(delegate.as_ref(), kind, Some(&queue))
                .map_err(|err| AudioError::ScreenCaptureKit(format!("{kind:?}: {err:?}")))?;
        }

        stream
            .start()
            .await
            .map_err(|err| AudioError::ScreenCaptureKit(format!("{err:?}")))?;

        Ok(Self {
            stream,
            _delegate: delegate,
            _queue: queue,
        })
    }

    /// Stops the stream. Frames already queued stay readable on the receiver.
    pub async fn stop(self) -> Result<()> {
        self.stream
            .stop()
            .await
            .map_err(|err| AudioError::ScreenCaptureKit(format!("{err:?}")))
    }
}

/// Rust state carried by the Objective-C delegate object.
#[repr(C)]
struct DelegateInner {
    tx: Sender<AudioFrame>,
    scratch: BufListScratch,
}

impl DelegateInner {
    fn handle(&mut self, sample: &mut cm::SampleBuf, kind: sc::OutputType) {
        let Some(track) = Track::from_output_type(kind) else {
            return;
        };

        match frame_from_sample_buffer(track, sample, &mut self.scratch) {
            Ok(frame) => {
                // A disconnected receiver means the consumer shut down first and the
                // stream is about to be stopped; dropping the frame is correct.
                let _ = self.tx.send(frame);
            }
            Err(error) => eprintln!("[{}] dropped buffer: {error}", track.as_str()),
        }
    }
}

define_obj_type!(
    CaptureDelegate + OutputImpl,
    DelegateInner,
    CAPTURE_DELEGATE
);

impl Output for CaptureDelegate {}

#[objc::add_methods]
impl OutputImpl for CaptureDelegate {
    extern "C" fn impl_stream_did_output_sample_buf(
        &mut self,
        _cmd: Option<&objc::Sel>,
        _stream: &sc::Stream,
        sample_buf: &mut cm::SampleBuf,
        kind: sc::OutputType,
    ) {
        self.inner_mut().handle(sample_buf, kind);
    }
}
