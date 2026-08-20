//! **Veilwalking** — the navigation state machine of the Thread, renderer-agnostic.
//!
//! This is the part of a browser that decides *which world is active*, *when to
//! step through a veil*, and *what the player is standing in front of* — all pure
//! math on positions and Locators. It knows nothing about the GPU: to actually
//! place a resolved world into a renderer it calls back through the [`WorldLoader`]
//! seam, which the browser implements. That single trait is the engine/UI split —
//! it's what lets a *second* browser (a different renderer, an embedded view)
//! reuse veilwalking without touching this file.
//!
//! Stepping into a veil kicks off a background resolve+fetch (network-first, local
//! fallback — see [`crate::resolver`]); when the destination arrives it's handed to
//! the loader and the player is teleported to its spawn.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, TryRecvError};

use std::collections::HashMap;

use glam::{Quat, Vec3};
use infinite_manifest::{Interaction, InteractionEffect, WorldManifest, WorldRules};

use crate::assets::AssetSource;
use crate::behavior::Action;
#[cfg(feature = "behaviors")]
use crate::behavior::{Actor, Behavior, InteractEvent};
#[cfg(feature = "behaviors")]
use crate::behavior_wasm::WasmBehavior;
use crate::resolver::{self, TravelResult};

/// How close (metres) the player must be to a veil to step through.
pub const ENTER_RADIUS: f32 = 2.2;
/// How close the player must be to a pedestal to inspect it.
pub const INSPECT_RADIUS: f32 = 2.6;
/// Grace period after arriving before another veil can trigger (prevents bounce).
const TRAVERSE_COOLDOWN: f32 = 1.2;
/// Drop from the player's origin to their feet (capsule half-height-ish).
pub const FEET_DROP: f32 = 0.9;
/// How close (metres) the player must be to a veil before its destination is
/// fetched for a live preview — comfortably outside [`ENTER_RADIUS`] so the
/// signpost is up before you can step through.
pub const PREVIEW_RADIUS: f32 = 18.0;
/// Default Thread API base for the live resolver.
const DEFAULT_THREAD_BASE: &str = "https://api.pixygon.io/v1";

/// A veil in the active world — a hyperlink you step through (renderer-agnostic).
#[derive(Debug, Clone)]
pub struct PortalNav {
    pub id: String,
    pub position: Vec3,
    /// Destination Locator (`thread://…`).
    pub to: String,
    pub label: String,
    /// Whether the far side may be previewed (manifest `preview` policy;
    /// `none` opts a veil out — an opaque doorway).
    pub preview: bool,
    /// The doorway's facing (yaw about Y), for window parallax.
    #[allow(clippy::derive_partial_eq_without_eq)]
    pub yaw: f32,
}

/// An inspectable placement (a Codex pedestal, a web signboard, or a placement
/// with a bound behavior module).
#[derive(Debug, Clone, Default)]
pub struct InteractableNav {
    pub position: Vec3,
    pub label: String,
    pub codex: Option<String>,
    pub web: Option<String>,
    /// The placement's manifest `name` — the identity a behavior receives.
    pub name: String,
    /// Id of the behavior module bound to this placement, if any.
    pub behavior: Option<String>,
    /// A codeless declarative interaction (label + effects), if the placement
    /// declared one. The simple path — no WASM behavior needed.
    pub interaction: Option<Interaction>,
    /// Progress toward completing the declarative interaction (a tree takes a few
    /// chops). Resets to 0 once `interaction.hits` is reached and effects fire.
    pub hits_done: u32,
    /// The placement's manifest `data` block, passed through to its behavior.
    pub data: serde_json::Value,
}

/// A solid volume in the active world — placement geometry as an oriented box,
/// for a browser to hand to its physics so floors hold and walls block. Portals
/// never appear here (a veil is stepped *through*). Renderer-agnostic.
#[derive(Debug, Clone, Copy)]
pub struct ColliderNav {
    pub half_extents: Vec3,
    pub position: Vec3,
    pub rotation: Quat,
}

/// What's on the far side of a veil — its title, sky, and how many veils lead
/// onward. Fetched live as the player approaches (see [`Navigator::portal_preview`]).
#[derive(Debug, Clone)]
pub struct PortalPreview {
    pub title: String,
    pub sky: SkyNav,
    /// Number of veils in the destination world.
    pub veils: usize,
    /// The destination's raw manifest text — enough for a browser to render a
    /// **true window** into the far side (a snapshot from its spawn).
    pub manifest_text: String,
    /// Where the destination's relative assets resolve (local base + optional
    /// hosted base URL) — mirrors [`crate::resolver::TravelResult`].
    pub asset_base: PathBuf,
    pub asset_base_url: Option<String>,
    /// The far side's presence relays + room key — enough for a browser to
    /// listen at the threshold and show WHO IS THERE through the window.
    pub presence_relays: Vec<String>,
    pub world_id: String,
}

/// A simple gradient sky (zenith→horizon) + sun direction. Renderer-agnostic; the
/// browser maps this onto its own atmosphere.
#[derive(Debug, Clone, Copy)]
pub struct SkyNav {
    pub zenith: [f32; 3],
    pub horizon: [f32; 3],
    pub sun_dir: [f32; 3],
}

impl Default for SkyNav {
    fn default() -> Self {
        SkyNav {
            zenith: [0.05, 0.06, 0.12],
            horizon: [0.18, 0.16, 0.24],
            sun_dir: [0.3, 0.7, 0.2],
        }
    }
}

/// What a [`WorldLoader`] reports after placing a world into its renderer — the
/// navigation metadata the [`Navigator`] needs, with no GPU handles.
pub struct LoadedWorldMeta {
    pub title: String,
    /// The world's stable id (`world.id`) — the presence room key for bare relays.
    pub world_id: String,
    /// Where to teleport the player on arrival.
    pub spawn: Vec3,
    pub portals: Vec<PortalNav>,
    pub interactables: Vec<InteractableNav>,
    pub sky: SkyNav,
    /// This world's presence relays (from `presence.relay_list()`), primary first.
    /// Empty → the world is solo. The browser tries them in order with fallback.
    pub presence_relays: Vec<String>,
    /// Placement geometry as oriented boxes, for the browser's physics. A loader
    /// with no collision story (headless, text) just returns empty.
    pub colliders: Vec<ColliderNav>,
}

/// The rendering seam. A browser implements this to place a *validated* manifest
/// into its renderer and hand back the navigation metadata. The [`Navigator`]
/// owns everything else (resolution, arming, cooldown, focus).
pub trait WorldLoader {
    /// Place `manifest` (already parsed + validated) into the renderer, resolving
    /// its glTF/texture assets through `assets` (which fetches remote URLs from a
    /// cache), and return the navigation metadata. The `anchor` is where the player
    /// will stand (for e.g. a void floor).
    fn load(
        &mut self,
        manifest: &WorldManifest,
        assets: &AssetSource,
        anchor: Vec3,
    ) -> LoadedWorldMeta;
}

/// What the player is standing in front of (Codex pedestal and/or web signboard).
pub struct Focus {
    pub label: String,
    pub codex: Option<String>,
    pub web: Option<String>,
    /// Id of the behavior module bound here — `interact()` will dispatch to it.
    pub behavior: Option<String>,
    /// The creator-defined verb for a declarative interaction here ("Chop",
    /// "Mine"), if any — drives the "[E] …" prompt.
    pub interact_label: Option<String>,
    /// The placement's manifest `data` block — so a browser can act on a bound
    /// placement (e.g. a `buy` stall's item + price) even when the sandboxed
    /// module layer is compiled out (graceful degradation).
    pub data: serde_json::Value,
}

/// One instantiated behavior module for the active world.
#[cfg(feature = "behaviors")]
struct BehaviorSlot {
    /// The manifest behavior id placements bind to.
    id: String,
    /// Whether the manifest declared `"tick"` in this behavior's `on[]`.
    ticks: bool,
    module: WasmBehavior,
}

struct ActiveWorld {
    title: String,
    world_id: String,
    /// The world's one-line description (`world.description`) — browsers show it
    /// on the arrival card when a traveler steps through a veil.
    description: String,
    /// The Locator this world was reached by (`None` for a local file load).
    locator: Option<String>,
    /// The raw manifest text — the world's source, view-source-able.
    source: String,
    portals: Vec<PortalNav>,
    interactables: Vec<InteractableNav>,
    sky: SkyNav,
    presence_relays: Vec<String>,
    /// The room lives only while its owner hosts it (a home with guests) —
    /// when its presence connection dies, the browser walks guests home.
    owner_required: bool,
    colliders: Vec<ColliderNav>,
    /// Opt-in game mechanics this world enforces (default all off).
    rules: WorldRules,
    /// The world's sandboxed behavior modules (behavior-abi-v0.1).
    #[cfg(feature = "behaviors")]
    behaviors: Vec<BehaviorSlot>,
    /// The world's **Weft** behaviors (weft-v0.1) — the native path, always on.
    weft_behaviors: Vec<WeftSlot>,
}

/// One loaded Weft behavior, bound by its manifest id.
struct WeftSlot {
    id: String,
    behavior: crate::behavior_weft::WeftBehavior,
    /// The events this behavior subscribed to (`behaviors[].on`): a slot
    /// listing `"tick"` gets the world's heartbeat.
    on: Vec<String>,
}

/// Load every `weft`-kind behavior in the manifest: fetch the asset, parse,
/// **verify** (the trust boundary), reject on any failure — a broken module is
/// simply absent; the world renders on (super-stable tenet).
fn load_weft_behaviors(manifest: &WorldManifest, assets: &AssetSource) -> Vec<WeftSlot> {
    manifest
        .behaviors
        .iter()
        .filter_map(|b| {
            // Two native paths: a raw module asset (`weft`), or a **package
            // export** (`weft_pack` + `weft_export` — the markup's `weft-use`).
            // Both end at the same trust boundary: verify before evaluate.
            let result = if let Some(wid) = b.weft.as_ref() {
                let asset = manifest.assets.iter().find(|a| &a.id == wid)?;
                let path = assets.resolve(&asset.uri)?;
                let text = std::fs::read_to_string(&path).ok()?;
                crate::behavior_weft::WeftBehavior::from_json(&text)
            } else if let (Some(pid), Some(export)) = (b.weft_pack.as_ref(), b.weft_export.as_ref())
            {
                let asset = manifest.assets.iter().find(|a| &a.id == pid)?;
                let path = assets.resolve(&asset.uri)?;
                let text = std::fs::read_to_string(&path).ok()?;
                crate::behavior_weft::WeftBehavior::from_package_json(&text, export)
            } else {
                return None;
            };
            match result {
                Ok(behavior) => {
                    tracing::info!("weft behavior '{}' verified + loaded", b.id);
                    Some(WeftSlot {
                        id: b.id.clone(),
                        behavior,
                        on: b.on.clone(),
                    })
                }
                Err(e) => {
                    tracing::warn!("weft behavior '{}' refused: {e}", b.id);
                    None
                }
            }
        })
        .collect()
}

/// A live preview of a veil's destination: in flight, arrived, or given up.
/// Failure is remembered so an unreachable host is asked exactly once a session.
enum PreviewSlot {
    Pending(Receiver<Option<PortalPreview>>),
    Ready(PortalPreview),
    Failed,
}

/// How a veilwalk relates to session history: a fresh navigation pushes the
/// departed world onto the back stack (and clears forward); back/forward move
/// between the stacks without forking history — exactly the web's model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NavKind {
    Normal,
    Back,
    Forward,
    /// Same world, fresh contents (e.g. build mode re-applying an edited home
    /// file) — history is untouched: a reload is not a navigation.
    Reload,
}

/// An in-flight veilwalk (a background resolve+fetch), plus where to anchor it.
struct Pending {
    rx: Receiver<Result<TravelResult, String>>,
    anchor: Vec3,
    /// The destination Locator (recorded into history on arrival).
    locator: String,
    kind: NavKind,
}

/// The veilwalking state machine: active world, in-flight travel, cooldown, arming.
pub struct Navigator {
    worlds_root: PathBuf,
    resolver_base: String,
    active: Option<ActiveWorld>,
    pending: Option<Pending>,
    cooldown: f32,
    /// A veil only fires when "armed" — set false after any traversal attempt and
    /// re-armed only once the player has stepped away from every veil. Prevents
    /// arrival-bounce and, on a failed veilwalk, spam-retry (walk out and back).
    armed: bool,
    /// Last veilwalk failure, surfaced in the HUD until the next attempt.
    last_error: Option<String>,
    /// Session history: worlds walked *from* (most recent last) / *back from*.
    back_stack: Vec<String>,
    forward_stack: Vec<String>,
    /// The session's home world — by default, wherever it opened.
    home: Option<String>,
    /// The traveler's Passport `sub`, exposed to behavior modules as
    /// `actor.passport_sub` — the id only, never the token (passport-v0.1 §4).
    actor_sub: Option<String>,
    /// Live previews of veil destinations, keyed by Locator. Session-cached —
    /// a destination's signpost is the same from every world that links it.
    previews: HashMap<String, PreviewSlot>,
    /// Accumulator gating Weft tick dispatch to a steady ~4 Hz — behaviors
    /// get a heartbeat, not a framerate (fuel discipline).
    weft_tick_accum: f32,
    /// The session's ground plane: every world anchors at THIS height, so a
    /// hundred veilwalks can never drift the traveler into the sky. Set from
    /// the first world's anchor; deliberately never derived from the player's
    /// live (possibly airborne) position again.
    base_y: Option<f32>,
}

impl Navigator {
    pub fn new(worlds_root: impl Into<PathBuf>) -> Self {
        let resolver_base = std::env::var("INFINITE_THREAD_BASE")
            .unwrap_or_else(|_| DEFAULT_THREAD_BASE.to_string());
        Self {
            worlds_root: worlds_root.into(),
            resolver_base,
            active: None,
            pending: None,
            cooldown: 0.0,
            armed: false,
            last_error: None,
            back_stack: Vec::new(),
            forward_stack: Vec::new(),
            home: None,
            actor_sub: None,
            previews: HashMap::new(),
            weft_tick_accum: 0.0,
            base_y: None,
        }
    }

    /// Name the traveler behind interactions: their Passport `sub` reaches
    /// behavior modules as `actor.passport_sub`. `None` → anonymous.
    pub fn set_actor(&mut self, sub: Option<String>) {
        self.actor_sub = sub.filter(|s| !s.trim().is_empty());
    }

    /// The most recent veilwalk failure (for a HUD notice), if any.
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    pub fn title(&self) -> Option<&str> {
        self.active.as_ref().map(|w| w.title.as_str())
    }

    /// The active world's stable id (its presence room key). Empty if none loaded.
    pub fn world_id(&self) -> &str {
        self.active
            .as_ref()
            .map(|w| w.world_id.as_str())
            .unwrap_or("")
    }

    /// The active world's Locator — the address bar. `None` before the first
    /// arrival or for a world loaded from a local file.
    pub fn locator(&self) -> Option<&str> {
        self.active.as_ref().and_then(|w| w.locator.as_deref())
    }

    /// The active world's raw manifest text — **view-source**, the Thread's
    /// learn-by-copying primitive. `None` when no world is loaded.
    pub fn world_source(&self) -> Option<&str> {
        self.active.as_ref().map(|w| w.source.as_str())
    }

    /// The active world's one-line description (may be empty) — for arrival cards.
    pub fn world_description(&self) -> Option<&str> {
        self.active.as_ref().map(|w| w.description.as_str())
    }

    /// The session's home world Locator — by default, wherever the session opened.
    pub fn home(&self) -> Option<&str> {
        self.home.as_deref()
    }

    /// Override the home world (a browser preference).
    pub fn set_home(&mut self, locator: impl Into<String>) {
        self.home = Some(locator.into());
    }

    /// The back-history Locators (most recent last). For a HUD/history panel.
    pub fn back_history(&self) -> &[String] {
        &self.back_stack
    }

    /// Whether `back()` / `forward()` would go anywhere.
    pub fn can_go_back(&self) -> bool {
        !self.back_stack.is_empty()
    }
    pub fn can_go_forward(&self) -> bool {
        !self.forward_stack.is_empty()
    }

    /// Veilwalk back to the previous world (the web's back button). Returns
    /// whether a walk began — false when idle history is empty or travel is
    /// already in flight.
    pub fn back(&mut self, anchor: Vec3) -> bool {
        if self.pending.is_some() {
            return false;
        }
        let Some(dest) = self.back_stack.pop() else {
            return false;
        };
        self.begin_travel_kind(dest, anchor, NavKind::Back);
        true
    }

    /// Veilwalk forward again after `back()` (the web's forward button).
    pub fn forward(&mut self, anchor: Vec3) -> bool {
        if self.pending.is_some() {
            return false;
        }
        let Some(dest) = self.forward_stack.pop() else {
            return false;
        };
        self.begin_travel_kind(dest, anchor, NavKind::Forward);
        true
    }

    /// Veilwalk to the session's home world (a normal navigation — it forks
    /// history like any other walk). Returns whether a walk began.
    pub fn go_home(&mut self, anchor: Vec3) -> bool {
        if self.pending.is_some() {
            return false;
        }
        let Some(dest) = self.home.clone() else {
            return false;
        };
        if self.locator() == Some(dest.as_str()) {
            return false; // already home
        }
        self.begin_travel(dest, anchor);
        true
    }

    /// The active world's sky/atmosphere, if a world is loaded.
    pub fn sky(&self) -> Option<SkyNav> {
        self.active.as_ref().map(|w| w.sky)
    }

    /// The opt-in game mechanics the active world enforces. Default all off (no
    /// world, home, or a place that declares no rules) — the browser stays a browser.
    pub fn rules(&self) -> WorldRules {
        self.active.as_ref().map(|w| w.rules).unwrap_or_default()
    }

    /// The active world's presence relays (primary first, then fallbacks). Empty
    /// when no world is loaded or the world is solo.
    pub fn presence_relays(&self) -> &[String] {
        self.active
            .as_ref()
            .map(|w| w.presence_relays.as_slice())
            .unwrap_or(&[])
    }

    /// Whether the active world's room is owner-tied (`presence.owner_required`):
    /// when such a room closes, the browser SHOULD walk a guest back home.
    pub fn presence_owner_required(&self) -> bool {
        self.active.as_ref().is_some_and(|w| w.owner_required)
    }

    /// Whether a veilwalk is in flight (destination being resolved/fetched).
    pub fn is_traveling(&self) -> bool {
        self.pending.is_some()
    }

    /// The active world's solid volumes — placement geometry as oriented boxes
    /// for the browser's physics. Empty when no world is loaded.
    pub fn colliders(&self) -> &[ColliderNav] {
        self.active
            .as_ref()
            .map(|w| w.colliders.as_slice())
            .unwrap_or(&[])
    }

    /// The live preview of a veil's destination, once its fetch has landed —
    /// keyed by the portal's `to` Locator. `None` while in flight, after a
    /// failed fetch, or for a veil that opted out (`preview: "none"`).
    pub fn portal_preview(&self, to: &str) -> Option<&PortalPreview> {
        match self.previews.get(to) {
            Some(PreviewSlot::Ready(p)) => Some(p),
            _ => None,
        }
    }

    /// The veils in the active world — for a browser to render as doorways, draw
    /// on a minimap, or (headless) walk toward. Empty when no world is loaded.
    pub fn portals(&self) -> &[PortalNav] {
        self.active
            .as_ref()
            .map(|w| w.portals.as_slice())
            .unwrap_or(&[])
    }

    /// Whether the navigator is idle and **armed** — no travel in flight, off the
    /// arrival cooldown, and the player has stepped clear of every veil since the
    /// last traversal. When true, stepping into a veil will fire immediately; a
    /// browser can use this to show a "▶ step through" prompt only when it'll work.
    pub fn armed(&self) -> bool {
        self.armed && self.pending.is_none() && self.cooldown <= 0.0
    }

    /// The veil the player is standing in front of, as `(label, destination)`.
    pub fn near_portal(&self, player_pos: Vec3) -> Option<(&str, &str)> {
        let w = self.active.as_ref()?;
        let i = nearest_portal(&w.portals, player_pos, ENTER_RADIUS)?;
        Some((w.portals[i].label.as_str(), w.portals[i].to.as_str()))
    }

    /// The inspectable placement the player is standing at (Codex pedestal or web
    /// signboard), as owned data for the HUD + inspect action.
    pub fn focused(&self, player_pos: Vec3) -> Option<Focus> {
        let w = self.active.as_ref()?;
        let i = nearest_interactable(&w.interactables, player_pos, INSPECT_RADIUS)?;
        let it = &w.interactables[i];
        Some(Focus {
            label: it.label.clone(),
            codex: it.codex.clone(),
            web: it.web.clone(),
            behavior: it.behavior.clone(),
            interact_label: it.interaction.as_ref().map(|i| i.label.clone()),
            data: it.data.clone(),
        })
    }

    /// The player interacted with whatever they're standing at: dispatch the event
    /// to the placement's bound behavior module and return the [`Action`]s it asks
    /// for — the *browser* performs them (the sandbox never touches IO). Empty when
    /// nothing is focused, nothing is bound, or the `behaviors` feature is off.
    pub fn interact(&mut self, player_pos: Vec3) -> Vec<Action> {
        let Some(w) = self.active.as_mut() else {
            return Vec::new();
        };
        let Some(i) = nearest_interactable(&w.interactables, player_pos, INSPECT_RADIUS) else {
            return Vec::new();
        };

        // Codeless declarative interaction — no WASM. Count the hit; once `hits`
        // presses land, fire the effects as browser Actions.
        if let Some(inter) = w.interactables[i].interaction.clone() {
            {
                let it = &mut w.interactables[i];
                it.hits_done += 1;
                if it.hits_done < inter.hits.max(1) {
                    return Vec::new(); // more hits to complete it
                }
                it.hits_done = 0;
            }
            let mut actions = effects_to_actions(&inter.effects);
            // `despawn` removes the object outright: drop its interactable + the
            // nearest world collider here, and tell the browser to hide its mesh.
            if inter
                .effects
                .iter()
                .any(|e| matches!(e, InteractionEffect::Despawn))
            {
                let pos = w.interactables[i].position;
                w.interactables.remove(i);
                if let Some(ci) = nearest_by(w.colliders.iter().map(|c| c.position), pos, 3.0) {
                    w.colliders.remove(ci);
                }
                actions.push(Action::Despawn {
                    position: pos.to_array(),
                });
            }
            return actions;
        }

        let it = &w.interactables[i];
        let Some(bid) = it.behavior.clone() else {
            return Vec::new();
        };
        // Native path first: a Weft behavior bound to this placement.
        if let Some(slot) = w.weft_behaviors.iter_mut().find(|s| s.id == bid) {
            let name = if it.name.is_empty() {
                it.label.clone()
            } else {
                it.name.clone()
            };
            let slug = it.codex.clone();
            return slot.behavior.on_interact(&name, slug.as_deref());
        }
        #[cfg(feature = "behaviors")]
        {
            // The placement's `codex` slug is mirrored into `data.slug` (when the
            // author didn't set one) so a viewer module doesn't need it duplicated.
            let mut data = it.data.clone();
            if let Some(slug) = &it.codex {
                if data.get("slug").is_none() {
                    if !data.is_object() {
                        data = serde_json::json!({});
                    }
                    if let Some(obj) = data.as_object_mut() {
                        obj.insert("slug".into(), slug.clone().into());
                    }
                }
            }
            let event = InteractEvent {
                placement: if it.name.is_empty() {
                    it.label.clone()
                } else {
                    it.name.clone()
                },
                actor: Actor {
                    passport_sub: self.actor_sub.clone(),
                },
                world: w.world_id.clone(),
                data,
            };
            if let Some(slot) = w.behaviors.iter_mut().find(|s| s.id == bid) {
                return slot.module.on_interact(&event);
            }
        }
        let _ = (bid, &self.actor_sub);
        Vec::new()
    }

    /// Deliver an async host event (e.g. a settled `purchase_result`) to every
    /// behavior module in the active world and return the [`Action`]s they ask
    /// for. Empty when no world is loaded or the `behaviors` feature is off.
    pub fn dispatch_event(&mut self, event: &serde_json::Value) -> Vec<Action> {
        #[cfg(feature = "behaviors")]
        if let Some(w) = self.active.as_mut() {
            return w
                .behaviors
                .iter_mut()
                .flat_map(|slot| slot.module.on_event(event))
                .collect();
        }
        let _ = event;
        Vec::new()
    }

    /// Advance the active world's ticking behavior modules (declared `"tick"` in
    /// their `on[]`) — **the world's heartbeat**. WASM modules tick every call;
    /// Weft modules tick at a steady ~4 Hz (fuel discipline) and their Actions
    /// are returned for the browser to perform — worlds that behave, not just
    /// react.
    pub fn tick_behaviors(&mut self, dt: f32) -> Vec<Action> {
        #[cfg(feature = "behaviors")]
        if let Some(w) = self.active.as_mut() {
            let dt_ms = (dt * 1000.0).round() as i32;
            for slot in w.behaviors.iter_mut().filter(|s| s.ticks) {
                slot.module.on_tick(dt_ms);
            }
        }
        const TICK_SECS: f32 = 0.25;
        self.weft_tick_accum += dt.max(0.0);
        if self.weft_tick_accum < TICK_SECS {
            return Vec::new();
        }
        let elapsed_ms = (self.weft_tick_accum * 1000.0) as i64;
        self.weft_tick_accum = 0.0;
        let Some(w) = self.active.as_mut() else {
            return Vec::new();
        };
        let mut actions = Vec::new();
        for slot in w
            .weft_behaviors
            .iter_mut()
            .filter(|s| s.on.iter().any(|e| e == "tick"))
        {
            actions.extend(slot.behavior.on_tick(elapsed_ms));
        }
        actions
    }

    /// Load a world from a local `world.json` (the startup path). Returns the
    /// spawn to teleport the player to.
    pub fn load_file(
        &mut self,
        loader: &mut dyn WorldLoader,
        path: &Path,
        anchor: Vec3,
    ) -> Option<Vec3> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("navigator: cannot read {}: {e}", path.display());
                return None;
            }
        };
        let base = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        self.apply_world(
            loader,
            &text,
            &AssetSource::local(base),
            anchor,
            None,
            NavKind::Normal,
        )
    }

    /// Re-apply a *local* world file in place — same world, fresh contents,
    /// keeping `locator` and touching no history. Build mode uses this after
    /// each edit to the home file: the scene rebuilds, but a reload is not a
    /// navigation, so back/forward don't fork.
    pub fn reload_file(
        &mut self,
        loader: &mut dyn WorldLoader,
        path: &Path,
        locator: Option<String>,
        anchor: Vec3,
    ) -> Option<Vec3> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("navigator: cannot reload {}: {e}", path.display());
                return None;
            }
        };
        let base = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        self.apply_world(
            loader,
            &text,
            &AssetSource::local(base),
            anchor,
            locator,
            NavKind::Reload,
        )
    }

    /// Turn resolved manifest text into the live world (validate here, place via
    /// the loader); returns the spawn. `locator`+`kind` record the arrival into
    /// session history.
    fn apply_world(
        &mut self,
        loader: &mut dyn WorldLoader,
        manifest_text: &str,
        assets: &AssetSource,
        anchor: Vec3,
        locator: Option<String>,
        kind: NavKind,
    ) -> Option<Vec3> {
        // `from_text` accepts both source forms — JSON and `.thread` markup — so
        // a host serving markup is as walkable as one serving JSON.
        let manifest = match WorldManifest::from_text(manifest_text) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("navigator: invalid world manifest: {e}");
                return None;
            }
        };
        let loaded = loader.load(&manifest, assets, anchor);
        tracing::info!(
            "veilwalked into '{}' — {} veils",
            loaded.title,
            loaded.portals.len(),
        );
        // Record the world we're leaving into history (the web's model: a fresh
        // navigation forks — clears forward; back/forward move between stacks).
        if let Some(prev) = self.active.as_ref().and_then(|w| w.locator.clone()) {
            match kind {
                NavKind::Normal => {
                    self.back_stack.push(prev);
                    self.forward_stack.clear();
                }
                NavKind::Back => self.forward_stack.push(prev),
                NavKind::Forward => self.back_stack.push(prev),
                NavKind::Reload => {}
            }
        }
        self.base_y.get_or_insert(anchor.y);
        // Arrive STANDING: the spawn is a feet/floor position; the player's
        // origin sits a capsule's drop above it (grounding settles the rest).
        let spawn = loaded.spawn + Vec3::new(0.0, FEET_DROP, 0.0);
        self.active = Some(ActiveWorld {
            title: loaded.title,
            world_id: loaded.world_id,
            description: manifest.world.description.clone(),
            locator,
            source: manifest_text.to_string(),
            portals: loaded.portals,
            interactables: loaded.interactables,
            sky: loaded.sky,
            presence_relays: loaded.presence_relays,
            owner_required: manifest.presence.as_ref().is_some_and(|p| p.owner_required),
            colliders: loaded.colliders,
            rules: manifest.environment.rules,
            #[cfg(feature = "behaviors")]
            behaviors: load_behaviors(&manifest, assets),
            weft_behaviors: load_weft_behaviors(&manifest, assets),
        });
        self.cooldown = TRAVERSE_COOLDOWN;
        Some(spawn)
    }

    /// Open a world by Locator (the startup entry to the live Thread). Kicks off
    /// the async resolve+fetch; the world loads on the next `update` poll. The
    /// first opened Locator becomes the session's home unless overridden.
    pub fn open(&mut self, locator: &str, anchor: Vec3) {
        tracing::info!("opening {locator} on the Thread…");
        if self.home.is_none() {
            self.home = Some(locator.to_string());
        }
        self.begin_travel(locator.to_string(), anchor);
    }

    /// Begin veilwalking to a Locator: resolve + fetch on a background thread.
    fn begin_travel(&mut self, locator: String, anchor: Vec3) {
        self.begin_travel_kind(locator, anchor, NavKind::Normal);
    }

    fn begin_travel_kind(&mut self, locator: String, anchor: Vec3, kind: NavKind) {
        // Anchor drift is how travelers end up standing on air: quantize every
        // walk's anchor to the session ground plane.
        let anchor = Vec3::new(anchor.x, self.base_y.unwrap_or(anchor.y), anchor.z);
        let (tx, rx) = channel();
        let base = self.resolver_base.clone();
        let root = self.worlds_root.clone();
        let loc = locator.clone();
        std::thread::spawn(move || {
            let result = resolver::fetch_world(&base, &root, &loc);
            // Download the world's remote assets into the cache here, off the main
            // thread, so the GPU load only ever reads local files.
            if let Ok(tr) = &result {
                if let Ok(manifest) = WorldManifest::from_text(&tr.manifest_text) {
                    AssetSource::hosted(&tr.asset_base, tr.asset_base_url.clone(), &tr.asset_base)
                        .prefetch(&manifest);
                }
            }
            let _ = tx.send(result);
        });
        self.pending = Some(Pending {
            rx,
            anchor,
            locator,
            kind,
        });
        self.cooldown = TRAVERSE_COOLDOWN;
        self.armed = false;
        self.last_error = None;
    }

    /// Advance live destination previews: land any in-flight fetch, and start
    /// one for the nearest unpreviewd veil the player has approached. One fetch
    /// at a time — calm, sequential, and an unreachable host can't fan out.
    fn update_previews(&mut self, player_pos: Vec3) {
        for slot in self.previews.values_mut() {
            if let PreviewSlot::Pending(rx) = slot {
                match rx.try_recv() {
                    Ok(Some(p)) => *slot = PreviewSlot::Ready(p),
                    Ok(None) | Err(TryRecvError::Disconnected) => *slot = PreviewSlot::Failed,
                    Err(TryRecvError::Empty) => {}
                }
            }
        }
        if self
            .previews
            .values()
            .any(|s| matches!(s, PreviewSlot::Pending(_)))
        {
            return;
        }
        let Some(w) = self.active.as_ref() else {
            return;
        };
        let Some(portal) = w
            .portals
            .iter()
            .filter(|p| p.preview && !self.previews.contains_key(&p.to))
            .map(|p| (p, (p.position - player_pos).length()))
            .filter(|(_, d)| *d <= PREVIEW_RADIUS)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(p, _)| p)
        else {
            return;
        };
        let (tx, rx) = channel();
        let base = self.resolver_base.clone();
        let root = self.worlds_root.clone();
        let to = portal.to.clone();
        std::thread::spawn(move || {
            let preview = resolver::fetch_world(&base, &root, &to)
                .ok()
                .and_then(|tr| {
                    let p = preview_from_manifest(&tr.manifest_text, &tr);
                    // Pull the far side's remote assets into the cache while we're
                    // still on this background thread — the browser's snapshot
                    // render then reads locally, never stalling a frame.
                    if p.is_some() {
                        if let Ok(m) = WorldManifest::from_text(&tr.manifest_text) {
                            AssetSource::hosted(
                                &tr.asset_base,
                                tr.asset_base_url.clone(),
                                &tr.asset_base,
                            )
                            .prefetch(&m);
                        }
                    }
                    p
                });
            let _ = tx.send(preview);
        });
        self.previews
            .insert(portal.to.clone(), PreviewSlot::Pending(rx));
    }

    /// Per-frame: complete an in-flight veilwalk, or start one if the player
    /// stepped into a veil. Returns `Some(spawn)` to reposition the player.
    pub fn update(
        &mut self,
        dt: f32,
        loader: &mut dyn WorldLoader,
        player_pos: Vec3,
    ) -> Option<Vec3> {
        self.update_previews(player_pos);
        // 1. A veilwalk is in flight — see if the destination arrived.
        if let Some(pending) = &self.pending {
            match pending.rx.try_recv() {
                Ok(Ok(tr)) => {
                    let p = self.pending.take().unwrap();
                    let assets = AssetSource::hosted(
                        &tr.asset_base,
                        tr.asset_base_url.clone(),
                        &tr.asset_base,
                    );
                    return self.apply_world(
                        loader,
                        &tr.manifest_text,
                        &assets,
                        p.anchor,
                        Some(p.locator),
                        p.kind,
                    );
                }
                Ok(Err(e)) => {
                    tracing::warn!("veilwalk failed: {e}");
                    self.last_error = Some(short_error(&e));
                    // A failed back/forward must not eat the history entry —
                    // restore it so the button still works after a blip.
                    let p = self.pending.take().unwrap();
                    match p.kind {
                        NavKind::Back => self.back_stack.push(p.locator),
                        NavKind::Forward => self.forward_stack.push(p.locator),
                        NavKind::Normal | NavKind::Reload => {}
                    }
                    // stays disarmed: walk out and back to retry
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.pending = None,
            }
            return None;
        }
        // 2. Cooldown after arriving.
        if self.cooldown > 0.0 {
            self.cooldown = (self.cooldown - dt).max(0.0);
            return None;
        }
        // 3. Arming: a veil fires only when armed, and re-arms only once the
        // player has stepped away from every veil (no bounce, no spam-retry).
        let w = self.active.as_ref()?;
        match nearest_portal(&w.portals, player_pos, ENTER_RADIUS) {
            None => {
                self.armed = true;
                None
            }
            Some(idx) if self.armed => {
                let to = w.portals[idx].to.clone();
                let anchor = player_pos - Vec3::new(0.0, FEET_DROP, 0.0);
                self.begin_travel(to, anchor);
                None
            }
            Some(_) => None, // standing at a veil but not armed → wait
        }
    }
}

/// Instantiate a world's declared behavior modules (wasm assets → sandboxes).
/// Every failure degrades to "that module just isn't there" — a world with a
/// broken behavior still renders and walks (the super-stable tenet).
#[cfg(feature = "behaviors")]
fn load_behaviors(manifest: &WorldManifest, assets: &AssetSource) -> Vec<BehaviorSlot> {
    manifest
        .behaviors
        .iter()
        .filter_map(|b| {
            let asset = manifest.assets.iter().find(|a| a.id == b.wasm)?;
            let path = assets.resolve(&asset.uri)?;
            let bytes = std::fs::read(&path).ok()?;
            match WasmBehavior::load(&bytes) {
                Ok(module) => {
                    tracing::info!("behavior '{}' loaded ({} KB)", b.id, bytes.len() / 1024);
                    Some(BehaviorSlot {
                        id: b.id.clone(),
                        ticks: b.on.iter().any(|e| e == "tick"),
                        module,
                    })
                }
                Err(e) => {
                    tracing::warn!("behavior '{}' failed to load: {e}", b.id);
                    None
                }
            }
        })
        .collect()
}

/// Extract the far side's signpost from fetched manifest text — pure, no IO.
/// Invalid manifests preview as nothing (a veil never breaks over its far side).
fn preview_from_manifest(text: &str, tr: &TravelResult) -> Option<PortalPreview> {
    let m = WorldManifest::from_text(text).ok()?;
    let mut sky = SkyNav::default();
    if let Some(s) = &m.environment.sky {
        sky.zenith = s.zenith;
        sky.horizon = s.horizon;
        if let Some(d) = s.sun_dir {
            sky.sun_dir = d;
        }
    }
    Some(PortalPreview {
        title: m.world.title.clone(),
        sky,
        veils: m.portals.len(),
        manifest_text: text.to_string(),
        asset_base: tr.asset_base.clone(),
        asset_base_url: tr.asset_base_url.clone(),
        presence_relays: m
            .presence
            .as_ref()
            .map(|p| p.relay_list())
            .unwrap_or_default(),
        world_id: m.world.id.clone(),
    })
}

/// Trim a raw fetch error to a short, friendly HUD line.
fn short_error(e: &str) -> String {
    let first = e.split(&['(', '—'][..]).next().unwrap_or(e).trim();
    let msg = if first.is_empty() { e } else { first };
    format!(
        "couldn't reach that world ({})",
        msg.chars().take(60).collect::<String>().trim()
    )
}

/// Index of the nearest portal within `radius` of `pos`, if any.
fn nearest_portal(portals: &[PortalNav], pos: Vec3, radius: f32) -> Option<usize> {
    nearest_by(portals.iter().map(|p| p.position), pos, radius)
}

/// Index of the nearest inspectable within `radius` of `pos`, if any.
fn nearest_interactable(items: &[InteractableNav], pos: Vec3, radius: f32) -> Option<usize> {
    nearest_by(items.iter().map(|p| p.position), pos, radius)
}

/// Translate a declarative interaction's effects into browser [`Action`]s — the
/// browser performs them. `Despawn` needs runtime object removal and is handled
/// in a later slice, so it produces no Action yet.
fn effects_to_actions(effects: &[InteractionEffect]) -> Vec<Action> {
    effects
        .iter()
        .filter_map(|e| match e {
            InteractionEffect::Message(text) => Some(Action::Notify {
                text: text.clone(),
                level: None,
            }),
            InteractionEffect::GiveItem { item, count } => Some(Action::GiveItem {
                item: item.number(),
                count: *count,
            }),
            InteractionEffect::Effect(name) => Some(Action::Notify {
                text: format!("✨ {name}"),
                level: None,
            }),
            InteractionEffect::Navigate(to) => Some(Action::Navigate { to: to.clone() }),
            InteractionEffect::Despawn => None, // runtime removal — next slice
        })
        .collect()
}

/// Index of the nearest point within `radius`, if any.
fn nearest_by(positions: impl Iterator<Item = Vec3>, pos: Vec3, radius: f32) -> Option<usize> {
    positions
        .enumerate()
        .map(|(i, p)| (i, (p - pos).length()))
        .filter(|(_, d)| *d <= radius)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn portal(pos: Vec3, label: &str) -> PortalNav {
        PortalNav {
            id: label.into(),
            position: pos,
            to: String::new(),
            label: label.into(),
            preview: true,
            yaw: 0.0,
        }
    }

    #[test]
    fn declarative_effects_become_browser_actions() {
        // A creator's effects (parsed from manifest JSON) map to Actions the
        // browser performs. Despawn is deferred → emits nothing yet.
        let effects: Vec<InteractionEffect> = serde_json::from_str(
            r#"[ {"give_item":{"item":"20100007","count":3}}, {"message":"Chopped!"}, "despawn" ]"#,
        )
        .unwrap();
        let actions = effects_to_actions(&effects);
        assert_eq!(
            actions.len(),
            2,
            "give_item + message emit; despawn deferred"
        );
        assert!(matches!(actions[0], Action::GiveItem { item: 7, count: 3 }));
        assert!(matches!(&actions[1], Action::Notify { text, .. } if text == "Chopped!"));
    }

    #[test]
    fn nearest_portal_picks_the_closest_within_reach() {
        let portals = vec![
            portal(Vec3::new(10.0, 0.0, 0.0), "far"),
            portal(Vec3::new(1.0, 0.0, 0.0), "near"),
        ];
        assert_eq!(
            nearest_portal(&portals, Vec3::new(0.0, 0.0, 0.0), 2.2),
            Some(1)
        );
        assert_eq!(
            nearest_portal(&portals, Vec3::new(0.0, 0.0, 5.0), 2.2),
            None
        );
    }

    /// A minimal in-memory loader — proves the navigator drives a world through the
    /// `WorldLoader` seam with no renderer at all (the whole point of the split).
    struct FakeLoader {
        loaded: u32,
    }
    impl WorldLoader for FakeLoader {
        fn load(
            &mut self,
            manifest: &WorldManifest,
            _assets: &AssetSource,
            anchor: Vec3,
        ) -> LoadedWorldMeta {
            self.loaded += 1;
            LoadedWorldMeta {
                title: manifest.world.title.clone(),
                world_id: manifest.world.id.clone(),
                spawn: anchor,
                portals: vec![portal(Vec3::new(0.0, 0.0, 0.0), "onward")],
                interactables: vec![],
                sky: SkyNav::default(),
                presence_relays: vec![],
                colliders: vec![],
            }
        }
    }

    const WORLD: &str = r#"{
        "thread": "thread/0.1",
        "world": { "id": "x", "title": "Test World" },
        "spawns": [{ "name": "entry", "position": [1, 2, 3] }]
    }"#;

    #[test]
    fn applies_a_world_through_the_loader_seam() {
        let mut nav = Navigator::new(".");
        let mut loader = FakeLoader { loaded: 0 };
        let spawn = nav.apply_world(
            &mut loader,
            WORLD,
            &AssetSource::local("."),
            Vec3::new(5.0, 0.0, 5.0),
            None,
            NavKind::Normal,
        );
        assert_eq!(
            loader.loaded, 1,
            "the navigator placed the world via the loader"
        );
        assert_eq!(
            spawn,
            Some(Vec3::new(5.0, FEET_DROP, 5.0)),
            "player arrives standing: anchor + the capsule's feet drop"
        );
        assert_eq!(nav.title(), Some("Test World"));
        // A veil is present; standing on it (once armed) begins travel.
        assert!(nav.near_portal(Vec3::new(0.0, 0.0, 0.0)).is_some());
    }

    /// Browsers accept `.thread` markup wherever they accept JSON — the same
    /// text seam (`apply_world`) compiles either form. The shipped Meadow is
    /// the fixture: a world whose only source is markup.
    #[test]
    fn thread_markup_loads_through_the_same_seam_as_json() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../worlds/meadow/world.thread");
        // The reference corpus is content, and content lives with the browser
        // that serves it. Published on its own the engine travels without it,
        // so this reports rather than fails — a missing file that was never
        // this crate's is not a broken crate.
        if !path.exists() {
            eprintln!("no reference corpus beside this crate — skipping");
            return;
        }
        let text = std::fs::read_to_string(&path).expect("the Meadow ships with the repo");
        let mut nav = Navigator::new(".");
        let mut loader = FakeLoader { loaded: 0 };
        let spawn = nav.apply_world(
            &mut loader,
            &text,
            &AssetSource::local("."),
            Vec3::ZERO,
            None,
            NavKind::Normal,
        );
        assert!(spawn.is_some(), "markup compiles and loads");
        assert_eq!(loader.loaded, 1);
        assert_eq!(nav.title(), Some("The Meadow"));
        // View-source shows what the author wrote — the markup itself.
        assert!(nav
            .world_source()
            .is_some_and(|s| s.contains("<world id=\"meadow\"")));
    }

    /// The Meadow's waystone — the first Weft-powered object in a shipped
    /// world: markup `weft=` → manifest asset + behavior → **verified** on
    /// load → interact answers with a running touch count. The Thread's
    /// native code, end to end, through a real world file.
    #[test]
    fn the_meadows_waystone_answers_through_weft() {
        struct MeadowLoader;
        impl WorldLoader for MeadowLoader {
            fn load(
                &mut self,
                m: &WorldManifest,
                _a: &AssetSource,
                anchor: Vec3,
            ) -> LoadedWorldMeta {
                LoadedWorldMeta {
                    title: m.world.title.clone(),
                    world_id: m.world.id.clone(),
                    spawn: anchor,
                    portals: vec![],
                    interactables: m
                        .placements
                        .iter()
                        .map(|pl| InteractableNav {
                            position: Vec3::from_array(pl.position),
                            label: pl.name.clone(),
                            codex: pl.codex.clone(),
                            web: None,
                            name: pl.name.clone(),
                            behavior: pl.behavior.clone(),
                            interaction: pl.interaction.clone(),
                            hits_done: 0,
                            data: pl.data.clone(),
                        })
                        .collect(),
                    sky: SkyNav::default(),
                    presence_relays: vec![],
                    colliders: vec![],
                }
            }
        }
        let world = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../worlds/meadow/world.thread");
        // The reference corpus is content, and content lives with the browser
        // that serves it. Published on its own the engine travels without it,
        // so this reports rather than fails — a missing file that was never
        // this crate's is not a broken crate.
        if !world.exists() {
            eprintln!("no reference corpus beside this crate — skipping");
            return;
        }
        let mut nav = Navigator::new(world.parent().unwrap());
        let mut loader = MeadowLoader;
        nav.load_file(&mut loader, &world, Vec3::ZERO)
            .expect("the Meadow loads");
        // The waystone stands at (0, 1, -2); stand beside it and touch twice.
        let by_the_stone = Vec3::new(0.0, 1.0, -1.4);
        assert_eq!(
            nav.interact(by_the_stone),
            vec![Action::Notify {
                text: "The waystone hums — 1 travelers have touched it.".into(),
                level: None
            }],
        );
        assert_eq!(
            nav.interact(by_the_stone),
            vec![Action::Notify {
                text: "The waystone hums — 2 travelers have touched it.".into(),
                level: None
            }],
            "state persists between touches"
        );
    }

    #[test]
    fn invalid_manifest_is_rejected_without_loading() {
        let mut nav = Navigator::new(".");
        let mut loader = FakeLoader { loaded: 0 };
        let spawn = nav.apply_world(
            &mut loader,
            "{ not a world }",
            &AssetSource::local("."),
            Vec3::ZERO,
            None,
            NavKind::Normal,
        );
        assert_eq!(spawn, None);
        assert_eq!(
            loader.loaded, 0,
            "a bad manifest never reaches the renderer"
        );
        assert_eq!(nav.title(), None);
    }

    #[test]
    fn a_manifest_previews_as_its_title_sky_and_veil_count() {
        let text = r#"{
            "thread": "thread/0.1",
            "world": { "id": "far", "title": "The Far Side" },
            "environment": { "sky": { "zenith": [0.1, 0.2, 0.3], "horizon": [0.4, 0.5, 0.6] } },
            "portals": [
                { "id": "a", "position": [0, 0, 0], "to": "thread://x" },
                { "id": "b", "position": [5, 0, 0], "to": "thread://y" }
            ]
        }"#;
        let tr = TravelResult {
            manifest_text: text.to_string(),
            asset_base: PathBuf::from("."),
            asset_base_url: None,
        };
        let p = preview_from_manifest(text, &tr).expect("valid manifest previews");
        assert_eq!(p.title, "The Far Side");
        assert_eq!(p.sky.zenith, [0.1, 0.2, 0.3]);
        assert_eq!(p.sky.horizon, [0.4, 0.5, 0.6]);
        assert_eq!(p.veils, 2);

        // A broken far side previews as nothing — the veil never breaks over it.
        let tr = TravelResult {
            manifest_text: String::new(),
            asset_base: PathBuf::from("."),
            asset_base_url: None,
        };
        assert!(preview_from_manifest("{ nope }", &tr).is_none());
    }

    #[test]
    fn previews_and_colliders_are_empty_until_a_world_provides_them() {
        let nav = Navigator::new(".");
        assert!(nav.portal_preview("thread://anywhere").is_none());
        assert!(nav.colliders().is_empty());
    }

    #[test]
    fn colliders_flow_from_the_loader_to_the_browser() {
        struct SolidLoader;
        impl WorldLoader for SolidLoader {
            fn load(
                &mut self,
                m: &WorldManifest,
                _a: &AssetSource,
                anchor: Vec3,
            ) -> LoadedWorldMeta {
                LoadedWorldMeta {
                    title: m.world.title.clone(),
                    world_id: m.world.id.clone(),
                    spawn: anchor,
                    portals: vec![],
                    interactables: vec![],
                    sky: SkyNav::default(),
                    presence_relays: vec![],
                    colliders: vec![ColliderNav {
                        half_extents: Vec3::new(2.0, 1.0, 2.0),
                        position: Vec3::new(0.0, 4.0, 0.0),
                        rotation: Quat::IDENTITY,
                    }],
                }
            }
        }
        let mut nav = Navigator::new(".");
        nav.apply_world(
            &mut SolidLoader,
            WORLD,
            &AssetSource::local("."),
            Vec3::ZERO,
            None,
            NavKind::Normal,
        );
        assert_eq!(nav.colliders().len(), 1);
        assert_eq!(nav.colliders()[0].half_extents, Vec3::new(2.0, 1.0, 2.0));
    }

    /// Arrive somewhere via `apply_world` with a Locator, as a completed veilwalk would.
    fn arrive(nav: &mut Navigator, loader: &mut FakeLoader, loc: &str, kind: NavKind) {
        nav.apply_world(
            loader,
            WORLD,
            &AssetSource::local("."),
            Vec3::ZERO,
            Some(loc.into()),
            kind,
        )
        .expect("world applies");
    }

    #[test]
    fn history_works_like_the_webs_back_and_forward() {
        let mut nav = Navigator::new(".");
        let mut loader = FakeLoader { loaded: 0 };

        // Walk A → B → C: two entries behind us, none ahead.
        arrive(&mut nav, &mut loader, "thread://a", NavKind::Normal);
        arrive(&mut nav, &mut loader, "thread://b", NavKind::Normal);
        arrive(&mut nav, &mut loader, "thread://c", NavKind::Normal);
        assert_eq!(nav.back_history(), ["thread://a", "thread://b"]);
        assert!(nav.can_go_back() && !nav.can_go_forward());
        assert_eq!(nav.locator(), Some("thread://c"));

        // Going back to B: C moves to the forward stack. (`back()` pops the
        // destination off the back stack before travel begins — mimic that here,
        // since `arrive` shortcuts the async walk.)
        assert_eq!(nav.back_stack.pop().as_deref(), Some("thread://b"));
        arrive(&mut nav, &mut loader, "thread://b", NavKind::Back);
        assert_eq!(nav.back_history(), ["thread://a"]);
        assert!(nav.can_go_forward());

        // A fresh navigation from B forks history — forward is gone.
        arrive(&mut nav, &mut loader, "thread://d", NavKind::Normal);
        assert_eq!(nav.back_history(), ["thread://a", "thread://b"]);
        assert!(!nav.can_go_forward());
    }

    #[test]
    fn a_reload_is_not_a_navigation() {
        let mut nav = Navigator::new(".");
        let mut loader = FakeLoader { loaded: 0 };
        arrive(&mut nav, &mut loader, "thread://a", NavKind::Normal);
        arrive(&mut nav, &mut loader, "thread://home", NavKind::Normal);
        assert_eq!(nav.back_history(), ["thread://a"]);

        // Build mode re-applies the home file: same world, fresh contents.
        arrive(&mut nav, &mut loader, "thread://home", NavKind::Reload);
        assert_eq!(nav.back_history(), ["thread://a"], "history didn't fork");
        assert!(!nav.can_go_forward());
        assert_eq!(nav.locator(), Some("thread://home"), "the address survives");
        assert_eq!(loader.loaded, 3, "the scene really rebuilt");
    }

    #[test]
    fn view_source_returns_the_manifest_text_and_home_is_where_you_opened() {
        let mut nav = Navigator::new(".");
        let mut loader = FakeLoader { loaded: 0 };
        arrive(&mut nav, &mut loader, "thread://a", NavKind::Normal);
        assert_eq!(
            nav.world_source(),
            Some(WORLD),
            "view-source is the exact bytes"
        );

        // `open()` records the session's home (a real open spawns a fetch thread,
        // so exercise the recording path directly here).
        assert!(nav.home().is_none());
        nav.set_home("thread://a");
        assert_eq!(nav.home(), Some("thread://a"));
        assert!(!nav.go_home(Vec3::ZERO), "already home → no walk begins");
    }

    #[test]
    fn back_with_empty_history_is_a_safe_no_op() {
        let mut nav = Navigator::new(".");
        assert!(!nav.back(Vec3::ZERO));
        assert!(!nav.forward(Vec3::ZERO));
        assert!(!nav.go_home(Vec3::ZERO), "no home set → no walk");
    }

    #[test]
    fn set_actor_keeps_only_real_subs() {
        let mut nav = Navigator::new(".");
        nav.set_actor(Some("  ".into()));
        assert!(nav.actor_sub.is_none(), "blank subs read as anonymous");
        nav.set_actor(Some("did:pixygon:abc".into()));
        assert_eq!(nav.actor_sub.as_deref(), Some("did:pixygon:abc"));
        nav.set_actor(None);
        assert!(nav.actor_sub.is_none());
    }

    #[test]
    fn dispatch_event_without_a_world_is_a_safe_no_op() {
        let mut nav = Navigator::new(".");
        assert!(nav
            .dispatch_event(&serde_json::json!({"event": "purchase_result"}))
            .is_empty());
    }

    /// The host-side commerce fallback contract: `focused()` surfaces a stall's
    /// `data` block, so a browser without the sandboxed module layer can still
    /// open the purchase dialog from `behavior:"buy"` + `data.item`.
    #[test]
    fn focused_carries_the_stall_data_for_hostside_commerce() {
        struct StallLoader;
        impl WorldLoader for StallLoader {
            fn load(
                &mut self,
                m: &WorldManifest,
                _a: &AssetSource,
                anchor: Vec3,
            ) -> LoadedWorldMeta {
                LoadedWorldMeta {
                    title: m.world.title.clone(),
                    world_id: m.world.id.clone(),
                    spawn: anchor,
                    portals: vec![],
                    interactables: m
                        .placements
                        .iter()
                        .map(|pl| InteractableNav {
                            position: Vec3::from_array(pl.position),
                            label: pl.name.clone(),
                            codex: pl.codex.clone(),
                            web: None,
                            name: pl.name.clone(),
                            behavior: pl.behavior.clone(),
                            interaction: pl.interaction.clone(),
                            hits_done: 0,
                            data: pl.data.clone(),
                        })
                        .collect(),
                    sky: SkyNav::default(),
                    presence_relays: vec![],
                    colliders: vec![],
                }
            }
        }
        // A market-shaped stall (matches worlds/market/world.json's wares).
        let world = r#"{
            "thread": "thread/0.1",
            "world": { "id": "market", "title": "The Bazaar" },
            "spawns": [{ "name": "entry", "position": [0, 0, 4] }],
            "prefabs": [{ "id": "60000002", "mesh": { "builtin": "cube" } }],
            "placements": [
                { "prefab": "60000002", "name": "wares:blade", "position": [0, 0, 0],
                  "behavior": "buy",
                  "data": { "item": "21010001", "name": "Veilwalker Blade",
                            "price": 250, "currency": "gold" } }
            ],
            "behaviors": [{ "id": "buy", "wasm": "commerce-wasm", "on": ["interact"] }],
            "assets": [{ "id": "commerce-wasm", "uri": "missing.wasm", "kind": "wasm" }]
        }"#;
        let mut nav = Navigator::new(".");
        nav.apply_world(
            &mut StallLoader,
            world,
            &AssetSource::local("."),
            Vec3::ZERO,
            None,
            NavKind::Normal,
        )
        .expect("world applies");

        let f = nav
            .focused(Vec3::new(0.0, 0.0, 0.5))
            .expect("standing at the stall");
        assert_eq!(f.behavior.as_deref(), Some("buy"));
        assert_eq!(f.data["item"], "21010001");
        assert_eq!(f.data["name"], "Veilwalker Blade");
    }

    /// The full behavior path: a manifest declares a wasm module + a bound
    /// placement, the navigator instantiates the sandbox on world load, and
    /// `interact()` at the placement returns the module's actions.
    #[cfg(feature = "behaviors")]
    #[test]
    fn interact_dispatches_to_the_worlds_wasm_behavior() {
        use crate::behavior::Action;

        // A conformant module: bump allocator + fixed `notify` reply.
        const REPLY: &str = r#"{"actions":[{"action":"notify","text":"the pedestal hums"}]}"#;
        let wat = format!(
            r#"(module
              (memory (export "memory") 1)
              (global $heap (mut i32) (i32.const 4096))
              (data (i32.const 0) "{data}")
              (func (export "thread_alloc") (param i32) (result i32)
                (local $p i32)
                global.get $heap local.set $p
                global.get $heap local.get 0 i32.add global.set $heap
                local.get $p)
              (func (export "thread_on_interact") (param i32 i32) (result i64)
                i64.const {len}))"#,
            data = REPLY.replace('"', "\\\""),
            len = REPLY.len(),
        );
        let dir = std::env::temp_dir().join(format!("loom-nav-behavior-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("hum.wasm"), wat::parse_str(&wat).unwrap()).unwrap();

        // A loader that surfaces the bound placement (as pbr_load/walk do).
        struct BindLoader;
        impl WorldLoader for BindLoader {
            fn load(
                &mut self,
                m: &WorldManifest,
                _a: &AssetSource,
                anchor: Vec3,
            ) -> LoadedWorldMeta {
                LoadedWorldMeta {
                    title: m.world.title.clone(),
                    world_id: m.world.id.clone(),
                    spawn: anchor,
                    portals: vec![],
                    interactables: m
                        .placements
                        .iter()
                        .map(|pl| InteractableNav {
                            position: Vec3::from_array(pl.position),
                            label: pl.name.clone(),
                            codex: pl.codex.clone(),
                            web: None,
                            name: pl.name.clone(),
                            behavior: pl.behavior.clone(),
                            interaction: pl.interaction.clone(),
                            hits_done: 0,
                            data: pl.data.clone(),
                        })
                        .collect(),
                    sky: SkyNav::default(),
                    presence_relays: vec![],
                    colliders: vec![],
                }
            }
        }

        let world = r#"{
            "thread": "thread/0.1",
            "world": { "id": "humming-hall", "title": "Humming Hall" },
            "spawns": [{ "name": "entry", "position": [0, 0, 4] }],
            "prefabs": [{ "id": "60000002", "mesh": { "builtin": "cube" } }],
            "placements": [
                { "prefab": "60000002", "name": "pedestal", "position": [0, 0, 0],
                  "behavior": "hum" }
            ],
            "behaviors": [{ "id": "hum", "wasm": "hum-wasm", "on": ["interact"] }],
            "assets": [{ "id": "hum-wasm", "uri": "hum.wasm", "kind": "wasm" }]
        }"#;

        let mut nav = Navigator::new(&dir);
        let mut loader = BindLoader;
        nav.apply_world(
            &mut loader,
            world,
            &AssetSource::local(&dir),
            Vec3::ZERO,
            None,
            NavKind::Normal,
        )
        .expect("world applies");

        // Standing at the pedestal, interact → the module's action comes back.
        let actions = nav.interact(Vec3::new(0.0, 0.0, 0.5));
        assert_eq!(
            actions,
            vec![Action::Notify {
                text: "the pedestal hums".into(),
                level: None
            }]
        );
        // Standing nowhere near it → nothing.
        assert!(nav.interact(Vec3::new(50.0, 0.0, 50.0)).is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The shipped Archive world + the committed reference `codex-viewer.wasm`
    /// (built from `examples/behaviors/codex-viewer`): standing at a pedestal,
    /// interact returns `codex.open` with that pedestal's slug — the engine
    /// mirrored the placement's `codex` field into `data.slug` for the module.
    #[cfg(feature = "behaviors")]
    #[test]
    fn the_archives_pedestals_open_the_codex_through_wasm() {
        use crate::behavior::Action;

        struct BindLoader;
        impl WorldLoader for BindLoader {
            fn load(
                &mut self,
                m: &WorldManifest,
                _a: &AssetSource,
                anchor: Vec3,
            ) -> LoadedWorldMeta {
                LoadedWorldMeta {
                    title: m.world.title.clone(),
                    world_id: m.world.id.clone(),
                    spawn: anchor,
                    portals: vec![],
                    interactables: m
                        .placements
                        .iter()
                        .map(|pl| InteractableNav {
                            position: Vec3::from_array(pl.position),
                            label: pl.name.clone(),
                            codex: pl.codex.clone(),
                            web: None,
                            name: pl.name.clone(),
                            behavior: pl.behavior.clone(),
                            interaction: pl.interaction.clone(),
                            hits_done: 0,
                            data: pl.data.clone(),
                        })
                        .collect(),
                    sky: SkyNav::default(),
                    presence_relays: vec![],
                    colliders: vec![],
                }
            }
        }

        let world = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../worlds/codex-archive/world.json");
        // The reference corpus is content, and content lives with the browser
        // that serves it. Published on its own the engine travels without it,
        // so this reports rather than fails — a missing file that was never
        // this crate's is not a broken crate.
        if !world.exists() {
            eprintln!("no reference corpus beside this crate — skipping");
            return;
        }
        let mut nav = Navigator::new(world.parent().unwrap());
        let mut loader = BindLoader;
        nav.load_file(&mut loader, &world, Vec3::ZERO)
            .expect("The Archive loads");

        // pedestal:veil sits at [-4.0, 0.5, 0.0] bearing codex "the-veil".
        let actions = nav.interact(Vec3::new(-4.0, 0.5, 0.3));
        assert_eq!(
            actions,
            vec![Action::CodexOpen {
                slug: "the-veil".into()
            }]
        );
    }

    /// The shipped Bazaar + the committed reference `commerce.wasm` (built from
    /// `examples/behaviors/commerce`): interact at a stall returns `commerce.buy`
    /// for that stall's item, and the browser's settled `purchase_result` comes
    /// back as the merchant's `notify` — the full commerce ABI, end to end.
    #[cfg(feature = "behaviors")]
    #[test]
    fn the_bazaars_stalls_sell_through_wasm() {
        use crate::behavior::Action;

        struct BindLoader;
        impl WorldLoader for BindLoader {
            fn load(
                &mut self,
                m: &WorldManifest,
                _a: &AssetSource,
                anchor: Vec3,
            ) -> LoadedWorldMeta {
                LoadedWorldMeta {
                    title: m.world.title.clone(),
                    world_id: m.world.id.clone(),
                    spawn: anchor,
                    portals: vec![],
                    interactables: m
                        .placements
                        .iter()
                        .map(|pl| InteractableNav {
                            position: Vec3::from_array(pl.position),
                            label: pl.name.clone(),
                            codex: pl.codex.clone(),
                            web: None,
                            name: pl.name.clone(),
                            behavior: pl.behavior.clone(),
                            interaction: pl.interaction.clone(),
                            hits_done: 0,
                            data: pl.data.clone(),
                        })
                        .collect(),
                    sky: SkyNav::default(),
                    presence_relays: vec![],
                    colliders: vec![],
                }
            }
        }

        let world =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../worlds/market/world.json");
        let mut nav = Navigator::new(world.parent().unwrap());
        nav.set_actor(Some("did:pixygon:abc".into()));
        let mut loader = BindLoader;
        nav.load_file(&mut loader, &world, Vec3::ZERO)
            .expect("The Bazaar loads");

        // wares:blade sits at [-3.0, 1.3, 0.0] selling item 21010001.
        let actions = nav.interact(Vec3::new(-3.0, 1.3, 0.3));
        assert_eq!(
            actions,
            vec![Action::CommerceBuy {
                item: "21010001".into(),
                price_ref: serde_json::json!("21010001"),
            }]
        );

        // The browser settles the purchase and replies — the merchant reacts.
        let replies = nav.dispatch_event(&serde_json::json!({
            "event": "purchase_result", "ok": true, "item": "21010001",
            "ref": "thread:u1:market:21010001", "duplicate": false,
        }));
        assert_eq!(
            replies,
            vec![Action::Notify {
                text: "The merchant wraps your purchase. \"A fine choice.\"".into(),
                level: None,
            }]
        );
    }
}
