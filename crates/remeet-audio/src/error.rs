use thiserror::Error;

#[derive(Debug, Error)]
pub enum AudioError {
    #[error("no display available to attach a capture stream to")]
    NoDisplay,

    /// Screen Recording or Microphone permission denied is the common cause here,
    /// and ScreenCaptureKit reports it as an ordinary stream error.
    #[error("screencapturekit: {0}")]
    ScreenCaptureKit(String),

    #[error("sample buffer carried no audio data")]
    NoAudioData,

    #[error("reading audio buffer list: {0}")]
    AudioBufferList(String),

    #[error("channel plane lengths differ: {0:?}")]
    RaggedPlanes(Vec<usize>),

    /// Voice-processing microphone capture (AVAudioEngine VPIO) could not start —
    /// usually Microphone permission denied, or no input device available.
    #[error("voice-processing mic: {0}")]
    Vpio(String),

    #[error("wav: {0}")]
    Wav(#[from] hound::Error),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, AudioError>;
