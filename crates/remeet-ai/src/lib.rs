//! Local AI providers for Remeet.
//!
//! Remeet does its language work by driving an AI CLI that is already installed and
//! logged in on the machine — Claude Code or Codex. No API key, no second
//! subscription, and nothing leaves the machine except the transcript text the user
//! asked to process.
//!
//! Both CLIs can be made to return schema-validated JSON, which is the whole reason
//! they are usable as a backend: the result is parsed, not scraped out of prose.
//! They differ in almost every mechanical detail, and [`Provider`] is where that
//! difference is absorbed:
//!
//! |                | Claude Code                    | Codex                            |
//! |----------------|--------------------------------|----------------------------------|
//! | Invocation     | `claude --print`               | `codex exec`                     |
//! | Schema         | inline `--json-schema`         | a file, `--output-schema`        |
//! | Result         | `structured_output` on stdout  | a file, `-o`                     |
//! | Tool limits    | `--disallowedTools`            | `--sandbox read-only`            |
//!
//! ## Cost shape
//!
//! Each invocation re-pays the CLI's own startup context — measured at ~47k input
//! tokens for Claude Code and ~18k for Codex on a two-token prompt. Call these once
//! per meeting over the whole transcript; never per line or per question.
//!
//! ## Transcripts are untrusted
//!
//! A transcript contains whatever was said on the call, which may include text
//! engineered to read as an instruction. Every provider here runs with tool access
//! restricted as far as its CLI allows, and prompts state that the transcript is
//! data. The restriction is not equally strong on both — see [`codex`].

mod claude;
mod codex;
mod error;
mod gemini;
mod http;
mod openai;
mod summary;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub use claude::ClaudeCode;
pub use codex::Codex;
pub use error::{AiError, Result};
pub use gemini::Gemini;
pub use openai::OpenAiCompatible;
pub use summary::{Summary, summarize};

/// Which backend does the language work.
///
/// Two families sit behind one enum: CLI providers that drive a logged-in tool on
/// the machine ([`ClaudeCode`](Self::ClaudeCode), [`Codex`](Self::Codex)), and
/// key-based providers that call an HTTP API ([`Gemini`](Self::Gemini),
/// [`Openai`](Self::Openai), and [`Custom`](Self::Custom) for any OpenAI-compatible
/// local server). They share the [`Provider`] contract; only their setup differs,
/// which is why [`is_api`](Self::is_api) exists — the UI shows a key field for one
/// family and a binary path for the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderId {
    ClaudeCode,
    Codex,
    Gemini,
    Openai,
    Custom,
}

impl ProviderId {
    /// The binary name looked up on `PATH` when no explicit path is configured.
    ///
    /// Only meaningful for the CLI family; for the key-based providers it is just a
    /// diagnostic label, never spawned.
    pub fn default_bin(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude",
            Self::Codex => "codex",
            Self::Gemini => "gemini",
            Self::Openai => "openai",
            Self::Custom => "custom",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::Codex => "Codex",
            Self::Gemini => "Gemini",
            Self::Openai => "OpenAI",
            Self::Custom => "Custom (OpenAI-compatible)",
        }
    }

    /// Whether this provider talks to an HTTP API with a key, rather than driving a
    /// local CLI.
    pub fn is_api(self) -> bool {
        matches!(self, Self::Gemini | Self::Openai | Self::Custom)
    }
}

/// How to reach one provider.
///
/// A single struct spans both families: the CLI providers read `bin`/`model`, the
/// key-based ones read `api_key`/`base_url`/`model`. The unused fields stay `None`
/// so switching families never discards the other's setup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub id: ProviderId,
    /// Path to the binary, or `None` to use the name on `PATH`. CLI providers only.
    #[serde(default)]
    pub bin: Option<PathBuf>,
    /// Model to request, or `None` to let the provider use its own default.
    ///
    /// Deliberately free text rather than a fixed list: which models an account may
    /// use is decided by the account behind the CLI or key, not by this app. A
    /// hardcoded menu would go stale and would lie about what is available. Required
    /// for the key-based providers, which have no server-side default.
    #[serde(default)]
    pub model: Option<String>,
    /// API key for a key-based provider, `None` for the CLI providers (and for a
    /// local server that needs no auth). Stored in plain text alongside the rest of
    /// settings — the same trust level as the config directory itself.
    #[serde(default)]
    pub api_key: Option<String>,
    /// Base URL override. Required for [`ProviderId::Custom`] (where the server
    /// lives); optional for the others, which have a default endpoint.
    #[serde(default)]
    pub base_url: Option<String>,
}

impl ProviderConfig {
    pub fn new(id: ProviderId) -> Self {
        Self {
            id,
            bin: None,
            model: None,
            api_key: None,
            base_url: None,
        }
    }

    fn bin_string(&self) -> String {
        self.bin
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| self.id.default_bin().to_owned())
    }
}

/// What a probe found out about an installed CLI.
#[derive(Debug, Clone, Serialize)]
pub struct Probe {
    pub installed: bool,
    /// Whatever `--version` printed, when it could be run.
    pub version: Option<String>,
    /// Why the probe failed, in the CLI's own words where possible.
    pub error: Option<String>,
}

/// A local AI CLI that can be asked for schema-validated JSON.
pub trait Provider {
    fn id(&self) -> ProviderId;

    /// Checks the binary exists and can run, without spending any tokens.
    ///
    /// This deliberately does not prove the CLI is logged in — that needs a real
    /// request. A round trip through [`run_json`](Self::run_json) is the only honest
    /// test of that, which is why the UI offers one separately.
    fn probe(&self) -> Probe;

    /// Runs `instructions` over `data`, returning JSON matching `schema`.
    ///
    /// `data` is untrusted; implementations must not give the model a way to act on
    /// it beyond producing text.
    fn run_json(
        &self,
        instructions: &str,
        data: &str,
        schema: &str,
    ) -> Result<serde_json::Value>;
}

/// Builds the provider for a configuration.
pub fn provider(config: ProviderConfig) -> Box<dyn Provider> {
    match config.id {
        ProviderId::ClaudeCode => Box::new(ClaudeCode::new(config)),
        ProviderId::Codex => Box::new(Codex::new(config)),
        ProviderId::Gemini => Box::new(Gemini::new(config)),
        // OpenAI and any OpenAI-compatible local server share one implementation.
        ProviderId::Openai | ProviderId::Custom => Box::new(OpenAiCompatible::new(config)),
    }
}
