use thiserror::Error;

#[derive(Debug, Error)]
pub enum AiError {
    #[error("could not run {bin}: {source}")]
    Spawn {
        bin: String,
        #[source]
        source: std::io::Error,
    },

    /// The CLI ran but reported failure. `stderr` carries its own words, which is
    /// usually the only thing that explains a login or model problem.
    #[error("{bin} exited with {code}: {stderr}")]
    CliFailed {
        bin: String,
        code: String,
        stderr: String,
    },

    #[error("{bin} returned no structured output: {detail}")]
    NoOutput { bin: String, detail: String },

    #[error("could not parse {bin} output: {source}")]
    BadJson {
        bin: String,
        #[source]
        source: serde_json::Error,
    },

    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, AiError>;
