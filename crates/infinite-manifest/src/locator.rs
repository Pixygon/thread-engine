//! The **Locator** — the address of a place on the Thread (the URL of the
//! spatial web).
//!
//! Grammar (v0.1):
//! ```text
//! thread://<host>/<path>[@<when>][#<place>]
//! ```
//! - `host`  — the world host authority (a domain, like the web's).
//! - `path`  — the world's path on that host (may be empty for the host root).
//! - `@when` — OPTIONAL timeline year; time is a first-class navigation axis.
//! - `#place`— OPTIONAL named anchor (a spawn or portal id) to arrive at.
//!
//! Examples:
//! - `thread://archive.pixygon.io/codex-archive`
//! - `thread://market.pixygon.io/market#entry`
//! - `thread://amebrak.pixygon.io/caul@0`  (the same place, at planet-year 0)

/// The URL scheme of the Thread.
pub const SCHEME: &str = "thread://";

/// A parsed [Locator]. Round-trips through [`Locator::parse`] / [`Locator::to_string`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locator {
    pub host: String,
    pub path: String,
    /// Timeline year to arrive at (`@when`), if specified.
    pub when: Option<i64>,
    /// Named anchor within the world (`#place`), if specified.
    pub place: Option<String>,
}

impl Locator {
    /// Parse a `thread://…` address. Returns `None` if the scheme or host is missing.
    pub fn parse(s: &str) -> Option<Locator> {
        let rest = s.strip_prefix(SCHEME)?;
        // Split trailing #place first (a '#' can't appear in host/path).
        let (rest, place) = match rest.split_once('#') {
            Some((r, p)) => (r, Some(p.to_string())),
            None => (rest, None),
        };
        // Then @when.
        let (rest, when) = match rest.split_once('@') {
            Some((r, w)) => (r, w.parse::<i64>().ok()),
            None => (rest, None),
        };
        // host is the first path segment; the remainder is the world path.
        let (host, path) = match rest.split_once('/') {
            Some((h, p)) => (h, p),
            None => (rest, ""),
        };
        if host.is_empty() {
            return None;
        }
        Some(Locator {
            host: host.to_string(),
            path: path.trim_end_matches('/').to_string(),
            when,
            place,
        })
    }
}

/// The canonical `.well-known` manifest URL for a host + world path —
/// `https://<host>/.well-known/thread[/<path>]/world.json`. This is the
/// decentralized, zero-registry convention: serve a world here and any browser
/// can reach it at `thread://<host>[/<path>]`. Single source of truth shared by
/// the browser resolver and the `thread` CLI.
pub fn well_known_url(host: &str, path: &str) -> String {
    // A host carrying an explicit port is a direct invite address (a browser
    // or LAN box hosting a place itself — e.g. a home with guests over); those
    // serve plain HTTP. Bare hosts are web hosts and stay TLS-only.
    let scheme = if host.contains(':') { "http" } else { "https" };
    let mut base = format!("{scheme}://{host}/.well-known/thread");
    let p = path.trim_matches('/');
    if !p.is_empty() {
        base.push('/');
        base.push_str(p);
    }
    format!("{base}/world.json")
}

impl std::fmt::Display for Locator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{SCHEME}{}", self.host)?;
        if !self.path.is_empty() {
            write!(f, "/{}", self.path)?;
        }
        if let Some(w) = self.when {
            write!(f, "@{w}")?;
        }
        if let Some(p) = &self.place {
            write!(f, "#{p}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_locator() {
        let l = Locator::parse("thread://market.pixygon.io/shops/bazaar@0#entry").unwrap();
        assert_eq!(l.host, "market.pixygon.io");
        assert_eq!(l.path, "shops/bazaar");
        assert_eq!(l.when, Some(0));
        assert_eq!(l.place.as_deref(), Some("entry"));
    }

    #[test]
    fn parses_host_only_and_roundtrips() {
        let l = Locator::parse("thread://archive.pixygon.io").unwrap();
        assert_eq!(l.host, "archive.pixygon.io");
        assert_eq!(l.path, "");
        assert_eq!(l.to_string(), "thread://archive.pixygon.io");

        let full = "thread://a.b/c/d@1850#x";
        assert_eq!(Locator::parse(full).unwrap().to_string(), full);
    }

    #[test]
    fn rejects_non_thread() {
        assert!(Locator::parse("https://example.com").is_none());
        assert!(Locator::parse("thread://").is_none());
    }

    #[test]
    fn builds_well_known_urls() {
        assert_eq!(
            well_known_url("mydomain.com", ""),
            "https://mydomain.com/.well-known/thread/world.json"
        );
        assert_eq!(
            well_known_url("studio.example.org", "gallery"),
            "https://studio.example.org/.well-known/thread/gallery/world.json"
        );
    }
}
