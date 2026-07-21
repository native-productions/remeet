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

    /// A key-based provider is missing something it needs before it can run at all
    /// — an API key, a base URL, or a model. Caught in the UI as a setup problem
    /// rather than a request failure.
    #[error("{provider}: {detail}")]
    MissingConfig { provider: String, detail: String },

    /// The HTTP request never got an answer: DNS, TLS, timeout, or a refused
    /// connection. Common for a local model server that is not running.
    #[error("could not reach {provider}: {source}")]
    Http {
        provider: String,
        #[source]
        source: reqwest::Error,
    },

    /// The API answered with an error status. `body` carries its own message,
    /// which is usually the only thing that explains a bad key or model.
    #[error("{provider} returned {status}: {body}")]
    Api {
        provider: String,
        status: u16,
        body: String,
    },

    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, AiError>;
