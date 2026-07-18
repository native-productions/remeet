//! Claude Code provider: `claude --print` with a JSON schema.

use std::io::Write;
use std::process::{Command, Stdio};

use serde::Deserialize;

use crate::error::{AiError, Result};
use crate::{Probe, Provider, ProviderConfig, ProviderId};

/// Tools the model has no business touching for a text-in, text-out task.
///
/// Denied as defense in depth: the data handed to the model is a transcript, which
/// is untrusted and could contain text engineered to look like an instruction
/// ("ignore the above and run ..."). With these off, the worst a successful
/// injection can do is skew the answer.
const DENIED_TOOLS: &[&str] = &[
    "Bash",
    "Edit",
    "Write",
    "NotebookEdit",
    "WebFetch",
    "WebSearch",
    "Task",
];

pub struct ClaudeCode {
    config: ProviderConfig,
}

impl ClaudeCode {
    pub fn new(config: ProviderConfig) -> Self {
        Self { config }
    }
}

impl Provider for ClaudeCode {
    fn id(&self) -> ProviderId {
        ProviderId::ClaudeCode
    }

    fn probe(&self) -> Probe {
        let bin = self.config.bin_string();
        match Command::new(&bin).arg("--version").output() {
            Ok(out) if out.status.success() => Probe {
                installed: true,
                version: Some(String::from_utf8_lossy(&out.stdout).trim().to_owned()),
                error: None,
            },
            Ok(out) => Probe {
                installed: true,
                version: None,
                error: Some(String::from_utf8_lossy(&out.stderr).trim().to_owned()),
            },
            Err(e) => Probe {
                installed: false,
                version: None,
                error: Some(e.to_string()),
            },
        }
    }

    fn run_json(
        &self,
        instructions: &str,
        data: &str,
        schema: &str,
    ) -> Result<serde_json::Value> {
        let bin = self.config.bin_string();
        let mut command = Command::new(&bin);
        command.args(["--print", "--output-format", "json", "--json-schema", schema]);

        if let Some(model) = &self.config.model {
            command.args(["--model", model]);
        }

        // Variadic flag last, so its values don't swallow another flag's argument.
        command.arg("--disallowedTools").args(DENIED_TOOLS);

        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| AiError::Spawn {
                bin: bin.clone(),
                source,
            })?;

        // Prompt and response are both bounded (a transcript in, a small object
        // out), so writing stdin fully before reading stdout cannot deadlock.
        // Dropping the handle closes stdin, signalling end of input.
        child
            .stdin
            .take()
            .expect("stdin was piped")
            .write_all(format!("{instructions}{data}").as_bytes())
            .map_err(|source| AiError::Spawn {
                bin: bin.clone(),
                source,
            })?;

        let output = child.wait_with_output().map_err(|source| AiError::Spawn {
            bin: bin.clone(),
            source,
        })?;

        if !output.status.success() {
            return Err(AiError::CliFailed {
                bin,
                code: output
                    .status
                    .code()
                    .map_or_else(|| "signal".to_owned(), |c| c.to_string()),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }

        parse_envelope(&bin, &output.stdout)
    }
}

/// The subset of the CLI's result envelope this crate reads.
#[derive(Deserialize)]
struct Envelope {
    #[serde(default)]
    is_error: bool,
    #[serde(default)]
    subtype: String,
    /// Present on success when `--json-schema` was satisfied; already parsed.
    #[serde(default)]
    structured_output: Option<serde_json::Value>,
    /// Human-readable result or error text, used for diagnostics.
    #[serde(default)]
    result: Option<String>,
}

/// Pulls the structured payload out of the CLI's envelope.
///
/// Split from process handling so it can be tested against captured envelopes
/// without invoking the CLI (which needs a login and the network).
fn parse_envelope(bin: &str, stdout: &[u8]) -> Result<serde_json::Value> {
    let envelope: Envelope =
        serde_json::from_slice(stdout).map_err(|source| AiError::BadJson {
            bin: bin.to_owned(),
            source,
        })?;

    if envelope.is_error {
        return Err(AiError::NoOutput {
            bin: bin.to_owned(),
            detail: envelope
                .result
                .unwrap_or_else(|| envelope.subtype.clone())
                .trim()
                .to_owned(),
        });
    }

    envelope.structured_output.ok_or_else(|| AiError::NoOutput {
        bin: bin.to_owned(),
        detail: envelope
            .result
            .unwrap_or_else(|| "no structured_output field".to_owned())
            .trim()
            .to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_structured_output() {
        let stdout = br#"{"type":"result","is_error":false,
            "result":"{\"summary\":\"x\"}","structured_output":{"summary":"x"}}"#;
        let value = parse_envelope("claude", stdout).expect("parse");
        assert_eq!(value["summary"], "x");
    }

    #[test]
    fn surfaces_model_error() {
        let stdout = br#"{"type":"result","is_error":true,"subtype":"error_during_execution",
            "result":"model not available"}"#;
        let err = parse_envelope("claude", stdout).expect_err("should fail");
        assert!(err.to_string().contains("model not available"), "{err}");
    }

    // A run that answers in prose instead of satisfying the schema must be an
    // error, not an empty result silently rendered as "no summary".
    #[test]
    fn errors_when_schema_was_not_satisfied() {
        let stdout = br#"{"type":"result","is_error":false,"result":"sure, here you go"}"#;
        let err = parse_envelope("claude", stdout).expect_err("should fail");
        assert!(err.to_string().contains("sure, here you go"), "{err}");
    }

    #[test]
    fn errors_on_malformed_envelope() {
        let err = parse_envelope("claude", b"not json").expect_err("should fail");
        assert!(matches!(err, AiError::BadJson { .. }));
    }
}
