//! In-world commerce — the "buy" side of the Thread.
//!
//! A world's `buy` behavior returns [`Action::CommerceBuy`](crate::Action); the
//! *browser* performs it here. The flow is deliberately server-authoritative
//! (Theme B): the client sends only `{ worldId, itemStructuredId, priceRef }` to
//! `POST <base>/thread/purchase` with the traveler's Passport as the bearer —
//! the server resolves the price from its own catalog (client-set prices are
//! dead on arrival), records the charge idempotently, and returns a grant.
//!
//! Entitlements — "what does this traveler already hold?" — come from
//! `GET <base>/entitlements/:projectId` with the same bearer.
//!
//! All network runs on background threads and lands via [`CommerceClient::poll`]
//! once per frame; failures degrade to a message, never a stall (super-stable
//! tenet). The base URL comes from `INFINITE_COMMERCE_BASE` (default
//! `https://api.pixygon.io/v1` — same origin as the Codex).

use std::sync::mpsc::{channel, Receiver, TryRecvError};

use serde::Deserialize;
use serde_json::Value;

/// What's on the counter — everything the purchase confirm dialog shows and
/// everything the wire call needs. Built from the behavior's `commerce.buy`
/// action plus the placement's `data` block via [`offer_from`].
#[derive(Debug, Clone, PartialEq)]
pub struct Offer {
    /// The item being bought, as a StructuredId string ("CCSSNNNN").
    pub item: String,
    /// Display name (from placement `data.name`, falling back to the item id).
    pub name: String,
    /// The world's *displayed* price ("250 gold"), informative only — the
    /// server resolves the authoritative charge from `price_ref`.
    pub price_label: Option<String>,
    /// The price-catalog key the server resolves the charge from.
    pub price_ref: String,
    /// The world the stall stands in (cat-45 world id).
    pub world_id: String,
}

/// Build an [`Offer`] from a `commerce.buy` action + the focused placement's
/// `data` block. `price_ref` resolution order: the action's value (string, or
/// an object's `priceRef`/`productKey`/`key`) → `data.priceRef` → the item id.
pub fn offer_from(item: &str, price_ref: &Value, data: &Value, world_id: &str) -> Offer {
    let name = data
        .get("name")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(item)
        .to_string();
    let price_label =
        data.get("price")
            .map(|p| match data.get("currency").and_then(Value::as_str) {
                Some(c) => format!("{p} {c}"),
                None => p.to_string(),
            });
    let price_ref = price_ref_string(price_ref)
        .or_else(|| {
            data.get("priceRef")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| item.to_string());
    Offer {
        item: item.to_string(),
        name,
        price_label,
        price_ref,
        world_id: world_id.to_string(),
    }
}

/// Read a priceRef out of the action's value: a plain string, or an object
/// carrying one under a conventional key.
fn price_ref_string(v: &Value) -> Option<String> {
    if let Some(s) = v.as_str() {
        return (!s.is_empty()).then(|| s.to_string());
    }
    for key in ["priceRef", "productKey", "key", "ref"] {
        if let Some(s) = v.get(key).and_then(Value::as_str) {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

/// The settled purchase, shaped to `POST /thread/purchase` (tolerant to extras).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Receipt {
    #[serde(default)]
    pub grant: Grant,
    #[serde(default)]
    pub charge: Charge,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Grant {
    #[serde(default, rename = "itemStructuredId")]
    pub item: String,
    #[serde(default)]
    pub granted: bool,
    /// Whether the grant landed on the traveler's durable inventory yet (the
    /// server acknowledges before persistence is wired; surface honestly).
    #[serde(default)]
    pub persisted: bool,
    /// The idempotency reference for this (traveler, world, item) charge.
    #[serde(default, rename = "ref")]
    pub reference: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Charge {
    /// Major-unit amount the server actually recorded (its price, not ours).
    #[serde(default)]
    pub amount: f64,
    #[serde(default)]
    pub currency: String,
    #[serde(default)]
    pub recorded: bool,
    /// True when this (traveler, world, item) was already charged — the server
    /// deduplicates, so "buying again" is free and safe.
    #[serde(default)]
    pub duplicate: bool,
}

/// What the traveler already holds against a project, shaped to
/// `GET /entitlements/:projectId` (tolerant to extras).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Entitlement {
    #[serde(default)]
    pub plus: bool,
    #[serde(default)]
    pub tier: String,
    #[serde(default)]
    pub source: String,
    #[serde(default, rename = "ownsGame")]
    pub owns_game: bool,
    #[serde(default)]
    pub credits: f64,
}

/// The browser's commerce client — one purchase in flight at a time (buying is
/// a deliberate act, not a queue), plus an independent entitlements read.
pub struct CommerceClient {
    base: String,
    /// Passport bearer token — purchases REQUIRE one (the server charges an
    /// authenticated traveler). `None` = walking anonymous, browsing only.
    token: Option<String>,
    pending: Option<(Offer, Receiver<Result<Receipt, String>>)>,
    ent_pending: Option<Receiver<Result<Entitlement, String>>>,
}

impl CommerceClient {
    pub fn new(base: impl Into<String>) -> Self {
        Self {
            base: base.into(),
            token: None,
            pending: None,
            ent_pending: None,
        }
    }

    /// Read the base URL from `INFINITE_COMMERCE_BASE` (falls back to the public API).
    pub fn from_env() -> Self {
        let base = std::env::var("INFINITE_COMMERCE_BASE")
            .unwrap_or_else(|_| "https://api.pixygon.io/v1".to_string());
        Self::new(base)
    }

    /// Present a Passport on purchases (`Authorization: Bearer <token>`).
    /// `None`/empty → anonymous: browsing works, buying doesn't.
    pub fn with_token(mut self, token: Option<String>) -> Self {
        self.token = token.filter(|t| !t.trim().is_empty());
        self
    }

    /// Whether a Passport is presented — the purchase UI gates its Buy on this.
    pub fn has_token(&self) -> bool {
        self.token.is_some()
    }

    /// The purchase currently settling, if any.
    pub fn in_flight(&self) -> Option<&Offer> {
        self.pending.as_ref().map(|(o, _)| o)
    }

    /// Start a purchase on a background thread. Returns `false` (and does
    /// nothing) without a Passport or while another purchase is settling.
    pub fn buy(&mut self, offer: Offer) -> bool {
        let Some(token) = self.token.clone() else {
            return false;
        };
        if self.pending.is_some() {
            return false;
        }
        let url = format!("{}/thread/purchase", self.base.trim_end_matches('/'));
        let body = serde_json::json!({
            "worldId": offer.world_id,
            "itemStructuredId": offer.item,
            "priceRef": offer.price_ref,
        });
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(post_blocking(&url, &token, &body));
        });
        self.pending = Some((offer, rx));
        true
    }

    /// Collect a settled purchase, if any. Call once per frame.
    pub fn poll(&mut self) -> Option<(Offer, Result<Receipt, String>)> {
        let (_, rx) = self.pending.as_ref()?;
        let result = match rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err("purchase thread stopped".into()),
        };
        let (offer, _) = self.pending.take()?;
        Some((offer, result))
    }

    /// Fetch what the traveler holds against a project (background thread).
    /// No-op without a Passport or while a previous read is in flight.
    pub fn entitlements(&mut self, project_id: &str) {
        let Some(token) = self.token.clone() else {
            return;
        };
        if self.ent_pending.is_some() || project_id.trim().is_empty() {
            return;
        }
        let url = format!(
            "{}/entitlements/{}",
            self.base.trim_end_matches('/'),
            project_id.trim()
        );
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(get_blocking(&url, &token));
        });
        self.ent_pending = Some(rx);
    }

    /// Collect a finished entitlements read, if any. Call once per frame.
    pub fn poll_entitlements(&mut self) -> Option<Result<Entitlement, String>> {
        let rx = self.ent_pending.as_ref()?;
        let result = match rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err("entitlements thread stopped".into()),
        };
        self.ent_pending = None;
        Some(result)
    }
}

/// The server's `{ "error": "..." }` body, when a call fails.
fn error_message(body: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct Err_ {
        error: String,
    }
    serde_json::from_str::<Err_>(body).ok().map(|e| e.error)
}

/// Blocking POST on a throwaway current-thread runtime (reqwest is async-only
/// in this workspace). Runs on a background thread, so blocking is fine here.
fn post_blocking(url: &str, token: &str, body: &Value) -> Result<Receipt, String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    rt.block_on(async {
        let resp = reqwest::Client::new()
            .post(url)
            .bearer_auth(token)
            .json(body)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| e.to_string())?;
        if !status.is_success() {
            return Err(error_message(&text).unwrap_or_else(|| format!("HTTP {}", status.as_u16())));
        }
        serde_json::from_str(&text).map_err(|e| e.to_string())
    })
}

/// Blocking GET with bearer, same runtime pattern as [`post_blocking`].
fn get_blocking(url: &str, token: &str) -> Result<Entitlement, String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    rt.block_on(async {
        let resp = reqwest::Client::new()
            .get(url)
            .bearer_auth(token)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| e.to_string())?;
        if !status.is_success() {
            return Err(error_message(&text).unwrap_or_else(|| format!("HTTP {}", status.as_u16())));
        }
        serde_json::from_str(&text).map_err(|e| e.to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offers_read_market_stall_data() {
        // The Bazaar's actual stall shape.
        let data = serde_json::json!({
            "item": "21010001", "name": "Veilwalker Blade", "price": 250, "currency": "gold"
        });
        let o = offer_from("21010001", &Value::Null, &data, "45010001");
        assert_eq!(o.name, "Veilwalker Blade");
        assert_eq!(o.price_label.as_deref(), Some("250 gold"));
        // No explicit priceRef anywhere → the item id is the catalog key.
        assert_eq!(o.price_ref, "21010001");
        assert_eq!(o.world_id, "45010001");
    }

    #[test]
    fn price_ref_resolution_order() {
        // The action's value wins…
        let o = offer_from(
            "21010001",
            &serde_json::json!("blade-sku"),
            &Value::Null,
            "w",
        );
        assert_eq!(o.price_ref, "blade-sku");
        // …including when it's an object carrying a conventional key…
        let o = offer_from(
            "21010001",
            &serde_json::json!({"productKey": "blade-pk"}),
            &Value::Null,
            "w",
        );
        assert_eq!(o.price_ref, "blade-pk");
        // …then the placement data's priceRef…
        let data = serde_json::json!({"priceRef": "from-data"});
        let o = offer_from("21010001", &Value::Null, &data, "w");
        assert_eq!(o.price_ref, "from-data");
        // …and a nameless placement falls back to the item id for display too.
        let o = offer_from("21010001", &Value::Null, &Value::Null, "w");
        assert_eq!(o.name, "21010001");
        assert!(o.price_label.is_none());
    }

    #[test]
    fn parses_the_purchase_response_shape() {
        // Verbatim shape from PixygonServer controllers/thread.js#purchase.
        let json = r#"{
            "grant": { "itemStructuredId": "21010001", "worldId": "45010001",
                       "userId": "u1", "granted": true, "persisted": false,
                       "ref": "thread:u1:45010001:21010001" },
            "charge": { "amount": 25, "currency": "NOK", "recorded": true, "duplicate": false }
        }"#;
        let r: Receipt = serde_json::from_str(json).unwrap();
        assert!(r.grant.granted);
        assert!(!r.grant.persisted);
        assert_eq!(r.grant.reference, "thread:u1:45010001:21010001");
        assert_eq!(r.charge.amount, 25.0);
        assert_eq!(r.charge.currency, "NOK");
        assert!(r.charge.recorded);
    }

    #[test]
    fn parses_the_entitlement_shape() {
        // Verbatim shape from PixygonServer services/entitlementService.js.
        let json = r#"{
            "projectId": "p1", "projectSlug": "infinite", "userId": "u1",
            "plus": true, "tier": "premium", "source": "subscription",
            "ownsGame": true, "credits": 120, "purchasedCredits": 100,
            "features": {}, "checkedAt": "2026-07-14T12:00:00Z"
        }"#;
        let e: Entitlement = serde_json::from_str(json).unwrap();
        assert!(e.plus);
        assert_eq!(e.tier, "premium");
        assert!(e.owns_game);
        assert_eq!(e.credits, 120.0);
    }

    #[test]
    fn surfaces_the_server_error_body() {
        assert_eq!(
            error_message(
                r#"{"error":"No server-side price for priceRef \"x\" in this world's project."}"#
            )
            .as_deref(),
            Some("No server-side price for priceRef \"x\" in this world's project.")
        );
        assert!(error_message("not json").is_none());
    }

    #[test]
    fn buying_requires_a_passport_and_one_at_a_time() {
        let offer = offer_from("21010001", &Value::Null, &Value::Null, "w");
        // Anonymous (or blank-token) travelers browse; they don't buy.
        let mut anon = CommerceClient::new("http://x").with_token(Some("  ".into()));
        assert!(!anon.has_token());
        assert!(!anon.buy(offer.clone()));
        assert!(anon.in_flight().is_none());
        // A second buy while one is settling is refused, not queued.
        let mut c = CommerceClient::new("http://127.0.0.1:9").with_token(Some("tok".into()));
        assert!(c.buy(offer.clone()));
        assert!(c.in_flight().is_some());
        assert!(!c.buy(offer));
    }

    #[test]
    fn entitlements_reads_need_a_token_and_a_project() {
        let mut anon = CommerceClient::new("http://x");
        anon.entitlements("p1");
        assert!(anon.poll_entitlements().is_none()); // never started
        let mut c = CommerceClient::new("http://x").with_token(Some("tok".into()));
        c.entitlements("  ");
        assert!(c.ent_pending.is_none()); // blank project → no call
    }
}
