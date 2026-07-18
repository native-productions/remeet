//! Codex provider: `codex exec` with a schema file and a result file.
//!
//! ## Isolation is weaker here, and knowingly so
//!
//! Claude Code takes an explicit list of tools to deny. Codex has no equivalent
//! flag, so the model keeps its shell tool and the containment is the sandbox
//! instead. Three things narrow it as far as the CLI allows:
//!
//! - `--sandbox read-only` — model-run commands cannot write or reach the network.
//! - `-C <empty temp dir>` — the working root holds nothing of the user's.
//! - `--ephemeral` — no session files are left behind on disk.
//!
//! Read-only still means readable, so a successful prompt injection could in
//! principle have Codex read files and describe them in its answer. That answer goes
//! nowhere but this app's own UI, which bounds the damage, but it is a real
//! difference from the Claude path and should not be papered over.

use std::io::Write;
use std::process::{Command, Stdio};

use crate::error::{AiError, Result};
use crate::{Probe, Provider, ProviderConfig, ProviderId};

pub struct Codex {
    config: ProviderConfig,
}

impl Codex {
    pub fn new(config: ProviderConfig) -> Self {
        Self { config }
    }
}

impl Provider for Codex {
    fn id(&self) -> ProviderId {
        ProviderId::Codex
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

        // Codex passes both the schema and the answer through files, so the run
        // needs a scratch directory. It doubles as the working root: an empty
        // directory is the least interesting place the model could look.
        let dir = tempfile::tempdir()?;
        let schema_path = dir.path().join("schema.json");
        let output_path = dir.path().join("answer.json");
        let work = dir.path().join("work");
        std::fs::write(&schema_path, schema)?;
        std::fs::create_dir(&work)?;

        let mut command = Command::new(&bin);
        command.arg("exec");
        if let Some(model) = &self.config.model {
            command.args(["--model", model]);
        }
        command
            .args(["--sandbox", "read-only"])
            .arg("--skip-git-repo-check")
            .arg("--ephemeral")
            .arg("-C")
            .arg(&work)
            .arg("--output-schema")
            .arg(&schema_path)
            .arg("-o")
            .arg(&output_path)
            // `-` reads the prompt from stdin rather than the command line, so a
            // long transcript never has to fit in an argument.
            .arg("-");

        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| AiError::Spawn {
                bin: bin.clone(),
                source,
            })?;

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
                // Codex reports model and auth problems on stdout as `ERROR:` lines,
                // not stderr, so both are worth showing.
                stderr: diagnostics(&output.stderr, &output.stdout),
            });
        }

        // The answer file is the contract; stdout is progress chatter around it.
        let answer = std::fs::read_to_string(&output_path).map_err(|_| AiError::NoOutput {
            bin: bin.clone(),
            detail: diagnostics(&output.stderr, &output.stdout),
        })?;

        serde_json::from_str(&answer).map_err(|source| AiError::BadJson { bin, source })
    }
}

/// Last few meaningful lines of the CLI's output, for an error message.
fn diagnostics(stderr: &[u8], stdout: &[u8]) -> String {
    let stderr = String::from_utf8_lossy(stderr);
    let stdout = String::from_utf8_lossy(stdout);

    let mut lines: Vec<&str> = stderr
        .lines()
        .chain(stdout.lines())
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();

    // Errors land at the end of the stream, and the whole session transcript is far
    // too much to put in a toast.
    let tail = lines.split_off(lines.len().saturating_sub(4));
    tail.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_keeps_the_tail() {
        let out = b"line1\nline2\nline3\nline4\nline5\nERROR: nope\n";
        let text = diagnostics(b"", out);
        assert!(text.contains("ERROR: nope"), "{text}");
        assert!(!text.contains("line1"), "{text}");
    }

    #[test]
    fn diagnostics_prefers_stderr_first() {
        let text = diagnostics(b"stderr line\n", b"stdout line\n");
        assert_eq!(text, "stderr line\nstdout line");
    }
}
