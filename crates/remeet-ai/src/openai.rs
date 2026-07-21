//! OpenAI and OpenAI-compatible provider: one `POST /chat/completions`.
//!
//! The same code serves hosted OpenAI and any local server that speaks its API
//! (Ollama, LM Studio, vLLM, llama.cpp). They differ only in the base URL and
//! whether a key is required, both of which are configuration — so [`ProviderId::Openai`]
//! and [`ProviderId::Custom`] share this one implementation.

use serde_json::json;

use crate::error::{AiError, Result};
use crate::http;
use crate::{Probe, Provider, ProviderConfig, ProviderId};

const OPENAI_BASE: &str = "https://api.openai.com/v1";

pub struct OpenAiCompatible {
    config: ProviderConfig,
}

impl OpenAiCompatible {
    pub fn new(config: ProviderConfig) -> Self {
        Self { config }
    }

    /// Hosted OpenAI has a fixed endpoint; a custom server must say where it lives,
    /// since there is no sensible default for "your local model".
    fn base_url(&self) -> Result<String> {
        match (self.config.id, self.config.base_url.as_deref()) {
            (_, Some(url)) if !url.trim().is_empty() => {
                Ok(url.trim().trim_end_matches('/').to_owned())
            }
            (ProviderId::Openai, _) => Ok(OPENAI_BASE.to_owned()),
            (ProviderId::Custom, _) => Err(AiError::MissingConfig {
                provider: self.label(),
                detail: "set the server's base URL (e.g. http://localhost:11434/v1)".into(),
            }),
            _ => Ok(OPENAI_BASE.to_owned()),
        }
    }

    fn label(&self) -> String {
        self.config.id.label().to_owned()
    }
}

impl Provider for OpenAiCompatible {
    fn id(&self) -> ProviderId {
        self.config.id
    }

    fn probe(&self) -> Probe {
        // No binary and no free round trip: a probe can only confirm the request is
        // reachable on paper. A local server needs a base URL; hosted OpenAI needs a
        // key. Being logged in is still only provable by the paid test.
        match (self.config.id, self.base_url()) {
            (_, Err(e)) => Probe {
                installed: false,
                version: None,
                error: Some(e.to_string()),
            },
            (ProviderId::Openai, _) if key(&self.config).is_none() => Probe {
                installed: false,
                version: None,
                error: Some("no API key set".into()),
            },
            (_, Ok(url)) => Probe {
                installed: true,
                version: Some(format!("configured — {url}")),
                error: None,
            },
        }
    }

    fn run_json(
        &self,
        instructions: &str,
        data: &str,
        schema: &str,
    ) -> Result<serde_json::Value> {
        let label = self.label();
        let base = self.base_url()?;
        let model = self.config.model.as_deref().filter(|m| !m.is_empty()).ok_or_else(|| {
            AiError::MissingConfig {
                provider: label.clone(),
                detail: "set a model".into(),
            }
        })?;

        let body = json!({
            "model": model,
            "messages": [{ "role": "user", "content": http::json_prompt(instructions, data, schema) }],
            "response_format": { "type": "json_object" },
            "temperature": 0,
        });

        let url = format!("{base}/chat/completions");
        let mut request = http::client(&label)?.post(&url).json(&body);
        // A local server may need no key; hosted OpenAI always does.
        if let Some(key) = key(&self.config) {
            request = request.bearer_auth(key);
        }

        let response = request.send().map_err(|source| AiError::Http {
            provider: label.clone(),
            source,
        })?;
        let status = response.status();
        let text = response.text().map_err(|source| AiError::Http {
            provider: label.clone(),
            source,
        })?;

        if !status.is_success() {
            return Err(AiError::Api {
                provider: label,
                status: status.as_u16(),
                body: http::clip_body(&text),
            });
        }

        let root: serde_json::Value =
            serde_json::from_str(&text).map_err(|source| AiError::BadJson {
                bin: label.clone(),
                source,
            })?;
        let content = root["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| AiError::NoOutput {
                bin: label.clone(),
                detail: "response had no message content".into(),
            })?;

        http::parse_json(&label, content)
    }
}

/// The key, treated as unset when blank so a stray empty string does not send an
/// `Authorization: Bearer` header a local server would reject.
fn key(config: &ProviderConfig) -> Option<&str> {
    config.api_key.as_deref().filter(|k| !k.trim().is_empty())
}
