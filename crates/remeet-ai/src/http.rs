//! Shared plumbing for the key-based HTTP providers.
//!
//! Unlike the CLIs, these models cannot be handed a schema flag and made to return
//! validated JSON. The portable move across OpenAI, Gemini, and the many
//! OpenAI-compatible local servers is the same everywhere: ask for JSON in the
//! prompt, turn on the endpoint's JSON mode, then parse the text that comes back.
//! Endpoint-specific schema fields (`response_schema`, strict `json_schema`) are
//! deliberately avoided — support for them is uneven and rejects the app's schemas
//! in subtle ways, whereas JSON-mode-plus-prompt works on all of them.

use std::time::Duration;

use crate::error::{AiError, Result};

/// A blocking client with a ceiling long enough for a full transcript but short
/// enough that an unreachable local server fails instead of hanging the summary.
pub(crate) fn client(provider: &str) -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|source| AiError::Http {
            provider: provider.to_owned(),
            source,
        })
}

/// Folds the task instructions, the required shape, and the untrusted data into one
/// prompt. The literal word "JSON" has to be present: OpenAI's JSON mode rejects a
/// request whose messages never mention it.
pub(crate) fn json_prompt(instructions: &str, data: &str, schema: &str) -> String {
    format!(
        "{instructions}\nRespond with a single JSON object and nothing else — no \
prose, no markdown, no code fences. The object must satisfy this JSON Schema:\n\
{schema}\n\n{data}"
    )
}

/// Pulls the JSON object out of a model's text answer.
///
/// JSON mode is supposed to guarantee a bare object, but some local servers still
/// wrap it in ```json fences or a line of prose. Slicing from the first `{` to the
/// last `}` recovers the object in those cases without a second round trip.
pub(crate) fn parse_json(provider: &str, content: &str) -> Result<serde_json::Value> {
    let trimmed = content.trim();
    let slice = match (trimmed.find('{'), trimmed.rfind('}')) {
        (Some(start), Some(end)) if end > start => &trimmed[start..=end],
        _ => trimmed,
    };
    serde_json::from_str(slice).map_err(|source| AiError::BadJson {
        bin: provider.to_owned(),
        source,
    })
}

/// Caps an error body so a provider that answers with an HTML page or a stack trace
/// does not flood the UI. Keeps the head, where the message usually is.
pub(crate) fn clip_body(body: &str) -> String {
    const MAX: usize = 800;
    let trimmed = body.trim();
    match trimmed.char_indices().nth(MAX) {
        Some((byte, _)) => format!("{}…", &trimmed[..byte]),
        None => trimmed.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_bare_object() {
        let value = parse_json("x", r#"{"reply":"OK"}"#).expect("parse");
        assert_eq!(value["reply"], "OK");
    }

    #[test]
    fn recovers_a_fenced_object() {
        let raw = "```json\n{\"reply\":\"OK\"}\n```";
        let value = parse_json("x", raw).expect("parse");
        assert_eq!(value["reply"], "OK");
    }

    #[test]
    fn recovers_object_after_prose() {
        let raw = "Sure, here you go:\n{\"reply\":\"OK\"}";
        let value = parse_json("x", raw).expect("parse");
        assert_eq!(value["reply"], "OK");
    }

    #[test]
    fn errors_on_non_json() {
        assert!(matches!(
            parse_json("x", "no json here"),
            Err(AiError::BadJson { .. })
        ));
    }
}
