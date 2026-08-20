//! The traveler's Passport — one portable identity across the Thread.
//!
//! Implements the *browser* side of [passport-v0.1](../../../docs/spec/passport-v0.1.md):
//! hold the token for the traveler, read its claims, and fetch the descriptor
//! (the portable "you": name, avatar, consent) fresh from the issuer. The token
//! comes from `INFINITE_PASSPORT`; the descriptor URL is the token's `avatar`
//! claim. Everything degrades gracefully — no passport means an anonymous
//! traveler, never a gate on walking (spec §4).
//!
//! Claims are decoded **unverified** here: the browser is the token's *holder*,
//! not a verifier — it reads its own claims for display and routing. Hosts and
//! relays are the parties that verify keys-only against the issuer's JWKS.

use std::collections::HashMap;
use std::sync::mpsc::{channel, Receiver, TryRecvError};

use infinite_avatar::AvatarSpec;
use serde::Deserialize;

/// The claims a browser reads from its own token (passport-v0.1 §2).
/// Tolerant to extra fields; every field optional so a dev-issued or
/// non-JWT opaque token still carries.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Claims {
    #[serde(default)]
    pub iss: String,
    /// DID-style stable id — THE identity.
    #[serde(default)]
    pub sub: String,
    #[serde(default)]
    pub name: Option<String>,
    /// The descriptor URL (spec §3) — where the portable "you" is fetched from.
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default)]
    pub exp: Option<u64>,
}

/// The fetchable descriptor (passport-v0.1 §3) — fetched fresh, rendered,
/// discarded. Never persisted beyond the session.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Descriptor {
    #[serde(default)]
    pub passport: String,
    #[serde(default)]
    pub identity: DescriptorIdentity,
    #[serde(default)]
    pub avatar: Option<AvatarDescriptor>,
    #[serde(default)]
    pub home: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DescriptorIdentity {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
}

/// avatar-v0.1 envelope — the portable "you" (spec §3 + avatar-v0.1).
///
/// The heart is `spec`: the **shared [`AvatarSpec`]** (slot → partId +
/// bodyHeight) — the exact same model `com.pixygon.avatar` (Unity) and
/// `@pixygon/avatar` (web) persist, so one saved avatar renders identically in
/// a Unity game, the web Studio, and any Thread browser. Parts resolve through
/// the Portable Item Convention (`infinite_avatar::manifest`): partId →
/// `AvatarAsset` doc → one GLB carrying its own meaning.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AvatarDescriptor {
    /// The shared avatar model. Absent → render the placeholder traveler.
    #[serde(default)]
    pub spec: Option<AvatarSpec>,
    #[serde(default)]
    pub colors: HashMap<String, String>,
    /// Catalog/CDN base for partId → GLB resolution.
    #[serde(default)]
    pub assets: Option<String>,
    /// Legacy draft envelope (pre avatar-v0.1) — still parsed so early issuers
    /// keep rendering. `spec` wins when both are present.
    #[serde(default)]
    pub base: Option<u32>,
    #[serde(default)]
    pub worn: Vec<u32>,
}

/// The traveler's held Passport: the token, its claims, and (once fetched)
/// the descriptor. Fetching runs on a background thread; call [`Passport::poll`]
/// once per frame, like the other Loom clients.
pub struct Passport {
    token: String,
    claims: Claims,
    descriptor: Option<Descriptor>,
    pending: Option<Receiver<Result<Descriptor, String>>>,
    error: Option<String>,
}

impl Passport {
    /// Hold a token. Claims decode best-effort — an opaque (non-JWT) dev token
    /// still carries, it just has no readable claims. Empty tokens are `None`:
    /// no passport is a valid, anonymous way to walk.
    pub fn from_token(token: impl Into<String>) -> Option<Self> {
        let token = token.into();
        if token.trim().is_empty() {
            return None;
        }
        let claims = decode_claims(&token).unwrap_or_default();
        Some(Self {
            token,
            claims,
            descriptor: None,
            pending: None,
            error: None,
        })
    }

    /// The traveler's passport from `INFINITE_PASSPORT` (unset/empty → anonymous).
    pub fn from_env() -> Option<Self> {
        Self::from_token(std::env::var("INFINITE_PASSPORT").unwrap_or_default())
    }

    /// The raw token, as presented on presence `join` / portal handoff.
    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn claims(&self) -> &Claims {
        &self.claims
    }

    /// The stable identity, if the token's claims were readable.
    pub fn sub(&self) -> Option<&str> {
        (!self.claims.sub.is_empty()).then_some(self.claims.sub.as_str())
    }

    /// Best display name: descriptor (freshest) → token claim → the sub itself.
    pub fn display_name(&self) -> Option<&str> {
        if let Some(d) = &self.descriptor {
            if !d.identity.name.is_empty() {
                return Some(&d.identity.name);
            }
        }
        if let Some(n) = &self.claims.name {
            if !n.is_empty() {
                return Some(n);
            }
        }
        self.sub()
    }

    /// The fetched descriptor, once [`Passport::poll`] has collected it.
    pub fn descriptor(&self) -> Option<&Descriptor> {
        self.descriptor.as_ref()
    }

    /// The traveler's own [`AvatarSpec`] (the shared avatar model), once the
    /// descriptor has landed and if the issuer provided one.
    pub fn avatar_spec(&self) -> Option<&AvatarSpec> {
        self.descriptor.as_ref()?.avatar.as_ref()?.spec.as_ref()
    }

    /// Why the last descriptor fetch failed, if it did (e.g. `410 Gone` after
    /// erasure — spec §6 — in which case the traveler renders anonymous).
    pub fn descriptor_error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Start fetching the descriptor from the token's `avatar` claim on a
    /// background thread. No-op without a claim URL, while a fetch is in
    /// flight, or once loaded (the descriptor is session-cached; spec §3).
    pub fn fetch_descriptor(&mut self) {
        if self.pending.is_some() || self.descriptor.is_some() {
            return;
        }
        let Some(url) = self.claims.avatar.clone() else {
            return;
        };
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(fetch_blocking(&url));
        });
        self.pending = Some(rx);
    }

    /// Collect a finished background fetch, if any. Call once per frame.
    /// Returns `true` on the frame the descriptor arrives.
    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.pending else {
            return false;
        };
        match rx.try_recv() {
            Ok(Ok(descriptor)) => {
                self.descriptor = Some(descriptor);
                self.error = None;
                self.pending = None;
                true
            }
            Ok(Err(msg)) => {
                self.error = Some(msg);
                self.pending = None;
                false
            }
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => {
                self.error = Some("fetch thread stopped".into());
                self.pending = None;
                false
            }
        }
    }
}

/// Fetches co-travelers' descriptors by the `avatar` URL the relay announced
/// (welcome occupants / join broadcast), so the browser renders "them" from
/// *their* Passport. Session-scoped and fetch-render-discard, per spec §3:
/// nothing here outlives the run. Keyed by occupant id.
#[derive(Default)]
pub struct DescriptorPool {
    pending: Vec<(u32, Receiver<Result<Descriptor, String>>)>,
    /// Ids already fetched (or failed) this session — no refetch churn.
    done: std::collections::HashSet<u32>,
}

impl DescriptorPool {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start fetching `id`'s descriptor from `url` on a background thread.
    /// No-op if this occupant was already fetched or is in flight.
    pub fn fetch(&mut self, id: u32, url: &str) {
        if self.done.contains(&id) || self.pending.iter().any(|(p, _)| *p == id) {
            return;
        }
        let url = url.to_string();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(fetch_blocking(&url));
        });
        self.pending.push((id, rx));
    }

    /// Collect descriptors that arrived since last frame. Failures are dropped
    /// silently — the traveler simply keeps their placeholder look.
    pub fn poll(&mut self) -> Vec<(u32, Descriptor)> {
        let mut arrived = Vec::new();
        self.pending.retain(|(id, rx)| match rx.try_recv() {
            Ok(Ok(d)) => {
                arrived.push((*id, d));
                false
            }
            Ok(Err(_)) | Err(TryRecvError::Disconnected) => false,
            Err(TryRecvError::Empty) => true,
        });
        for (id, _) in &arrived {
            self.done.insert(*id);
        }
        arrived
    }

    /// Forget a traveler (they left) so a rejoin fetches fresh.
    pub fn forget(&mut self, id: u32) {
        self.done.remove(&id);
        self.pending.retain(|(p, _)| *p != id);
    }
}

/// Parse a descriptor colour (`"#rrggbb"`) into linear-ish RGBA for rendering.
pub fn parse_color(hex: &str) -> Option<[f32; 4]> {
    let h = hex.strip_prefix('#')?;
    if h.len() != 6 {
        return None;
    }
    let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some([
        f32::from(byte(0)?) / 255.0,
        f32::from(byte(2)?) / 255.0,
        f32::from(byte(4)?) / 255.0,
        1.0,
    ])
}

/// Decode a JWT's payload segment without verifying — the holder reading its
/// own claims. Returns `None` for opaque (non-JWT) tokens.
fn decode_claims(token: &str) -> Option<Claims> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64url_decode(payload)?;
    serde_json::from_slice(&bytes).ok()
}

/// Minimal base64url (RFC 4648 §5, padding optional) — just enough to read a
/// JWT payload without pulling a dependency into the lean browser build.
fn base64url_decode(s: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some(u32::from(c - b'A')),
            b'a'..=b'z' => Some(u32::from(c - b'a') + 26),
            b'0'..=b'9' => Some(u32::from(c - b'0') + 52),
            b'-' => Some(62),
            b'_' => Some(63),
            _ => None,
        }
    }
    let s = s.trim_end_matches('=');
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut buf, mut bits) = (0u32, 0u32);
    for &c in s.as_bytes() {
        buf = (buf << 6) | val(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Some(out)
}

/// Blocking fetch on a throwaway current-thread runtime (reqwest is async-only
/// in this workspace). Runs on a background thread, so blocking is fine here.
fn fetch_blocking(url: &str) -> Result<Descriptor, String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    rt.block_on(async {
        let resp = reqwest::get(url).await.map_err(|e| e.to_string())?;
        let status = resp.status();
        if status.as_u16() == 410 {
            // Erasure (spec §6): the identity went dark. Render anonymous.
            return Err("passport erased (410 Gone)".into());
        }
        if !status.is_success() {
            return Err(format!("HTTP {}", status.as_u16()));
        }
        let text = resp.text().await.map_err(|e| e.to_string())?;
        serde_json::from_str(&text).map_err(|e| e.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build an unsigned JWT-shaped token around a JSON payload.
    fn token_with_payload(payload: &str) -> String {
        fn b64url(data: &[u8]) -> String {
            const ALPHABET: &[u8] =
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
            let mut out = String::new();
            for chunk in data.chunks(3) {
                let mut buf = [0u8; 3];
                buf[..chunk.len()].copy_from_slice(chunk);
                let n = (u32::from(buf[0]) << 16) | (u32::from(buf[1]) << 8) | u32::from(buf[2]);
                let chars = [(n >> 18) & 63, (n >> 12) & 63, (n >> 6) & 63, n & 63];
                for (i, &c) in chars.iter().enumerate() {
                    if i <= chunk.len() {
                        out.push(ALPHABET[c as usize] as char);
                    }
                }
            }
            out
        }
        format!(
            "{}.{}.sig",
            b64url(br#"{"alg":"none"}"#),
            b64url(payload.as_bytes())
        )
    }

    /// The avatar-v0.1 descriptor envelope carries the SHARED `AvatarSpec` —
    /// the same JSON the Unity package and the web Studio persist.
    #[test]
    fn descriptor_carries_the_shared_avatar_spec() {
        let d: Descriptor = serde_json::from_str(
            r##"{
              "passport": "0.1",
              "identity": { "id": "did:pixygon:abc", "name": "Anders" },
              "avatar": {
                "spec": { "bodyHeight": 0.7, "parts": { "Body": 11010001, "Jacket": 123456 } },
                "colors": { "skin": "#c8956c" },
                "assets": "https://api.pixygon.io/v1/avatar/assets"
              }
            }"##,
        )
        .unwrap();
        let avatar = d.avatar.expect("avatar envelope");
        let spec = avatar.spec.expect("shared spec");
        assert_eq!(spec.body_height, 0.7);
        assert_eq!(spec.get(infinite_avatar::AvatarSlot::Jacket), 123456);
        assert_eq!(spec.get(infinite_avatar::AvatarSlot::Body), 11010001);
        assert_eq!(
            avatar.assets.as_deref(),
            Some("https://api.pixygon.io/v1/avatar/assets")
        );
        // The legacy draft envelope still parses (spec absent, base/worn kept).
        let legacy: Descriptor =
            serde_json::from_str(r#"{ "avatar": { "base": 11010001, "worn": [21050001] } }"#)
                .unwrap();
        let a = legacy.avatar.unwrap();
        assert!(a.spec.is_none());
        assert_eq!(a.base, Some(11010001));
    }

    #[test]
    fn decodes_the_spec_claims() {
        let token = token_with_payload(
            r#"{"iss":"https://api.pixygon.io/v1/passport",
                "sub":"did:pixygon:6981e8ed",
                "name":"Anders",
                "avatar":"https://api.pixygon.io/v1/passport/avatar/did:pixygon:6981e8ed",
                "scopes":["presence","commerce"],
                "iat":1789000000,"exp":1789086400}"#,
        );
        let p = Passport::from_token(token).unwrap();
        assert_eq!(p.sub(), Some("did:pixygon:6981e8ed"));
        assert_eq!(p.display_name(), Some("Anders"));
        assert_eq!(p.claims().scopes, ["presence", "commerce"]);
        assert_eq!(
            p.claims().avatar.as_deref(),
            Some("https://api.pixygon.io/v1/passport/avatar/did:pixygon:6981e8ed")
        );
        assert_eq!(p.claims().exp, Some(1789086400));
    }

    #[test]
    fn opaque_tokens_still_carry() {
        // A dev relay running open accepts any string — the browser must too.
        let p = Passport::from_token("not-a-jwt").unwrap();
        assert_eq!(p.token(), "not-a-jwt");
        assert_eq!(p.sub(), None);
        assert_eq!(p.display_name(), None);
    }

    #[test]
    fn empty_token_is_anonymous() {
        assert!(Passport::from_token("").is_none());
        assert!(Passport::from_token("   ").is_none());
    }

    #[test]
    fn parses_the_descriptor_shape() {
        let json = r##"{
            "passport": "0.1",
            "identity": { "id": "did:pixygon:6981e8ed", "name": "Anders" },
            "avatar": {
                "base": 11010001,
                "worn": [21050001, 28010002],
                "colors": { "skin": "#c8956c", "hair": "#2a1f1a" },
                "assets": "https://cdn.pixygon.io/avatars/"
            },
            "consent": { "version": 3 },
            "home": "thread://pixygon.io/the-archive"
        }"##;
        let d: Descriptor = serde_json::from_str(json).unwrap();
        assert_eq!(d.identity.name, "Anders");
        let a = d.avatar.unwrap();
        assert_eq!(a.base, Some(11010001));
        assert_eq!(a.worn, [21050001, 28010002]);
        assert_eq!(a.colors.get("skin").map(String::as_str), Some("#c8956c"));
        assert_eq!(d.home.as_deref(), Some("thread://pixygon.io/the-archive"));
    }

    #[test]
    fn descriptor_name_wins_over_claim() {
        let token = token_with_payload(r#"{"sub":"did:x:1","name":"OldName"}"#);
        let mut p = Passport::from_token(token).unwrap();
        assert_eq!(p.display_name(), Some("OldName"));
        p.descriptor = Some(Descriptor {
            identity: DescriptorIdentity {
                id: "did:x:1".into(),
                name: "Fresh".into(),
            },
            ..Default::default()
        });
        // The descriptor is fetched fresh each session — it is the later word.
        assert_eq!(p.display_name(), Some("Fresh"));
    }

    #[test]
    fn parses_descriptor_colors() {
        let c = parse_color("#c8956c").unwrap();
        assert!((c[0] - 200.0 / 255.0).abs() < 1e-6);
        assert!((c[1] - 149.0 / 255.0).abs() < 1e-6);
        assert!((c[2] - 108.0 / 255.0).abs() < 1e-6);
        assert_eq!(c[3], 1.0);
        assert!(parse_color("c8956c").is_none(), "requires the # prefix");
        assert!(parse_color("#fff").is_none(), "short form unsupported");
        assert!(parse_color("#zzzzzz").is_none());
    }

    #[test]
    fn base64url_handles_padding_and_url_chars() {
        assert_eq!(
            base64url_decode("aGVsbG8").as_deref(),
            Some(b"hello".as_slice())
        );
        assert_eq!(
            base64url_decode("aGVsbG8=").as_deref(),
            Some(b"hello".as_slice())
        );
        // '-' and '_' are the url-safe 62/63.
        assert_eq!(
            base64url_decode("-_8").as_deref(),
            Some([0xfb, 0xff].as_slice())
        );
        assert!(base64url_decode("not base64!").is_none());
    }
}
