use thiserror::Error;

#[derive(Debug, Error)]
pub enum SessionError {
    #[error("capture: {0}")]
    Audio(#[from] remeet_audio::AudioError),

    #[error("transcription: {0}")]
    Transcribe(#[from] remeet_transcribe::TranscribeError),

    #[error("reading {path}: {source}")]
    WavRead { path: String, source: hound::Error },

    #[error("writing wav: {0}")]
    WavWrite(#[from] hound::Error),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("the recording thread panicked")]
    CollectorPanicked,

    #[error("no audio was captured on any track")]
    NothingCaptured,
}

pub type Result<T> = std::result::Result<T, SessionError>;
