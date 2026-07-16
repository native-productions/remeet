//! Action-item extraction for Remeet.
//!
//! Turns a meeting transcript into a list of [`Todo`]s by delegating the language
//! understanding to the locally installed Claude CLI (`~/.claude`). This uses the
//! machine's existing Claude Code login — no API key, no separate subscription.
//!
//! The CLI is driven in headless mode (`--print`) with a JSON schema, so it returns
//! validated structured output rather than prose that would need scraping.
//!
//! ```no_run
//! use remeet_todo::Extractor;
//!
//! let transcript = "\
//! [them] can you deploy staging today?
//! [me] yeah I'll take the deploy, you update the docs";
//!
//! let todos = Extractor::new().extract(transcript)?;
//! for todo in &todos {
//!     println!("[{}] {}", todo.owner, todo.task);
//! }
//! # Ok::<(), remeet_todo::TodoError>(())
//! ```

mod claude;
mod error;

pub use claude::Extractor;
pub use error::{Result, TodoError};

use std::fmt;

use serde::Deserialize;

/// Who is responsible for a task, resolved against the speaker who said it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Owner {
    /// The local user — the "me" track.
    Me,
    /// A remote participant — the "them" track.
    Them,
    /// Stated as an action item, but with no clear owner.
    Unassigned,
}

impl fmt::Display for Owner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Me => "me",
            Self::Them => "them",
            Self::Unassigned => "unassigned",
        })
    }
}

/// One extracted action item.
#[derive(Debug, Clone, Deserialize)]
pub struct Todo {
    /// The task, phrased as an imperative.
    pub task: String,
    /// Who owns it. See [`Owner`].
    pub owner: Owner,
    /// The transcript line the task was drawn from, kept verbatim so a todo can
    /// always be traced back to what was actually said.
    pub quote: String,
    /// A deadline, if one was mentioned, in the transcript's own words
    /// (e.g. "by Friday", "hari ini"). `None` when none was stated.
    #[serde(default)]
    pub due: Option<String>,
}
