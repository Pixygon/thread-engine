//! Bookmarks — a traveler's personal constellation.
//!
//! The web felt navigable because you could *keep* places: bookmarks, a home
//! page, history. This is the Thread's version of the first of those — a small,
//! durable list of `(label, locator)` pairs a browser persists across sessions.
//! Storage is one human-readable JSON file (view-source culture applies to your
//! own data too); a missing or corrupt file degrades to an empty list, never an
//! error.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// One kept place: a Locator plus the label it was saved under (usually the
/// world's title at the time).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bookmark {
    pub label: String,
    /// A `thread://` Locator.
    pub locator: String,
}

/// A persisted bookmark list. Every mutation saves immediately — the file is
/// tiny and losing a bookmark to a crash would violate the "solid" tenet.
pub struct Bookmarks {
    path: PathBuf,
    entries: Vec<Bookmark>,
}

impl Bookmarks {
    /// Load from `path` (missing or unreadable → empty list, never an error).
    pub fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let entries = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Self { path, entries }
    }

    /// The default per-user store: `~/.config/infinite/bookmarks.json` (or the
    /// platform equivalent via `XDG_CONFIG_HOME`), falling back to the current
    /// directory when no home is known.
    pub fn default_path() -> PathBuf {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("infinite")
            .join("bookmarks.json")
    }

    pub fn entries(&self) -> &[Bookmark] {
        &self.entries
    }

    pub fn contains(&self, locator: &str) -> bool {
        self.entries.iter().any(|b| b.locator == locator)
    }

    /// Add a bookmark (de-duplicated by Locator; re-adding refreshes the label).
    pub fn add(&mut self, label: impl Into<String>, locator: impl Into<String>) {
        let (label, locator) = (label.into(), locator.into());
        if let Some(b) = self.entries.iter_mut().find(|b| b.locator == locator) {
            b.label = label;
        } else {
            self.entries.push(Bookmark { label, locator });
        }
        self.save();
    }

    /// Remove by Locator. Returns whether anything was removed.
    pub fn remove(&mut self, locator: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|b| b.locator != locator);
        let removed = self.entries.len() != before;
        if removed {
            self.save();
        }
        removed
    }

    /// Bookmark it if it isn't, un-bookmark it if it is (the star button).
    /// Returns `true` when the place is bookmarked *after* the call.
    pub fn toggle(&mut self, label: impl Into<String>, locator: impl Into<String>) -> bool {
        let locator = locator.into();
        if self.contains(&locator) {
            self.remove(&locator);
            false
        } else {
            self.add(label, locator);
            true
        }
    }

    fn save(&self) {
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        match serde_json::to_string_pretty(&self.entries) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&self.path, json) {
                    tracing::warn!("bookmarks: cannot write {}: {e}", self.path.display());
                }
            }
            Err(e) => tracing::warn!("bookmarks: serialize failed: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("loom-bookmarks-{}-{tag}.json", std::process::id()))
    }

    #[test]
    fn add_toggle_and_persist_across_loads() {
        let path = temp_store("roundtrip");
        let _ = std::fs::remove_file(&path);

        let mut b = Bookmarks::load(&path);
        assert!(b.entries().is_empty());
        b.add("The Nexus", "thread://nexus.pixygon.io/nexus");
        assert!(b.contains("thread://nexus.pixygon.io/nexus"));

        // The star button: on, then off.
        assert!(
            b.toggle("The Forge", "thread://pixiel.ai/forge"),
            "first toggle bookmarks it"
        );
        assert!(
            !b.toggle("The Forge", "thread://pixiel.ai/forge"),
            "second removes it"
        );

        // A fresh load sees what was saved.
        let again = Bookmarks::load(&path);
        assert_eq!(again.entries().len(), 1);
        assert_eq!(again.entries()[0].label, "The Nexus");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn re_adding_refreshes_the_label_without_duplicating() {
        let path = temp_store("relabel");
        let _ = std::fs::remove_file(&path);

        let mut b = Bookmarks::load(&path);
        b.add("Old Title", "thread://a/x");
        b.add("New Title", "thread://a/x");
        assert_eq!(b.entries().len(), 1);
        assert_eq!(b.entries()[0].label, "New Title");

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_corrupt_store_degrades_to_empty_not_error() {
        let path = temp_store("corrupt");
        std::fs::write(&path, "not json at all {{{").unwrap();
        let b = Bookmarks::load(&path);
        assert!(b.entries().is_empty());
        let _ = std::fs::remove_file(&path);
    }
}
