//! Spaces: the folders recordings are filed into.
//!
//! ## Where membership lives
//!
//! The list of spaces is one JSON file in the app config directory. Which space a
//! recording belongs to is written *into the recording's own directory*, as
//! `meta.json`, rather than into a central index.
//!
//! That is deliberate. Audio already lives on disk as one directory per session, and
//! the disk is the source of truth: a recording can be moved, backed up, or deleted
//! in Finder, and the app has to agree with whatever it finds afterwards. A central
//! index would have to be reconciled against the filesystem on every launch, and the
//! failure mode is a list that claims recordings which are not there. Keeping
//! membership beside the audio makes that class of bug impossible: delete the folder
//! and the membership goes with it.
//!
//! A database earns its place when there are queries a directory walk cannot answer
//! (full-text search over transcripts, say). Filing a few dozen recordings into a few
//! spaces is not that.
//!
//! ## The default space
//!
//! "Default Space" is not a stored record. It is what a recording with no `meta.json`
//! belongs to, which also makes it what every recording made before spaces existed
//! belongs to. A space that has been deleted leaves its recordings pointing at an id
//! that no longer resolves; those fall back to the default too, so a delete can never
//! strand a recording somewhere it cannot be seen.

use std::path::Path;

use serde::{Deserialize, Serialize};

const FILE: &str = "spaces.json";

/// A user-created space.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Space {
    /// Stable id, generated once at creation. Never the name: renaming a space must
    /// not orphan the recordings filed into it.
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Unix seconds.
    pub created: u64,
}

/// One recording's filing, stored next to its audio.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecordingMeta {
    /// Space id, or `None` for the default space.
    #[serde(default)]
    pub space: Option<String>,
}

const META: &str = "meta.json";

pub fn load_all(config_dir: &Path) -> Vec<Space> {
    std::fs::read_to_string(config_dir.join(FILE))
        .ok()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

pub fn save_all(config_dir: &Path, spaces: &[Space]) -> std::io::Result<()> {
    std::fs::create_dir_all(config_dir)?;
    let json = serde_json::to_string_pretty(spaces).unwrap_or_else(|_| "[]".to_owned());
    std::fs::write(config_dir.join(FILE), json)
}

/// Reads a recording's filing. A missing or unreadable file means the default space,
/// which is also the right answer for recordings made before spaces existed.
pub fn load_meta(dir: &Path) -> RecordingMeta {
    std::fs::read_to_string(dir.join(META))
        .ok()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

pub fn save_meta(dir: &Path, meta: &RecordingMeta) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(meta).unwrap_or_else(|_| "{}".to_owned());
    std::fs::write(dir.join(META), json)
}

/// A short, sortable, collision-resistant id.
///
/// Time plus a hash of the name is enough here: ids are created by one user clicking
/// a button, so the only collision to defend against is two spaces created in the
/// same second.
pub fn new_id(name: &str, now: u64) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in name.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("sp-{now:x}-{:04x}", hash & 0xffff)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_spaces_file_is_an_empty_list() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(load_all(dir.path()).is_empty());
    }

    #[test]
    fn spaces_round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let spaces = vec![Space {
            id: "sp-1".to_owned(),
            name: "Client work".to_owned(),
            description: "Calls with the agency".to_owned(),
            created: 42,
        }];
        save_all(dir.path(), &spaces).expect("save");

        let loaded = load_all(dir.path());
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "Client work");
    }

    // A recording folder that predates spaces, or one hand-copied in, must still
    // list rather than being skipped or crashing the walk.
    #[test]
    fn recording_without_meta_belongs_to_the_default_space() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(load_meta(dir.path()).space.is_none());
    }

    #[test]
    fn corrupt_meta_falls_back_to_the_default_space() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join(META), "{{{").expect("write");
        assert!(load_meta(dir.path()).space.is_none());
    }

    #[test]
    fn meta_round_trips() {
        let dir = tempfile::tempdir().expect("tempdir");
        save_meta(
            dir.path(),
            &RecordingMeta {
                space: Some("sp-7".to_owned()),
            },
        )
        .expect("save");
        assert_eq!(load_meta(dir.path()).space.as_deref(), Some("sp-7"));
    }

    #[test]
    fn ids_differ_for_different_names_in_the_same_second() {
        assert_ne!(new_id("Client work", 100), new_id("Personal", 100));
    }
}
