use thiserror::Error;

#[derive(Debug, Error)]
pub enum TodoError {
    #[error("could not run the Claude CLI ({bin}): {source}")]
    Spawn { bin: String, source: std::io::Error },

    /// The CLI ran but exited non-zero — usually not logged in, or a bad flag.
    #[error("Claude CLI exited with {code}: {stderr}")]
    CliFailed { code: String, stderr: String },

    /// The CLI reported a turn-level error (e.g. it hit a guardrail or timed out).
    #[error("Claude reported an error: {0}")]
    ModelError(String),

    #[error("could not parse the CLI's JSON envelope: {0}")]
    Envelope(#[source] serde_json::Error),

    /// The envelope parsed but held no structured output — the model returned prose
    /// instead of satisfying the schema.
    #[error("response contained no structured output")]
    NoStructuredOutput,

    #[error("could not parse extracted todos: {0}")]
    Todos(#[source] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, TodoError>;
