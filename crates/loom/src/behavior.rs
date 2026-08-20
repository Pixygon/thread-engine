//! **Behavior ABI** — the sandboxed interactivity layer (behavior-abi-v0.1).
//!
//! A world's WASM behavior module receives events (a player interacts, a tick) and
//! returns **declarative [`Action`]s** — intents the *browser* performs. The module
//! never touches IO: it can't open a URL, buy something, or veilwalk directly; it
//! asks the host to, so the sandbox stays pure and the host stays authoritative.
//! That's the whole safety model, and it's what lets a world be interactive without
//! a browser trusting its code.
//!
//! This module is the transport-agnostic core: the [`Action`]/[`InteractEvent`]
//! types (the wire between module and host) and the [`Behavior`] seam. The actual
//! WASM execution is a feature-gated binding (`behaviors`, see
//! [`crate::behavior_wasm`]); tests here use a plain-Rust `Behavior`, so the ABI and
//! the dispatch model are verified with no WASM runtime.

use serde::{Deserialize, Serialize};

/// A declarative effect a behavior asks the host to perform (module → host). The
/// host — never the module — carries it out, so the sandbox never touches IO.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action")]
pub enum Action {
    /// Open a canonical Codex entry panel.
    #[serde(rename = "codex.open")]
    CodexOpen { slug: String },
    /// Buy an item. `price_ref` is a *reference*, never authoritative — the server
    /// sets the real price (matches server-side pricing). Replies `purchase_result`.
    #[serde(rename = "commerce.buy")]
    CommerceBuy {
        item: String,
        #[serde(default)]
        price_ref: serde_json::Value,
    },
    /// Veilwalk to a Locator.
    #[serde(rename = "navigate")]
    Navigate { to: String },
    /// A transient UI toast.
    #[serde(rename = "notify")]
    Notify {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        level: Option<String>,
    },
    /// Relay an interaction to co-present travelers.
    #[serde(rename = "presence.emit")]
    PresenceEmit {
        event: String,
        #[serde(default)]
        data: serde_json::Value,
    },
    /// Mutate visible placement state (colour, visibility, transform).
    #[serde(rename = "set_state")]
    SetState {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        placement: Option<String>,
        patch: serde_json::Value,
    },
    /// Put an item in the visitor's inventory. `item` is a material/item number
    /// (the `NNNN` of a `StructuredId`). Emitted by declarative interactions.
    #[serde(rename = "give_item")]
    GiveItem { item: u16, count: u32 },
    /// Remove a placed object from the world (mesh + collider) at `position` — a
    /// declarative `despawn`. The browser hides the nearest rendered instance.
    #[serde(rename = "despawn")]
    Despawn { position: [f32; 3] },
    /// Conjure a new object into the world: a builtin mesh (`cube`/`sphere`/…)
    /// at `position`, uniformly scaled, colored, optionally **named** — a named
    /// spawn is addressable by later `set_state` (move/tint) calls. The browser
    /// caps runaway spawners; behaviors conjure decor, not geometry floods.
    #[serde(rename = "spawn")]
    Spawn {
        builtin: String,
        position: [f32; 3],
        #[serde(default = "one_f32")]
        scale: f32,
        #[serde(default = "white_rgb")]
        color: [f32; 3],
        #[serde(default)]
        name: String,
        /// Declarative idle motion for the conjured object: `"spin"`, `"bob"`,
        /// or empty for static — the same vocabulary as manifest `animate`.
        #[serde(default)]
        animate: String,
        #[serde(default = "one_f32")]
        anim_speed: f32,
        #[serde(default = "quarter_f32")]
        anim_amp: f32,
    },
}

fn quarter_f32() -> f32 {
    0.25
}

fn one_f32() -> f32 {
    1.0
}
fn white_rgb() -> [f32; 3] {
    [1.0, 1.0, 1.0]
}

/// The return of `thread_on_interact` — the effects a behavior wants performed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ActionList {
    #[serde(default)]
    pub actions: Vec<Action>,
}

/// Who triggered an event (the interacting player), as far as a behavior may know.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Actor {
    /// The interacting Passport subject, if the player is identified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passport_sub: Option<String>,
}

/// `host → module` when a player interacts with a bound placement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractEvent {
    /// The placement name that was interacted with.
    pub placement: String,
    #[serde(default)]
    pub actor: Actor,
    /// The world's id.
    pub world: String,
    /// The placement's manifest `data` block (e.g. `{item, price, currency}`).
    #[serde(default)]
    pub data: serde_json::Value,
}

/// One instantiated behavior module (a WASM instance, or a plain-Rust one in tests).
/// The browser owns a [`Behavior`] per manifest `behavior` id and dispatches events
/// to it, then performs the [`Action`]s it returns.
pub trait Behavior {
    /// Called once after instantiation.
    fn on_load(&mut self) {}
    /// A player interacted with a bound placement → the effects to perform.
    fn on_interact(&mut self, event: &InteractEvent) -> Vec<Action>;
    /// A per-frame tick, if the behavior declared `"tick"` in its `on[]`.
    fn on_tick(&mut self, _dt_ms: i32) {}
    /// An asynchronous reply (`purchase_result`, `codex_ready`, presence, …).
    fn on_event(&mut self, _event: &serde_json::Value) -> Vec<Action> {
        Vec::new()
    }
}

/// Parse an `ActionList` from a behavior's JSON return (empty on malformed — a
/// misbehaving module must never break the world; the super-stable tenet).
pub fn parse_actions(json: &str) -> Vec<Action> {
    serde_json::from_str::<ActionList>(json)
        .map(|a| a.actions)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn actions_use_the_dotted_wire_names() {
        let a = Action::CodexOpen {
            slug: "the-thread".into(),
        };
        assert_eq!(
            serde_json::to_value(&a).unwrap(),
            json!({"action": "codex.open", "slug": "the-thread"})
        );
        let n = Action::Navigate {
            to: "thread://pixygon.io".into(),
        };
        assert_eq!(serde_json::to_value(&n).unwrap()["action"], "navigate");
    }

    #[test]
    fn action_list_round_trips() {
        let list = ActionList {
            actions: vec![
                Action::Notify {
                    text: "Sold!".into(),
                    level: Some("info".into()),
                },
                Action::SetState {
                    placement: Some("shelf".into()),
                    patch: json!({"sold": true}),
                },
            ],
        };
        let json = serde_json::to_string(&list).unwrap();
        assert_eq!(parse_actions(&json), list.actions);
    }

    #[test]
    fn malformed_returns_no_actions_not_an_error() {
        assert!(parse_actions("not json").is_empty());
        assert!(parse_actions("{}").is_empty());
    }

    /// The two reference behaviors (§7) implemented in plain Rust — proves the ABI +
    /// dispatch model with no WASM runtime.
    struct CodexViewer;
    impl Behavior for CodexViewer {
        fn on_interact(&mut self, e: &InteractEvent) -> Vec<Action> {
            match e.data.get("slug").and_then(|s| s.as_str()) {
                Some(slug) => vec![Action::CodexOpen {
                    slug: slug.to_string(),
                }],
                None => vec![],
            }
        }
    }

    #[test]
    fn codex_viewer_reference_behavior() {
        let mut b = CodexViewer;
        let ev = InteractEvent {
            placement: "pedestal".into(),
            actor: Actor::default(),
            world: "codex-archive".into(),
            data: json!({ "slug": "the-thread" }),
        };
        assert_eq!(
            b.on_interact(&ev),
            vec![Action::CodexOpen {
                slug: "the-thread".into()
            }]
        );
    }
}
