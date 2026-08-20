//! Asset resolution — turn a World Manifest asset URI into a readable local file,
//! fetching remote ones into an on-disk cache.
//!
//! A manifest references glTF meshes and textures by [`Asset`] URI, which may be a
//! path relative to the world, an absolute `http(s)://` URL, or `ipfs://…`. An
//! [`AssetSource`] resolves any of these to a local path a renderer can open —
//! downloading remote ones once and caching them by URL, so revisits are instant.
//!
//! It is renderer-agnostic on purpose: the browser hands it to its `WorldLoader`,
//! which calls [`AssetSource::resolve`] wherever it used to join a local path.
//! [`AssetSource::prefetch`] downloads a whole world's assets up front (call it on
//! the background travel thread), so the main thread only ever reads local files.

use std::path::PathBuf;

use infinite_manifest::WorldManifest;

/// Where a world's assets live, and where fetched ones are cached.
pub struct AssetSource {
    /// Base for relative URIs when the world is local (a `world.json` directory).
    base_dir: PathBuf,
    /// Base URL for relative URIs when the world is hosted (e.g. its `.well-known`
    /// directory). `None` for a local world.
    base_url: Option<String>,
    /// Directory fetched remote assets are cached in.
    cache_dir: PathBuf,
}

impl AssetSource {
    /// A local world: relative URIs resolve against `base_dir`; absolute URLs still
    /// fetch to a shared cache.
    pub fn local(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
            base_url: None,
            cache_dir: default_cache(),
        }
    }

    /// A hosted world: `base_url` is the world's directory URL (relative URIs resolve
    /// against it); remote assets cache under `cache_dir`.
    pub fn hosted(
        base_dir: impl Into<PathBuf>,
        base_url: Option<String>,
        cache_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            base_dir: base_dir.into(),
            base_url,
            cache_dir: cache_dir.into(),
        }
    }

    /// Resolve an asset URI to a readable local path, fetching + caching if remote.
    /// Returns `None` if a remote fetch failed (the caller degrades gracefully —
    /// e.g. falls back to a built-in mesh or a flat colour).
    pub fn resolve(&self, uri: &str) -> Option<PathBuf> {
        match self.absolute_url(uri) {
            Some(url) => self.fetch_cached(&url),
            None => Some(self.base_dir.join(uri)),
        }
    }

    /// Download every remote asset in `manifest` into the cache (idempotent). Run on
    /// a background thread before the main-thread GPU load so it only reads locally.
    pub fn prefetch(&self, manifest: &WorldManifest) {
        for a in &manifest.assets {
            if self.absolute_url(&a.uri).is_some() && self.resolve(&a.uri).is_none() {
                tracing::warn!("asset: could not fetch '{}'", a.uri);
            }
        }
    }

    /// The absolute URL an asset URI resolves to, or `None` if it's a local relative
    /// path (a local world with no base URL).
    fn absolute_url(&self, uri: &str) -> Option<String> {
        if uri.starts_with("http://") || uri.starts_with("https://") {
            Some(uri.to_string())
        } else if let Some(rest) = uri.strip_prefix("ipfs://") {
            Some(format!("https://ipfs.io/ipfs/{rest}"))
        } else if let Some(base) = &self.base_url {
            join_url(base, uri)
        } else {
            None
        }
    }

    /// Fetch `url` into the cache (skipping the download if already present).
    fn fetch_cached(&self, url: &str) -> Option<PathBuf> {
        let path = self.cache_dir.join(cache_name(url));
        if path.exists() {
            return Some(path);
        }
        let bytes = match http_get_bytes(url) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!("asset: fetch failed for {url}: {e}");
                return None;
            }
        };
        std::fs::create_dir_all(&self.cache_dir).ok()?;
        std::fs::write(&path, &bytes).ok()?;
        tracing::info!("asset: fetched {} ({} KB)", url, bytes.len() / 1024);
        Some(path)
    }
}

/// Default shared cache directory for fetched assets.
fn default_cache() -> PathBuf {
    std::env::temp_dir().join("infinite_thread_assets")
}

/// Join a base directory URL with a relative URI (`base` should end in `/`).
fn join_url(base: &str, uri: &str) -> Option<String> {
    reqwest::Url::parse(base)
        .ok()?
        .join(uri)
        .ok()
        .map(|u| u.to_string())
}

/// A stable cache filename for a URL: a hash of the URL + the asset's extension
/// (extension preserved so glTF/image loaders can sniff the format).
fn cache_name(url: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in url.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    let ext = url
        .rsplit('/')
        .next()
        .and_then(|seg| seg.split(['?', '#']).next())
        .and_then(|name| name.rsplit_once('.'))
        .map(|(_, e)| e)
        .filter(|e| e.len() <= 5 && e.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or("bin");
    format!("{h:016x}.{ext}")
}

/// Blocking HTTP GET returning the body bytes (on a throwaway runtime).
fn http_get_bytes(url: &str) -> Result<Vec<u8>, String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    rt.block_on(async {
        // Some hosts (Wikimedia among them) refuse UA-less requests.
        let client = reqwest::Client::builder()
            .user_agent("Infinite-Thread-Browser/0.1 (+https://github.com/Pixygon/Infinite)")
            .build()
            .map_err(|e| e.to_string())?;
        let resp = client.get(url).send().await.map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status().as_u16()));
        }
        resp.bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| e.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn local_relative_uris_join_the_base_dir() {
        let src = AssetSource::local("/worlds/gallery");
        assert_eq!(
            src.resolve("meshes/statue.glb"),
            Some(PathBuf::from("/worlds/gallery/meshes/statue.glb"))
        );
        // A local world with no base URL never treats a relative path as remote.
        assert!(src.absolute_url("meshes/statue.glb").is_none());
    }

    #[test]
    fn absolute_and_ipfs_uris_become_urls() {
        let src = AssetSource::local("/w");
        assert_eq!(
            src.absolute_url("https://cdn.example/tree.glb").as_deref(),
            Some("https://cdn.example/tree.glb")
        );
        assert_eq!(
            src.absolute_url("ipfs://Qm123/model.glb").as_deref(),
            Some("https://ipfs.io/ipfs/Qm123/model.glb")
        );
    }

    #[test]
    fn hosted_relative_uris_resolve_against_the_world_url() {
        let src = AssetSource::hosted(
            Path::new("/cache"),
            Some("https://studio.example/.well-known/thread/gallery/".into()),
            "/cache",
        );
        assert_eq!(
            src.absolute_url("meshes/statue.glb").as_deref(),
            Some("https://studio.example/.well-known/thread/gallery/meshes/statue.glb")
        );
    }

    /// The live remote-asset walk: fetch a real glTF over the network through the
    /// same path the background travel thread uses (`prefetch` → `resolve`).
    /// Ignored by default (network); run with `cargo test -p loom -- --ignored`.
    /// Once the hosted worlds' assetBase points at the CDN, this is the code path
    /// every remote world walks through.
    #[test]
    #[ignore = "requires network"]
    fn a_remote_gltf_fetches_once_and_caches() {
        // Khronos' sample Box.glb — tiny (~1.6 KB), stable, canonical.
        const URL: &str = "https://raw.githubusercontent.com/KhronosGroup/glTF-Sample-Assets/main/Models/Box/glTF-Binary/Box.glb";
        let cache = std::env::temp_dir().join(format!("loom-remote-asset-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&cache);
        let src = AssetSource::hosted(Path::new("/nowhere"), None, &cache);

        // Prefetch via a manifest, exactly as the travel thread does.
        let manifest: WorldManifest = serde_json::from_value(serde_json::json!({
            "thread": "0.1", "world": { "id": "remote-test", "title": "Remote" },
            "assets": [{ "id": "box", "uri": URL, "kind": "gltf" }]
        }))
        .unwrap();
        src.prefetch(&manifest);

        let path = src
            .resolve(URL)
            .expect("remote glb resolves to a local path");
        let bytes = std::fs::read(&path).expect("cached file is readable");
        assert_eq!(
            &bytes[..4],
            b"glTF",
            "the cached bytes are a real binary glTF"
        );

        // Second resolve is a cache hit — same path, no re-download needed.
        let mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(src.resolve(URL).as_deref(), Some(path.as_path()));
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            mtime,
            "not re-fetched"
        );

        let _ = std::fs::remove_dir_all(&cache);
    }

    #[test]
    fn cache_names_are_stable_and_keep_the_extension() {
        let a = cache_name("https://cdn/tree.glb");
        assert_eq!(a, cache_name("https://cdn/tree.glb"), "deterministic");
        assert!(a.ends_with(".glb"));
        assert!(
            cache_name("https://cdn/img.png?v=2").ends_with(".png"),
            "strips query"
        );
        assert!(
            cache_name("https://cdn/noext").ends_with(".bin"),
            "falls back"
        );
        assert_ne!(
            cache_name("https://cdn/a.glb"),
            cache_name("https://cdn/b.glb")
        );
    }
}
