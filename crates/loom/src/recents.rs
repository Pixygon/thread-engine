//! Recents — the threads a traveler last walked.
//!
//! The web's history bar, kept the Thread's way: a small, durable,
//! most-recent-first list of `(label, locator)` pairs. The home-space renders
//! it as an arc of veils ([`crate::home::sync_recents`]) so "where was I?"
//! is answered by looking around your own home, not by opening a menu.
//! Storage is one human-readable JSON file beside the bookmarks; a missing or
//! corrupt file degrades to an empty list, never an error.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// How many recent places the list keeps (and the home renders).
pub const CAP: usize = 6;

/// One recently visited place.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recent {
    pub label: String,
    /// A `thread://` Locator.
    pub locator: String,
}

/// A persisted recent-places list, most recent first. Every mutation saves
/// immediately (the file is tiny).
pub struct Recents {
    path: PathBuf,
    entries: Vec<Recent>,
}

impl Recents {
    /// Load from `path` (missing or unreadable → empty list, never an error).
    pub fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let entries = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Self { path, entries }
    }

    /// The default per-user store: `~/.config/infinite/recents.json`.
    pub fn default_path() -> PathBuf {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("infinite")
            .join("recents.json")
    }

    /// Most recent first.
    pub fn entries(&self) -> &[Recent] {
        &self.entries
    }

    /// Record a visit: moves the place to the front (refreshing its label),
    /// trims to [`CAP`]. Home itself is never recorded — you don't "recently
    /// visit" your own hearth. Returns whether the list changed.
    pub fn visit(&mut self, label: &str, locator: &str) -> bool {
        let locator = locator
            .split(['#', '@'])
            .next()
            .unwrap_or(locator)
            .trim_end_matches('/');
        if locator.is_empty() || crate::home::is_home(locator) {
            return false;
        }
        let label = if label.trim().is_empty() {
            locator
        } else {
            label.trim()
        };
        if self
            .entries
            .first()
            .is_some_and(|r| r.locator == locator && r.label == label)
        {
            return false; // already freshest — no rewrite
        }
        self.entries.retain(|r| r.locator != locator);
        self.entries.insert(
            0,
            Recent {
                label: label.to_string(),
                locator: locator.to_string(),
            },
        );
        self.entries.truncate(CAP);
        self.save();
        true
    }

    fn save(&self) {
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_string_pretty(&self.entries) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&self.path, json) {
                    tracing::warn!("recents: cannot write {}: {e}", self.path.display());
                }
            }
            Err(e) => tracing::warn!("recents: serialize failed: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(tag: &str) -> PathBuf {
        let p =
            std::env::temp_dir().join(format!("loom-recents-{}-{tag}.json", std::process::id()));
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn visits_dedupe_reorder_cap_and_persist() {
        let path = temp_store("basic");
        let mut r = Recents::load(&path);
        assert!(r.visit("The Nexus", "thread://pixygon.io#entry"));
        assert!(r.visit("The Commons", "thread://commons.pixygon.io"));
        // Re-visiting moves to the front and strips the anchor; no duplicate.
        assert!(r.visit("The Nexus", "thread://pixygon.io"));
        assert_eq!(r.entries().len(), 2);
        assert_eq!(r.entries()[0].locator, "thread://pixygon.io");
        // Freshest repeat is a no-op (no file churn).
        assert!(!r.visit("The Nexus", "thread://pixygon.io"));
        // Home is never recorded.
        assert!(!r.visit("Home", crate::home::HOME_LOCATOR));
        // Cap holds.
        for i in 0..10 {
            r.visit(&format!("W{i}"), &format!("thread://w{i}.example"));
        }
        assert_eq!(r.entries().len(), CAP);
        assert_eq!(r.entries()[0].label, "W9");
        // And it round-trips from disk.
        let reloaded = Recents::load(&path);
        assert_eq!(reloaded.entries(), r.entries());
        let _ = std::fs::remove_file(&path);
    }
}
