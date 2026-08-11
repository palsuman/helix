//! Recent workspaces, the last 20, kept in user data (REQ-FS-001.6).
//!
//! Stored at `~/.helix/recent.json`, beside `settings.json`, and never inside a
//! workspace: the list is a property of the person, not of any project, and a
//! recent-workspace list committed to a repository would be both noise and a
//! small privacy leak.
//!
//! The list is keyed by workspace key, so re-opening the same workspace moves
//! its entry to the front rather than adding a second one, and a workspace
//! whose roots changed keeps one entry with the new root set.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ts_rs::TS;

/// How many entries the list keeps (REQ-FS-001.6).
pub const MAX_RECENT: usize = 20;

/// File name inside `~/.helix/`.
pub const RECENT_FILE_NAME: &str = "recent.json";

/// `~/.helix/recent.json`, when a home directory can be determined.
///
/// Derived the same way [`helix_config::user_settings_path`] derives the user
/// settings path, so the two cannot end up in different places on a machine
/// with an unusual environment.
pub fn recent_path() -> Option<PathBuf> {
    helix_config::user_settings_path()
        .and_then(|settings| settings.parent().map(|dir| dir.join(RECENT_FILE_NAME)))
}

/// One entry in the recent list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct RecentWorkspace {
    /// The workspace key, which is what makes re-opening move an entry rather
    /// than duplicate it.
    pub key: String,
    pub name: String,
    /// Absolute roots, in workspace order, so the welcome experience can show
    /// what a workspace actually is (REQ-WB-004.4).
    pub roots: Vec<String>,
    pub last_opened_ms: u64,
}

/// The list, most recently opened first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../frontend/src/generated/")]
pub struct RecentWorkspaces {
    pub entries: Vec<RecentWorkspace>,
}

impl RecentWorkspaces {
    /// Parse a stored list, keeping whatever entries are well-formed.
    ///
    /// A corrupt recent list is a cosmetic problem, so it is treated as one: an
    /// unreadable or malformed file yields an empty list and the next open
    /// rewrites it. Refusing to start over a bad most-recently-used list would
    /// be absurd.
    pub fn parse(body: &str) -> Self {
        let Ok(value) = serde_json::from_str::<Value>(body) else {
            return Self::default();
        };
        let entries = value
            .get("entries")
            .and_then(Value::as_array)
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|entry| {
                        serde_json::from_value::<RecentWorkspace>(entry.clone()).ok()
                    })
                    .take(MAX_RECENT)
                    .collect()
            })
            .unwrap_or_default();
        Self { entries }
    }

    /// Record a workspace as just opened, moving it to the front and trimming
    /// the list to [`MAX_RECENT`].
    pub fn record(&mut self, key: &str, name: &str, roots: &[PathBuf]) {
        self.entries.retain(|entry| entry.key != key);
        self.entries.insert(
            0,
            RecentWorkspace {
                key: key.to_string(),
                name: name.to_string(),
                roots: roots
                    .iter()
                    .map(|root| root.to_string_lossy().to_string())
                    .collect(),
                last_opened_ms: now_ms(),
            },
        );
        self.entries.truncate(MAX_RECENT);
    }

    /// Forget one workspace, for "Remove from Recent".
    pub fn forget(&mut self, key: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|entry| entry.key != key);
        before != self.entries.len()
    }

    pub fn to_pretty_json(&self) -> String {
        let mut text = serde_json::to_string_pretty(&json!({ "entries": self.entries }))
            .unwrap_or_else(|_| "{\"entries\":[]}".to_string());
        text.push('\n');
        text
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The entry for a key, if the list has one.
    pub fn get(&self, key: &str) -> Option<&RecentWorkspace> {
        self.entries.iter().find(|entry| entry.key == key)
    }

    /// Whether a stored root still exists, so the welcome experience can grey
    /// out a workspace whose folders are gone rather than offering a dead link.
    pub fn roots_present(entry: &RecentWorkspace) -> bool {
        entry.roots.iter().any(|root| Path::new(root).is_dir())
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots(paths: &[&str]) -> Vec<PathBuf> {
        paths.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn recording_puts_the_newest_first() {
        let mut recent = RecentWorkspaces::default();
        recent.record("a", "Api", &roots(&["/work/api"]));
        recent.record("b", "Web", &roots(&["/work/web"]));

        assert_eq!(recent.entries[0].key, "b");
        assert_eq!(recent.entries[1].key, "a");
        assert_eq!(recent.entries[0].roots, vec!["/work/web".to_string()]);
    }

    #[test]
    fn reopening_moves_an_entry_rather_than_duplicating_it() {
        let mut recent = RecentWorkspaces::default();
        recent.record("a", "Api", &roots(&["/work/api"]));
        recent.record("b", "Web", &roots(&["/work/web"]));
        recent.record("a", "Api", &roots(&["/work/api", "/work/web"]));

        assert_eq!(recent.len(), 2);
        assert_eq!(recent.entries[0].key, "a");
        assert_eq!(
            recent.entries[0].roots.len(),
            2,
            "the entry carries the root set it was last opened with"
        );
    }

    #[test]
    fn the_list_stops_at_twenty() {
        let mut recent = RecentWorkspaces::default();
        for i in 0..25 {
            recent.record(&format!("k{i}"), &format!("W{i}"), &roots(&["/work/x"]));
        }
        assert_eq!(recent.len(), MAX_RECENT);
        assert_eq!(recent.entries[0].key, "k24", "the newest survives");
        assert!(recent.get("k0").is_none(), "the oldest is dropped");
    }

    #[test]
    fn a_list_round_trips_through_its_stored_form() {
        let mut recent = RecentWorkspaces::default();
        recent.record("a", "Api", &roots(&["/work/api"]));
        let parsed = RecentWorkspaces::parse(&recent.to_pretty_json());
        assert_eq!(parsed, recent);
    }

    #[test]
    fn a_corrupt_list_is_an_empty_list_not_a_failure() {
        assert!(RecentWorkspaces::parse("{ not json").is_empty());
        assert!(RecentWorkspaces::parse("[]").is_empty());
        assert!(RecentWorkspaces::parse(r#"{ "entries": "nope" }"#).is_empty());
    }

    #[test]
    fn a_malformed_entry_is_skipped_and_the_rest_survive() {
        let body = r#"{ "entries": [
            { "key": "good", "name": "Api", "roots": ["/work/api"], "last_opened_ms": 1 },
            { "key": "bad" }
        ] }"#;
        let parsed = RecentWorkspaces::parse(body);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed.entries[0].key, "good");
    }

    #[test]
    fn forgetting_removes_one_entry() {
        let mut recent = RecentWorkspaces::default();
        recent.record("a", "Api", &roots(&["/work/api"]));
        assert!(recent.forget("a"));
        assert!(!recent.forget("a"));
        assert!(recent.is_empty());
    }

    #[test]
    fn the_recent_list_lives_beside_the_user_settings_file() {
        if let (Some(recent), Some(settings)) = (recent_path(), helix_config::user_settings_path())
        {
            assert_eq!(recent.parent(), settings.parent());
            assert!(recent.ends_with(RECENT_FILE_NAME));
        }
    }
}
