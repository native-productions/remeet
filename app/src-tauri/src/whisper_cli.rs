//! Transcription through the external OpenAI `whisper` command-line tool.
//!
//! An alternative to the built-in whisper.cpp engine. It runs on the single mixdown
//! file — so there is no per-speaker attribution — but its decoding rejects the
//! silence hallucinations the built-in engine produces, which is why it is offered.
//!
//! The child is streamed rather than run to completion in one blocking call: whisper
//! prints each segment as it decodes it, and forwarding those live gives the UI a
//! progress feed instead of a bare spinner. The child's PID is handed back so a
//! cancel can signal it — killing the process is the only way to stop a decode already
//! blocked inside the tool. The final, authoritative result still comes from the JSON
//! the tool writes when it finishes.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;

/// One transcribed span from the tool's JSON output.
pub struct Segment {
    pub start_secs: f64,
    pub end_secs: f64,
    pub text: String,
}

/// Runs `whisper` on `wav` and returns its segments.
///
/// `language` is an ISO code to force, or `None` to let the tool detect it. Output is
/// written as JSON next to the input and parsed back for the return value; the tool has
/// no streaming mode, so this blocks until the whole file is done.
///
/// `register_pid` is called once with the child's PID as soon as it spawns, so a
/// cancel can signal it. `on_segment` is called for each segment as the tool prints it,
/// for a live preview — the returned segments come from the JSON, not these lines.
pub fn transcribe<P, S>(
    bin: &str,
    model: &str,
    language: Option<&str>,
    wav: &Path,
    register_pid: P,
    mut on_segment: S,
) -> Result<Vec<Segment>, String>
where
    P: FnOnce(u32),
    S: FnMut(Segment),
{
    let dir = wav.parent().ok_or("recording has no directory")?;
    let stem = wav
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or("bad audio path")?;

    let mut cmd = Command::new(bin);
    cmd.arg(wav)
        .arg("--model")
        .arg(model)
        .arg("--output_format")
        .arg("json")
        .arg("--output_dir")
        .arg(dir)
        // The default half-precision path warns and falls back on CPU/MPS; asking for
        // full precision keeps the run quiet and portable.
        .args(["--fp16", "False"])
        // Python block-buffers stdout when it is a pipe rather than a terminal, so the
        // verbose per-segment lines would arrive only in a burst at the end. Unbuffered
        // makes them stream as they are printed, which is the whole point of reading
        // them live below.
        .env("PYTHONUNBUFFERED", "1")
        .env("PATH", child_path(bin))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(language) = language {
        cmd.arg("--language").arg(language);
    }

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("could not run `{bin}`: {e}. Set the full path in Settings."))?;
    register_pid(child.id());

    // Drain stderr on its own thread so its pipe never backs up and stalls the child;
    // keep the last non-empty line to explain a failure.
    let stderr = child.stderr.take().ok_or("whisper produced no stderr pipe")?;
    let stderr_tail = thread::spawn(move || {
        let mut last = String::new();
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if !line.trim().is_empty() {
                last = line;
            }
        }
        last
    });

    // whisper prints one line per segment as it decodes them (`--verbose`, on by
    // default): "[mm:ss.sss --> mm:ss.sss] text". Forward each as it arrives.
    let stdout = child.stdout.take().ok_or("whisper produced no stdout pipe")?;
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        if let Some(segment) = parse_verbose_line(&line) {
            on_segment(segment);
        }
    }

    let status = child.wait().map_err(|e| format!("whisper wait failed: {e}"))?;
    let stderr_tail = stderr_tail.join().unwrap_or_default();
    if !status.success() {
        let detail = if stderr_tail.is_empty() {
            "process exited with an error".to_owned()
        } else {
            stderr_tail
        };
        return Err(format!("whisper failed: {detail}"));
    }

    // The JSON written alongside the audio is the authoritative result — exact
    // timestamps and full text — so the saved transcript comes from it, not the
    // truncated live lines above.
    let json_path = dir.join(format!("{stem}.json"));
    let json = std::fs::read_to_string(&json_path)
        .map_err(|e| format!("whisper wrote no output: {e}"))?;
    let value: serde_json::Value =
        serde_json::from_str(&json).map_err(|e| format!("unreadable whisper output: {e}"))?;

    let segments = value["segments"].as_array().ok_or("whisper output had no segments")?;
    Ok(segments
        .iter()
        .filter_map(|s| {
            let text = s["text"].as_str()?.trim().to_owned();
            if text.is_empty() {
                return None;
            }
            Some(Segment {
                start_secs: s["start"].as_f64().unwrap_or(0.0),
                end_secs: s["end"].as_f64().unwrap_or(0.0),
                text,
            })
        })
        .collect())
}

/// The `PATH` handed to the whisper child.
///
/// openai-whisper shells out to `ffmpeg` (by bare name) to decode the audio. A GUI
/// app launched from Finder inherits only the minimal launchd `PATH`
/// (`/usr/bin:/bin:/usr/sbin:/sbin`), so a Homebrew `ffmpeg` — the usual install —
/// is invisible and the decode fails. Prepend the common install dirs, plus the
/// whisper binary's own directory (its venv often has a sibling `ffmpeg`), ahead of
/// whatever `PATH` we were given, so the child resolves tools the way a terminal run
/// would. Order matters only for shadowing; prepending is safe.
fn child_path(bin: &str) -> String {
    let mut dirs: Vec<String> = vec!["/opt/homebrew/bin".into(), "/usr/local/bin".into()];

    if let Some(parent) = Path::new(bin).parent()
        && !parent.as_os_str().is_empty()
    {
        dirs.push(parent.display().to_string());
    }

    if let Ok(existing) = std::env::var("PATH")
        && !existing.is_empty()
    {
        dirs.push(existing);
    }

    dirs.join(":")
}

/// Parses one whisper verbose line, `[mm:ss.sss --> mm:ss.sss] text`, into a segment.
/// Returns `None` for the tool's other chatter (language-detection notes, blanks).
fn parse_verbose_line(line: &str) -> Option<Segment> {
    let rest = line.trim().strip_prefix('[')?;
    let (span, text) = rest.split_once(']')?;
    let (start, end) = span.split_once("-->")?;
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Some(Segment {
        start_secs: parse_timestamp(start.trim()),
        end_secs: parse_timestamp(end.trim()),
        text: text.to_owned(),
    })
}

/// Folds a `mm:ss.sss` (or `hh:mm:ss.sss` past an hour) timestamp into seconds.
/// Unparseable parts count as zero, so a malformed line degrades rather than fails.
fn parse_timestamp(s: &str) -> f64 {
    s.split(':')
        .fold(0.0, |acc, part| acc * 60.0 + part.parse::<f64>().unwrap_or(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_verbose_segment_line() {
        let seg = parse_verbose_line("[00:04.120 --> 00:07.800]  oke lanjut ya").unwrap();
        assert!((seg.start_secs - 4.12).abs() < 1e-6);
        assert!((seg.end_secs - 7.8).abs() < 1e-6);
        assert_eq!(seg.text, "oke lanjut ya");
    }

    #[test]
    fn parses_an_hour_long_timestamp() {
        let seg = parse_verbose_line("[01:02:03.500 --> 01:02:05.000] halo").unwrap();
        assert!((seg.start_secs - 3723.5).abs() < 1e-6);
    }

    #[test]
    fn ignores_non_segment_chatter() {
        assert!(parse_verbose_line("Detecting language using up to the first 30 seconds").is_none());
        assert!(parse_verbose_line("").is_none());
        assert!(parse_verbose_line("[00:00.000 --> 00:02.000]   ").is_none());
    }
}
