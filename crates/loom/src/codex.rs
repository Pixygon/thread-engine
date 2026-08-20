//! In-world Codex reader — the "inspect" side of the Thread.
//!
//! When the player inspects a Codex-linked placement, this fetches the canonical
//! entry from the main agent's live endpoint (`GET <base>/codex/entities/:slug`)
//! on a background thread and hands back a display panel. Anonymous reads see
//! the public tier; a traveler carrying a Passport presents it as a bearer
//! token, unlocking whatever tier their identity holds (under-tier reads still
//! answer — with the universe's in-fiction refusal).
//! Failures degrade gracefully (the world keeps running — super-stable tenet);
//! successes are cached. The base URL comes from `INFINITE_CODEX_BASE`
//! (default `https://api.pixygon.io/v1`); when the resolver is wired it will
//! supply `codexBase` per world.

use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, TryRecvError};

use serde::Deserialize;

/// A Codex entry, shaped to the main agent's read API (tolerant to extra fields).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CodexEntry {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub slug: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub subkind: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, rename = "accessTier")]
    pub access_tier: Option<String>,
}

/// What to draw for the Codex reader this frame (owned so egui closures are free).
#[derive(Debug, Clone)]
pub struct CodexPanel {
    pub title: String,
    pub subtitle: String,
    pub body: String,
    pub loading: bool,
}

enum State {
    Idle,
    Loading(String),
    Loaded(Box<CodexEntry>),
    Error(String, String),
}

/// The in-world Codex reader.
pub struct CodexClient {
    base: String,
    /// Passport bearer token — presented on every read so tiered entries
    /// unlock to whatever this identity holds. `None` reads as public.
    token: Option<String>,
    state: State,
    cache: HashMap<String, CodexEntry>,
    pending: Option<(String, Receiver<Result<CodexEntry, String>>)>,
}

impl CodexClient {
    pub fn new(base: impl Into<String>) -> Self {
        Self {
            base: base.into(),
            token: None,
            state: State::Idle,
            cache: HashMap::new(),
            pending: None,
        }
    }

    /// Present a Passport on Codex reads (`Authorization: Bearer <token>`),
    /// unlocking the identity's access tier. `None`/empty → anonymous (public).
    pub fn with_token(mut self, token: Option<String>) -> Self {
        self.token = token.filter(|t| !t.trim().is_empty());
        self
    }

    /// Read the base URL from `INFINITE_CODEX_BASE` (falls back to the public API).
    pub fn from_env() -> Self {
        let base = std::env::var("INFINITE_CODEX_BASE")
            .unwrap_or_else(|_| "https://api.pixygon.io/v1".to_string());
        Self::new(base)
    }

    /// Whether a reader panel is currently showing (or loading).
    pub fn is_open(&self) -> bool {
        !matches!(self.state, State::Idle)
    }

    pub fn close(&mut self) {
        self.state = State::Idle;
        self.pending = None;
    }

    /// Open a Codex entry by slug (cached, or fetched on a background thread).
    pub fn open(&mut self, slug: &str) {
        if let Some(entry) = self.cache.get(slug) {
            self.state = State::Loaded(Box::new(entry.clone()));
            return;
        }
        let url = entry_url(&self.base, slug);
        let token = self.token.clone();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(fetch_blocking(&url, token.as_deref()));
        });
        self.state = State::Loading(slug.to_string());
        self.pending = Some((slug.to_string(), rx));
    }

    /// Collect a finished background fetch, if any. Call once per frame.
    pub fn poll(&mut self) {
        let Some((slug, rx)) = &self.pending else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(entry)) => {
                self.cache.insert(slug.clone(), entry.clone());
                self.state = State::Loaded(Box::new(entry));
                self.pending = None;
            }
            Ok(Err(msg)) => {
                self.state = State::Error(slug.clone(), msg);
                self.pending = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.state = State::Error(slug.clone(), "fetch thread stopped".into());
                self.pending = None;
            }
        }
    }

    /// The panel to render, or `None` when the reader is closed.
    pub fn panel(&self) -> Option<CodexPanel> {
        match &self.state {
            State::Idle => None,
            State::Loading(slug) => Some(CodexPanel {
                title: slug.clone(),
                subtitle: "Consulting the Codex…".into(),
                body: String::new(),
                loading: true,
            }),
            State::Loaded(e) => Some(CodexPanel {
                title: if e.name.is_empty() {
                    e.slug.clone()
                } else {
                    e.name.clone()
                },
                subtitle: match &e.subkind {
                    Some(s) if !s.is_empty() => format!("{} · {s}", e.kind),
                    _ => e.kind.clone(),
                },
                body: e
                    .description
                    .clone()
                    .filter(|s| !s.is_empty())
                    .or_else(|| e.summary.clone())
                    .unwrap_or_default(),
                loading: false,
            }),
            State::Error(slug, msg) => Some(CodexPanel {
                title: slug.clone(),
                subtitle: "The Codex is out of reach".into(),
                body: msg.clone(),
                loading: false,
            }),
        }
    }
}

/// Build the read URL for a slug.
fn entry_url(base: &str, slug: &str) -> String {
    format!("{}/codex/entities/{}", base.trim_end_matches('/'), slug)
}

/// Blocking fetch on a throwaway current-thread runtime (reqwest is async-only
/// in this workspace). Runs on a background thread, so blocking is fine here.
/// A Passport token, when present, rides as a bearer header (tier unlock).
fn fetch_blocking(url: &str, token: Option<&str>) -> Result<CodexEntry, String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    rt.block_on(async {
        let client = reqwest::Client::new();
        let mut req = client.get(url);
        if let Some(t) = token {
            req = req.bearer_auth(t);
        }
        let resp = req.send().await.map_err(|e| e.to_string())?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| e.to_string())?;
        if !status.is_success() {
            // Under-tier reads return a diegetic refusal voiced by the universe
            // (the Codex speaks in-fiction). Surface that message — it's content.
            return Err(
                refusal_message(&text).unwrap_or_else(|| format!("HTTP {}", status.as_u16()))
            );
        }
        parse_entry(&text)
    })
}

fn parse_entry(json: &str) -> Result<CodexEntry, String> {
    serde_json::from_str(json).map_err(|e| e.to_string())
}

/// The in-fiction refusal message from a gated read, if the body carries one.
fn refusal_message(body: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct Refusal {
        in_lore: Option<InLore>,
    }
    #[derive(Deserialize)]
    struct InLore {
        message: String,
    }
    serde_json::from_str::<Refusal>(body)
        .ok()?
        .in_lore
        .map(|l| l.message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_entry_url() {
        assert_eq!(
            entry_url("https://api.pixygon.io/v1", "the-veil"),
            "https://api.pixygon.io/v1/codex/entities/the-veil"
        );
        // Trailing slash is normalized.
        assert_eq!(
            entry_url("https://api.pixygon.io/v1/", "amebrak"),
            "https://api.pixygon.io/v1/codex/entities/amebrak"
        );
    }

    #[test]
    fn parses_the_read_api_shape() {
        let json = r#"{
            "name": "The Veil", "slug": "the-veil", "kind": "concept",
            "subkind": "membrane", "summary": "A temporary membrane.",
            "description": "The magical, temporary membrane...", "tags": ["wardenship"],
            "accessTier": "public", "extraFieldWeIgnore": 42
        }"#;
        let e = parse_entry(json).unwrap();
        assert_eq!(e.name, "The Veil");
        assert_eq!(e.kind, "concept");
        assert_eq!(e.subkind.as_deref(), Some("membrane"));
        assert_eq!(e.access_tier.as_deref(), Some("public"));

        let panel = CodexClient {
            base: String::new(),
            token: None,
            state: State::Loaded(Box::new(e)),
            cache: HashMap::new(),
            pending: None,
        }
        .panel()
        .unwrap();
        assert_eq!(panel.title, "The Veil");
        assert_eq!(panel.subtitle, "concept · membrane");
        assert!(!panel.loading);
    }

    #[test]
    fn blank_tokens_read_as_anonymous() {
        assert!(CodexClient::new("x").with_token(None).token.is_none());
        assert!(CodexClient::new("x")
            .with_token(Some("  ".into()))
            .token
            .is_none());
        assert_eq!(
            CodexClient::new("x")
                .with_token(Some("tok".into()))
                .token
                .as_deref(),
            Some("tok")
        );
    }

    #[test]
    fn surfaces_diegetic_refusal_from_a_gated_read() {
        let body = r#"{"error":"access_denied","in_lore":{"message":"The Archivist meets your gaze. 'Walk on, traveler.'","seal":"oath-of-the-archive","tier":"archivist"}}"#;
        assert_eq!(
            refusal_message(body).as_deref(),
            Some("The Archivist meets your gaze. 'Walk on, traveler.'")
        );
        // A plain error body has no in-fiction message.
        assert!(refusal_message(r#"{"error":"nope"}"#).is_none());
    }
}
