//! User settings, stored as JSON under the app's config directory.
//!
//! A file rather than a database row: settings are a handful of values read at the
//! start of an operation and edited by hand about as often. When SQLite lands it
//! will hold recordings and projects — things with history and relations — not this.
//!
//! Unreadable or corrupt settings fall back to defaults instead of failing the app:
//! losing a model preference is recoverable, refusing to open is not.

use std::path::{Path, PathBuf};

use remeet_ai::{ProviderConfig, ProviderId};
use serde::{Deserialize, Serialize};

const FILE: &str = "settings.json";

/// How transcription trades speed against accuracy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum TranscribeSpeed {
    /// Beam search on the full model: the slow, accurate default.
    #[default]
    Accurate,
    /// Greedy decoding: several times faster, with a real accuracy cost — for when a
    /// rough transcript now beats an exact one later.
    Fast,
}

impl TranscribeSpeed {
    /// The beam width this mode decodes with. 1 selects greedy sampling.
    pub fn beam_size(self) -> usize {
        match self {
            Self::Accurate => 5,
            Self::Fast => 1,
        }
    }
}

/// Which engine transcribes the audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum TranscribeEngine {
    /// whisper.cpp, built in — offline, no external dependency, Metal-accelerated.
    #[default]
    Builtin,
    /// The OpenAI `whisper` command-line tool, run on the mixdown. Its decoding
    /// rejects the silence hallucinations the built-in engine emits, at the cost of an
    /// external install and no per-speaker attribution.
    WhisperCli,
}

/// Where to find the external `whisper` tool and which model to run.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WhisperCliConfig {
    /// The `whisper` executable. A bare name is looked up on `PATH`; set a full path
    /// when it lives in a virtualenv the app's environment cannot see.
    pub bin: String,
    /// Model name passed to `--model` (e.g. `turbo`, `large-v3`).
    pub model: String,
}

impl Default for WhisperCliConfig {
    fn default() -> Self {
        Self {
            bin: "whisper".to_owned(),
            model: "turbo".to_owned(),
        }
    }
}

/// Which model the built-in whisper.cpp engine loads.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WhisperBuiltinConfig {
    /// GGML model name, resolved to `~/whisper/models/ggml-<model>.bin` (e.g.
    /// `large-v3`, `large-v3-turbo`). `REMEET_MODEL` overrides the whole path.
    pub model: String,
}

impl Default for WhisperBuiltinConfig {
    fn default() -> Self {
        Self {
            model: "large-v3".to_owned(),
        }
    }
}

/// Everything the user can configure.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Which CLI handles language work (summaries today, more later).
    pub provider: ProviderId,
    /// Per-provider binary path and model, kept separately so switching providers
    /// does not discard the other one's setup.
    pub claude_code: ProviderConfig,
    pub codex: ProviderConfig,
    /// Space the next recording is filed into, or `None` for the default space.
    ///
    /// Sticky on purpose: someone recording a day of calls for one client sets it
    /// once, not before every call.
    pub active_space: Option<String>,
    /// Whether to notify when another app puts a call on the mic and speakers, in
    /// case the user forgot to record it. On by default; the whole point of the app
    /// is not missing meetings.
    pub call_reminder: bool,
    /// Speed/accuracy trade-off for transcription. Accurate by default.
    pub transcribe_speed: TranscribeSpeed,
    /// Force a transcription language as an ISO code (`"id"`, `"en"`), or `None`/empty
    /// to auto-detect. Set it when auto-detect guesses wrong — common for Indonesian
    /// meetings that mix in English.
    pub transcribe_language: Option<String>,
    /// Suppress background noise on the microphone before transcribing. Off by
    /// default — it can clip a quiet voice, so it is opt-in for noisy places.
    pub mic_denoise: bool,
    /// Which engine transcribes. Built-in by default.
    pub transcribe_engine: TranscribeEngine,
    /// External `whisper` tool location and model, used when the engine is the CLI.
    pub whisper_cli: WhisperCliConfig,
    /// Which GGML model the built-in engine loads.
    pub whisper_builtin: WhisperBuiltinConfig,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            provider: ProviderId::ClaudeCode,
            claude_code: ProviderConfig::new(ProviderId::ClaudeCode),
            codex: ProviderConfig::new(ProviderId::Codex),
            active_space: None,
            call_reminder: true,
            transcribe_speed: TranscribeSpeed::default(),
            transcribe_language: None,
            mic_denoise: false,
            transcribe_engine: TranscribeEngine::default(),
            whisper_cli: WhisperCliConfig::default(),
            whisper_builtin: WhisperBuiltinConfig::default(),
        }
    }
}

impl Settings {
    /// The configuration for one provider, with its id forced to match the slot it
    /// was stored in — a hand-edited file cannot make `codex` mean `claude`.
    pub fn config_for(&self, id: ProviderId) -> ProviderConfig {
        let mut config = match id {
            ProviderId::ClaudeCode => self.claude_code.clone(),
            ProviderId::Codex => self.codex.clone(),
        };
        config.id = id;
        config
    }

    /// The configuration for the currently selected provider.
    pub fn active(&self) -> ProviderConfig {
        self.config_for(self.provider)
    }
}

pub fn load(dir: &Path) -> Settings {
    std::fs::read_to_string(dir.join(FILE))
        .ok()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

pub fn save(dir: &Path, settings: &Settings) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let json = serde_json::to_string_pretty(settings)
        .unwrap_or_else(|_| "{}".to_owned());
    std::fs::write(dir.join(FILE), json)
}

/// Where settings live, given the app's config directory.
pub fn path(dir: &Path) -> PathBuf {
    dir.join(FILE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_yields_defaults() {
        let dir = tempfile::tempdir().expect("tempdir");
        let settings = load(dir.path());
        assert_eq!(settings.provider, ProviderId::ClaudeCode);
    }

    #[test]
    fn corrupt_file_yields_defaults_rather_than_failing() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(path(dir.path()), "{ this is not json").expect("write");
        assert_eq!(load(dir.path()).provider, ProviderId::ClaudeCode);
    }

    #[test]
    fn round_trips() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut settings = Settings::default();
        settings.provider = ProviderId::Codex;
        settings.codex.model = Some("gpt-5.5".to_owned());
        save(dir.path(), &settings).expect("save");

        let loaded = load(dir.path());
        assert_eq!(loaded.provider, ProviderId::Codex);
        assert_eq!(loaded.active().model.as_deref(), Some("gpt-5.5"));
    }

    // Both slots keep their own settings, so flipping the provider back and forth
    // does not quietly erase the other one's model.
    #[test]
    fn config_for_forces_the_id_to_match_its_slot() {
        let mut settings = Settings::default();
        settings.claude_code.id = ProviderId::Codex;
        assert_eq!(
            settings.config_for(ProviderId::ClaudeCode).id,
            ProviderId::ClaudeCode
        );
    }
}
