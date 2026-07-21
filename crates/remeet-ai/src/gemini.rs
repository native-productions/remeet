//! Google Gemini provider: `POST .../models/{model}:generateContent`.
//!
//! `response_mime_type: application/json` turns on JSON mode; the schema rides in
//! the prompt like the other HTTP providers (see [`crate::http`]). Gemini's own
//! `response_schema` is skipped on purpose — it is an OpenAPI subset that rejects
//! `additionalProperties`, which the app's schemas set.

use serde_json::json;

use crate::error::{AiError, Result};
use crate::http;
use crate::{Probe, Provider, ProviderConfig, ProviderId};

const GEMINI_BASE: &str = "https://generativelanguage.googleapis.com/v1beta";

pub struct Gemini {
    config: ProviderConfig,
}

impl Gemini {
    pub fn new(config: ProviderConfig) -> Self {
        Self { config }
    }

    fn base_url(&self) -> String {
        match self.config.base_url.as_deref() {
            Some(url) if !url.trim().is_empty() => url.trim().trim_end_matches('/').to_owned(),
            _ => GEMINI_BASE.to_owned(),
        }
    }
}

impl Provider for Gemini {
    fn id(&self) -> ProviderId {
        ProviderId::Gemini
    }

    fn probe(&self) -> Probe {
        match key(&self.config) {
            Some(_) => Probe {
                installed: true,
                version: Some("configured".into()),
                error: None,
            },
            None => Probe {
                installed: false,
                version: None,
                error: Some("no API key set".into()),
            },
        }
    }

    fn run_json(
        &self,
        instructions: &str,
        data: &str,
        schema: &str,
    ) -> Result<serde_json::Value> {
        let label = ProviderId::Gemini.label().to_owned();
        let key = key(&self.config).ok_or_else(|| AiError::MissingConfig {
            provider: label.clone(),
            detail: "set an API key".into(),
        })?;
        let model = self
            .config
            .model
            .as_deref()
            .filter(|m| !m.is_empty())
            .ok_or_else(|| AiError::MissingConfig {
                provider: label.clone(),
                detail: "set a model (e.g. gemini-2.5-flash)".into(),
            })?;

        let body = json!({
            "contents": [{
                "parts": [{ "text": http::json_prompt(instructions, data, schema) }],
            }],
            "generationConfig": {
                "response_mime_type": "application/json",
                "temperature": 0,
            },
        });

        let url = format!("{}/models/{model}:generateContent", self.base_url());
        // Key in a header, not the query string, so it does not land in request logs.
        let response = http::client(&label)?
            .post(&url)
            .header("x-goog-api-key", key)
            .json(&body)
            .send()
            .map_err(|source| AiError::Http {
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
        let content = root["candidates"][0]["content"]["parts"][0]["text"]
            .as_str()
            .ok_or_else(|| AiError::NoOutput {
                bin: label.clone(),
                // A blocked prompt comes back with no text but a reason worth showing.
                detail: root["candidates"][0]["finishReason"]
                    .as_str()
                    .map(|r| format!("no text returned (finishReason: {r})"))
                    .unwrap_or_else(|| "response had no text".into()),
            })?;

        http::parse_json(&label, content)
    }
}

fn key(config: &ProviderConfig) -> Option<&str> {
    config.api_key.as_deref().filter(|k| !k.trim().is_empty())
}
