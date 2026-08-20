//! Locator resolution — the Thread's "DNS", network-first with a local fallback.
//!
//! Given a `thread://…` Locator, resolve it to a world's manifest text (+ where
//! its assets live). Tries the main agent's live resolver
//! (`GET <base>/thread/resolve?loc=…` → `{ manifestUrl, assetBase, … }`, then
//! fetches the manifest), and falls back to a **local** `world.json`
//! (`<worlds_root>/<path>/world.json`) when the network is unavailable. So the
//! browser works today against local worlds and cuts over to the live network
//! the moment the resolver is deployed — no code change.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use infinite_manifest::Locator;
use serde::Deserialize;

use crate::home::is_home;

/// A resolved world: its manifest text + where its assets resolve.
pub struct TravelResult {
    pub manifest_text: String,
    /// Local base for relative asset URIs (a local `world.json` dir), or the cache
    /// dir for a hosted world.
    pub asset_base: PathBuf,
    /// Base URL for relative asset URIs when the world is hosted (its directory URL),
    /// so `meshes/x.glb` fetches from the right place. `None` for a local world.
    pub asset_base_url: Option<String>,
}

/// The main agent's resolver response (only the fields the browser consumes).
#[derive(Deserialize)]
struct ResolveResponse {
    #[serde(rename = "manifestUrl")]
    manifest_url: String,
    #[serde(rename = "assetBase", default)]
    _asset_base: Option<String>,
}

/// A browser-registered **world synthesizer**: given a Locator no host serves
/// and no local file matches, it may build the world on the spot (the spatial
/// encyclopedia generates halls the moment someone steps toward them). Runs
/// on the background travel thread; return `None` to decline.
type Synthesizer = Box<dyn Fn(&str) -> Option<TravelResult> + Send + Sync>;
static SYNTHESIZER: OnceLock<Synthesizer> = OnceLock::new();

/// Register the session's synthesizer (first registration wins).
pub fn register_synthesizer(f: impl Fn(&str) -> Option<TravelResult> + Send + Sync + 'static) {
    let _ = SYNTHESIZER.set(Box::new(f));
}

/// Resolve a Locator to a world (blocking — call on a background thread), in
/// order of decreasing coupling:
///   1. the configured **resolver/registry** (Pixygon worlds + value-adds),
///   2. the host's own **`.well-known/thread`** — decentralized, zero-registry:
///      this is what lets *anyone* host a world on their domain with zero contact,
///   3. a **local** `world.json` (dev / offline authoring).
pub fn fetch_world(
    resolver_base: &str,
    worlds_root: &Path,
    locator: &str,
) -> Result<TravelResult, String> {
    let cache = || std::env::temp_dir().join("infinite_thread_assets");

    // 0. The traveler's own home — `thread://home` is the Thread's `localhost`:
    //    always the local home file, never the network. Resolving it here makes
    //    home first-class everywhere a Locator goes (address bar, history,
    //    portal destinations) without any host ever seeing it.
    if is_home(locator) {
        let path = crate::home::world_path(&crate::home::default_dir());
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("no home world at {} ({e})", path.display()))?;
        return Ok(TravelResult {
            manifest_text: text,
            asset_base: path.parent().unwrap_or(Path::new(".")).to_path_buf(),
            asset_base_url: None,
        });
    }

    // Dogfood mode: INFINITE_NO_REGISTRY=1 walks the Thread exactly as a
    // spec-only third-party browser would — `.well-known` and nothing else
    // (no registry, no local dev worlds, no synthesizer). If the constellation
    // rots behind the resolver, THIS run is how we notice before an early
    // adopter does.
    let spec_only = std::env::var("INFINITE_NO_REGISTRY").is_ok_and(|v| !v.is_empty() && v != "0");
    if spec_only {
        return match resolve_well_known(locator) {
            Ok((text, base_url)) => {
                tracing::info!("resolved '{locator}' via .well-known (spec-only mode)");
                Ok(TravelResult {
                    manifest_text: text,
                    asset_base: cache(),
                    asset_base_url: base_url,
                })
            }
            Err(e) => Err(format!(
                "no .well-known world for {locator} ({e}) — spec-only mode"
            )),
        };
    }

    // 1. Registry / resolver.
    let remote_err = match resolve_remote(resolver_base, locator) {
        Ok((text, base_url)) => {
            return Ok(TravelResult {
                manifest_text: text,
                asset_base: cache(),
                asset_base_url: base_url,
            })
        }
        Err(e) => e,
    };

    // 2. Decentralized: the host's own `.well-known/thread/…/world.json`.
    if let Ok((text, base_url)) = resolve_well_known(locator) {
        tracing::info!("resolved '{locator}' via .well-known (no registry)");
        return Ok(TravelResult {
            manifest_text: text,
            asset_base: cache(),
            asset_base_url: base_url,
        });
    }

    // 3. Local dev file.
    let path = local_path(worlds_root, locator)
        .ok_or_else(|| format!("'{locator}' is not a Locator ({remote_err})"))?;
    if let Ok(text) = std::fs::read_to_string(&path) {
        tracing::info!("resolved '{locator}' locally (network: {remote_err})");
        return Ok(TravelResult {
            manifest_text: text,
            asset_base: path.parent().unwrap_or(Path::new(".")).to_path_buf(),
            asset_base_url: None,
        });
    }

    // 4. Synthesis: a registered generator may build the world on approach.
    if let Some(synth) = SYNTHESIZER.get() {
        if let Some(tr) = synth(locator) {
            tracing::info!("synthesized '{locator}' on approach");
            return Ok(tr);
        }
    }
    Err(format!("no world at {} ({remote_err})", path.display()))
}

/// Fetch the host's own manifest directly by the `.well-known` convention —
/// no resolver, no registry, no Pixygon involvement. Returns the manifest text and
/// the world's directory URL (so relative assets fetch from the right place).
fn resolve_well_known(locator: &str) -> Result<(String, Option<String>), String> {
    let loc = Locator::parse(locator).ok_or_else(|| format!("'{locator}' is not a Locator"))?;
    let manifest_url = infinite_manifest::well_known_url(&loc.host, &loc.path);
    match http_get_text(&manifest_url) {
        Ok(text) => Ok((text, dir_url(&manifest_url))),
        Err(json_err) => {
            // A host may serve Thread markup instead of JSON — same convention,
            // `.thread` extension (exactly like serving HTML). The navigator's
            // `from_text` parses either form.
            let markup_url = format!(
                "{}world.thread",
                manifest_url.trim_end_matches("world.json")
            );
            let text = http_get_text(&markup_url).map_err(|_| json_err)?;
            Ok((text, dir_url(&markup_url)))
        }
    }
}

/// Hit the live resolver, then download the manifest it points at. Returns the
/// manifest text + the asset base URL (the resolver's `assetBase`, or the manifest's
/// own directory).
fn resolve_remote(base: &str, locator: &str) -> Result<(String, Option<String>), String> {
    let url = format!(
        "{}/thread/resolve?loc={}",
        base.trim_end_matches('/'),
        urlencode(locator)
    );
    let body = http_get_text(&url)?;
    let resolved: ResolveResponse = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let text = http_get_text(&resolved.manifest_url)?;
    let base_url = resolved
        ._asset_base
        .clone()
        .or_else(|| dir_url(&resolved.manifest_url));
    Ok((text, base_url))
}

/// The directory URL of a manifest URL (everything up to and including the last `/`).
fn dir_url(manifest_url: &str) -> Option<String> {
    manifest_url
        .rsplit_once('/')
        .map(|(dir, _)| format!("{dir}/"))
}

/// Map a Locator to its local `world.json` — or `world.thread` markup when only
/// that exists (dev fallback resolver; both source forms are first-class).
fn local_path(worlds_root: &Path, locator: &str) -> Option<PathBuf> {
    let loc = Locator::parse(locator)?;
    let dir = if loc.path.is_empty() {
        loc.host
    } else {
        loc.path
    };
    let json = worlds_root.join(&dir).join("world.json");
    if !json.exists() {
        let thread = worlds_root.join(&dir).join("world.thread");
        if thread.exists() {
            return Some(thread);
        }
    }
    Some(json)
}

/// Blocking HTTP GET on a throwaway runtime (reqwest is async-only here).
fn http_get_text(url: &str) -> Result<String, String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    rt.block_on(async {
        let resp = reqwest::get(url).await.map_err(|e| e.to_string())?;
        let status = resp.status();
        if !status.is_success() {
            return Err(format!("HTTP {}", status.as_u16()));
        }
        resp.text().await.map_err(|e| e.to_string())
    })
}

/// Percent-encode a Locator for use as a query value.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_locators_to_local_files() {
        let root = Path::new("worlds");
        assert_eq!(
            local_path(root, "thread://market.pixygon.io/market"),
            Some(PathBuf::from("worlds/market/world.json"))
        );
        assert_eq!(
            local_path(root, "thread://archive.pixygon.io/codex-archive#entry"),
            Some(PathBuf::from("worlds/codex-archive/world.json"))
        );
        assert!(local_path(root, "https://not-thread.com").is_none());
    }

    #[test]
    fn local_resolution_falls_back_to_markup_when_only_that_exists() {
        // Run against the reference corpus so the existence checks are honest
        // — and skip when it isn't there, because that corpus is content that
        // travels with the browser, not with the engine.
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../worlds");
        if !root.is_dir() {
            eprintln!("no reference corpus beside this crate — skipping");
            return;
        }
        // The Meadow ships as markup only → resolves to its .thread source.
        let meadow = local_path(&root, "thread://meadow").expect("locator parses");
        assert!(
            meadow.ends_with("meadow/world.thread"),
            "got {}",
            meadow.display()
        );
        // A world with a world.json keeps resolving to JSON.
        let market = local_path(&root, "thread://market.pixygon.io/market").unwrap();
        assert!(
            market.ends_with("market/world.json"),
            "got {}",
            market.display()
        );
    }

    #[test]
    fn thread_home_is_recognized_in_all_its_spellings() {
        assert!(is_home("thread://home"));
        assert!(is_home("thread://home#entry"));
        assert!(is_home("thread://home@1200"));
        // A world *named* home on a real host is not the home-space.
        assert!(!is_home("thread://example.com/home"));
        assert!(!is_home("thread://homestead.io"));
        assert!(!is_home("not a locator"));
    }

    #[test]
    fn percent_encodes_locators() {
        assert_eq!(
            urlencode("thread://market.pixygon.io/market#entry"),
            "thread%3A%2F%2Fmarket.pixygon.io%2Fmarket%23entry"
        );
    }
}
