use thiserror::Error;

#[derive(Debug, Error)]
pub enum TranscribeError {
    #[error("loading model at {path}: {source}")]
    ModelLoad {
        path: String,
        source: whisper_rs::WhisperError,
    },

    #[error("whisper: {0}")]
    Whisper(#[from] whisper_rs::WhisperError),

    #[error("building resampler: {0}")]
    ResamplerConstruction(#[from] rubato::ResamplerConstructionError),

    #[error("resampling: {0}")]
    Resample(#[from] rubato::ResampleError),

    #[error("empty audio: nothing to transcribe")]
    EmptyAudio,
}

pub type Result<T> = std::result::Result<T, TranscribeError>;
