//! The worlds directory — a "jump anywhere" list of places on the Thread.
//!
//! Fetches the live world list (`GET <base>/thread/worlds`) on a background
//! thread and offers it as a panel; selecting an entry veilwalks there. It's the
//! browser's address book + a guaranteed way home, so you can never get stranded.

use std::sync::mpsc::{channel, Receiver, TryRecvError};

use serde::Deserialize;

/// One listed world.
#[derive(Debug, Clone)]
pub struct WorldEntry {
    pub title: String,
    pub locator: String,
    pub description: String,
}

#[derive(Deserialize)]
struct WorldsResponse {
    #[serde(default)]
    worlds: Vec<RawEntry>,
}

#[derive(Deserialize)]
struct RawEntry {
    #[serde(default)]
    title: String,
    #[serde(default)]
    locator: String,
    #[serde(default)]
    description: String,
}

/// The worlds directory.
pub struct Directory {
    base: String,
    open: bool,
    entries: Option<Vec<WorldEntry>>,
    error: Option<String>,
    pending: Option<Receiver<Result<Vec<WorldEntry>, String>>>,
    /// The ranked live search: query text this result set answers, its results
    /// (content match + portal-graph centrality, server-side), and the fetch
    /// in flight. Last-typed wins; a stale landing is dropped.
    search_query: String,
    search_results: Option<Vec<SearchHit>>,
    search_pending: Option<(String, Receiver<Result<Vec<SearchHit>, String>>)>,
}

/// One ranked search result from the Waystone index (`/thread/search`).
#[derive(Debug, Clone)]
pub struct SearchHit {
    pub title: String,
    pub locator: String,
    /// Content snippet matching the query (the "why" of the hit).
    pub snippet: String,
    /// How many veils lead here — the Thread's PageRank cousin.
    pub veils_in: u32,
}

impl Directory {
    pub fn from_env() -> Self {
        let base = std::env::var("INFINITE_THREAD_BASE")
            .unwrap_or_else(|_| "https://api.pixygon.io/v1".to_string());
        Self {
            base,
            open: false,
            entries: None,
            error: None,
            pending: None,
            search_query: String::new(),
            search_results: None,
            search_pending: None,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }
    pub fn close(&mut self) {
        self.open = false;
    }

    /// Open/close the directory. Fetches the list the first time it's opened.
    pub fn toggle(&mut self) {
        self.open = !self.open;
        self.ensure_loaded();
    }

    /// Start fetching the world list if we don't have it yet (without opening the
    /// panel) — so portal previews have data from the first frame.
    pub fn ensure_loaded(&mut self) {
        if self.entries.is_some() || self.pending.is_some() {
            return;
        }
        self.error = None;
        let url = format!("{}/thread/worlds", self.base.trim_end_matches('/'));
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(fetch_worlds(&url));
        });
        self.pending = Some(rx);
    }

    /// Look up a listed world by Locator (ignoring `#anchor` / `@when`).
    pub fn lookup(&self, locator: &str) -> Option<&WorldEntry> {
        let key = normalize_locator(locator);
        self.entries
            .as_ref()?
            .iter()
            .find(|e| normalize_locator(&e.locator) == key)
    }

    /// Ask the Waystone index for ranked places matching `query` — content
    /// match + portal-graph in-degree, the live "Google of the Thread" call.
    /// Call freely as the user types: identical queries are deduped, a newer
    /// query replaces the in-flight one (last-typed wins), and an empty query
    /// clears the results (the panel falls back to the plain list).
    pub fn search(&mut self, query: &str) {
        let q = query.trim().to_string();
        if q == self.search_query {
            return;
        }
        self.search_query = q.clone();
        if q.is_empty() {
            self.search_results = None;
            self.search_pending = None;
            return;
        }
        let url = format!(
            "{}/thread/search?q={}",
            self.base.trim_end_matches('/'),
            urlencode(&q)
        );
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(fetch_search(&url));
        });
        self.search_pending = Some((q, rx));
    }

    /// The ranked results for the current query, once landed.
    pub fn search_results(&self) -> Option<&[SearchHit]> {
        self.search_results.as_deref()
    }
    /// Whether a ranked search is still in flight.
    pub fn search_loading(&self) -> bool {
        self.search_pending.is_some()
    }

    /// Collect a finished fetch, if any. Call once per frame.
    pub fn poll(&mut self) {
        // Land a finished ranked search (only if it answers the current query).
        if let Some((q, rx)) = &self.search_pending {
            match rx.try_recv() {
                Ok(Ok(hits)) => {
                    if *q == self.search_query {
                        self.search_results = Some(hits);
                    }
                    self.search_pending = None;
                }
                Ok(Err(_)) | Err(TryRecvError::Disconnected) => {
                    // Ranked search failing is non-fatal — the panel's local
                    // substring filter still works; just stop spinning.
                    self.search_pending = None;
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        self.poll_worlds();
    }

    fn poll_worlds(&mut self) {
        let Some(rx) = &self.pending else { return };
        match rx.try_recv() {
            Ok(Ok(list)) => {
                self.entries = Some(list);
                self.pending = None;
            }
            Ok(Err(e)) => {
                self.error = Some(e);
                self.pending = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.error = Some("fetch stopped".into());
                self.pending = None;
            }
        }
    }

    pub fn entries(&self) -> Option<&[WorldEntry]> {
        self.entries.as_deref()
    }
    pub fn is_loading(&self) -> bool {
        self.pending.is_some()
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

fn fetch_worlds(url: &str) -> Result<Vec<WorldEntry>, String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    rt.block_on(async {
        let resp = reqwest::get(url).await.map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status().as_u16()));
        }
        let text = resp.text().await.map_err(|e| e.to_string())?;
        parse_worlds(&text)
    })
}

fn fetch_search(url: &str) -> Result<Vec<SearchHit>, String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    rt.block_on(async {
        let resp = reqwest::get(url).await.map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status().as_u16()));
        }
        let text = resp.text().await.map_err(|e| e.to_string())?;
        parse_search(&text)
    })
}

/// Minimal percent-encoding for a query-string value.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[derive(Deserialize)]
struct SearchResponse {
    #[serde(default)]
    places: Vec<RawHit>,
}

#[derive(Deserialize)]
struct RawHit {
    #[serde(default)]
    title: String,
    #[serde(default)]
    locator: String,
    #[serde(default)]
    snippet: String,
    #[serde(default, rename = "veilsIn")]
    veils_in: u32,
}

fn parse_search(json: &str) -> Result<Vec<SearchHit>, String> {
    let parsed: SearchResponse = serde_json::from_str(json).map_err(|e| e.to_string())?;
    Ok(parsed
        .places
        .into_iter()
        .filter(|h| !h.locator.is_empty())
        .map(|h| SearchHit {
            title: h.title,
            locator: h.locator,
            snippet: h.snippet,
            veils_in: h.veils_in,
        })
        .collect())
}

/// Strip a Locator to its base (`thread://host/path`) for comparison.
fn normalize_locator(loc: &str) -> String {
    loc.split(['#', '@'])
        .next()
        .unwrap_or(loc)
        .trim_end_matches('/')
        .to_string()
}

fn parse_worlds(json: &str) -> Result<Vec<WorldEntry>, String> {
    let parsed: WorldsResponse = serde_json::from_str(json).map_err(|e| e.to_string())?;
    Ok(parsed
        .worlds
        .into_iter()
        .filter(|e| !e.locator.is_empty())
        .map(|e| WorldEntry {
            title: e.title,
            locator: e.locator,
            description: e.description,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ranked_search_hits() {
        let json = r#"{"query":"weft","total":17,"count":2,"places":[
            {"locator":"thread://weft.pixygon.io","title":"The Weavery","snippet":"packages","veilsIn":3,"score":1.5},
            {"title":"No Locator","snippet":"dropped"},
            {"locator":"thread://wiki.pixygon.io","title":"The Atrium","snippet":"clock","veilsIn":2}
        ]}"#;
        let hits = parse_search(json).unwrap();
        assert_eq!(hits.len(), 2, "hits without a locator are dropped");
        assert_eq!(hits[0].title, "The Weavery");
        assert_eq!(hits[0].veils_in, 3);
        assert_eq!(hits[1].snippet, "clock");
    }

    #[test]
    fn urlencode_covers_query_text() {
        assert_eq!(urlencode("the weavery"), "the+weavery");
        assert_eq!(urlencode("a/b?c"), "a%2Fb%3Fc");
        assert_eq!(urlencode("plain-text_1.0~x"), "plain-text_1.0~x");
    }

    #[test]
    fn parses_the_worlds_list() {
        let json = r#"{"worlds":[
            {"id":"46000003","title":"The Nexus","locator":"thread://pixygon.io","description":"front door"},
            {"title":"No Locator","description":"skipped"},
            {"title":"The Forge","locator":"thread://pixiel.ai/forge","description":"workshop"}
        ]}"#;
        let list = parse_worlds(json).unwrap();
        assert_eq!(list.len(), 2, "entries without a locator are dropped");
        assert_eq!(list[0].title, "The Nexus");
        assert_eq!(list[1].locator, "thread://pixiel.ai/forge");
    }
}
