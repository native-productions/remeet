//! Spike: prove that a transcript turns into a correctly attributed todo list.
//!
//! Reads a `[speaker] text` transcript on stdin, sends it to the local Claude CLI
//! via `remeet-todo`, and prints the extracted action items grouped by owner.
//! Success is judged by reading it: real commitments become todos, chatter does
//! not, and each task lands on the right person's list.
//!
//! ```sh
//! cargo run -p todo < sample-transcript.txt
//! ```
//!
//! Throwaway — the reusable half lives in `remeet-todo`.

use std::io::Read;

use anyhow::{Context, Result};
use remeet_todo::{Extractor, Owner, Todo};

fn main() -> Result<()> {
    let mut transcript = String::new();
    std::io::stdin()
        .read_to_string(&mut transcript)
        .context("reading transcript from stdin")?;

    let transcript = transcript.trim();
    anyhow::ensure!(!transcript.is_empty(), "empty transcript on stdin");

    eprintln!("Extracting action items via the Claude CLI...");
    let todos = Extractor::new()
        .extract(transcript)
        .context("extracting todos")?;

    if todos.is_empty() {
        println!("No action items found.");
        return Ok(());
    }

    print_group("Yours", &todos, Owner::Me);
    print_group("Theirs", &todos, Owner::Them);
    print_group("Unassigned", &todos, Owner::Unassigned);

    Ok(())
}

/// Prints every todo for one owner, skipping the heading when there are none.
fn print_group(heading: &str, todos: &[Todo], owner: Owner) {
    let group: Vec<&Todo> = todos.iter().filter(|t| t.owner == owner).collect();
    if group.is_empty() {
        return;
    }

    println!("\n{heading}:");
    for todo in group {
        let due = todo
            .due
            .as_deref()
            .map(|d| format!(" (due: {d})"))
            .unwrap_or_default();
        println!("  - {}{}", todo.task, due);
        println!("      from: \"{}\"", todo.quote);
    }
}
