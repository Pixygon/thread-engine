//! # Loom — the embeddable engine for the Thread
//!
//! Loom is to the Thread what a rendering engine (Chromium) is to a browser
//! (Chrome): the reusable, renderer-agnostic core that *any* Thread browser can
//! build on. **Infinite** is the reference browser — winit + wgpu + egui — but
//! it holds no monopoly: everything a browser needs to reach, parse, and stay
//! present in worlds lives here, behind a rendering seam, so a second browser
//! (a different renderer, a different UI, an embedded view) can be built without
//! re-implementing the network core.
//!
//! What Loom owns (all renderer-agnostic):
//! - [`resolver`] — Locator → world manifest ("DNS"): registry → the host's own
//!   `.well-known/thread` → local file. This is what lets *anyone* host a world
//!   on their domain with zero contact.
//! - [`navigator`] — **veilwalking**: the renderer-agnostic navigation state
//!   machine (which world is active, when to step through a veil, what's in
//!   focus), driving a renderer through the [`navigator::WorldLoader`] seam.
//! - [`engine`] — [`Loom`], the one-call facade that owns your loader and reduces
//!   a whole browser to `new` + `open` + `update` each frame.
//! - [`walk`] — scripted walks: drive the engine through a recorded path over a
//!   corpus (headless) and report where it landed — the engine as its own oracle.
//! - [`bookmarks`] — kept places (a personal constellation), persisted as one
//!   human-readable JSON file.
//! - [`home`] — the home-space (`thread://home`): a local world the traveler
//!   owns, seeded on first launch, where bookmarks materialize as veils and
//!   build mode persists blocks.
//! - [`mesh`] — Tier-1 **serverless P2P presence**: a transport-agnostic mesh
//!   coordinator so small worlds are shared with no relay at all.
//! - [`codex`] — canonical-meaning lookups against the live Codex.
//! - [`commerce`] — server-authoritative purchases (`commerce.buy` → the
//!   Thread's purchase endpoint) and entitlement reads, Passport-bearer signed.
//! - [`passport`] — the traveler's held identity (token + fetched descriptor),
//!   presented on presence join and portal handoff, never persisted by hosts.
//! - [`directory`] — the worlds directory (discovery / jump-to).
//! - [`web`] — the in-world reader for the *old* 2D web (transclusion), so
//!   visitors never have to leave the Thread.
//! - [`presence`] — the presence relay client (WebSocket), feeding
//!   [`traveler::PresenceState`].
//!
//! What Loom deliberately does **not** own: windowing, the GPU, meshes, the HUD.
//! Those belong to the browser. The world *runtime* (physics, chunks, player,
//! camera, actors) lives in the host application, and the
//! world *format* in [`infinite_manifest`]; Loom re-exports both so an embedder
//! depends on one crate.

pub mod assets;
pub mod behavior;
#[cfg(feature = "behaviors")]
pub mod behavior_wasm;
pub mod behavior_weft;
pub mod bookmarks;
pub mod codex;
pub mod commerce;
pub mod directory;
pub mod engine;
pub mod home;
pub mod host;
pub mod mesh;
#[cfg(feature = "p2p")]
pub mod mesh_webrtc;
pub mod navigator;
pub mod passport;
pub mod presence;
pub mod recents;
pub mod resolver;
pub mod walk;
pub mod web;

// Re-export the lower layers + math so an embedder pulls in one crate and can
// name the vector type that appears in Loom's public API.
pub use glam;
pub mod traveler;
pub use traveler::{Instance, PoseSample, PresenceState, TravelerPose};
pub use infinite_avatar;
pub use infinite_manifest;

// The most-used types, promoted to the crate root for ergonomic embedding.
pub use assets::AssetSource;
pub use behavior::{Action, ActionList, Behavior, InteractEvent};
pub use bookmarks::{Bookmark, Bookmarks};
pub use codex::CodexClient;
pub use commerce::CommerceClient;
pub use directory::Directory;
pub use engine::Loom;
pub use mesh::{MeshCoordinator, MeshEvent, MeshPose, MeshTransport, Signal};
pub use navigator::{Focus, Navigator, WorldLoader};
pub use passport::Passport;
pub use presence::PresenceClient;
pub use resolver::{fetch_world, TravelResult};
pub use walk::{walk, Script, Step, WalkOutcome};
pub use web::WebReader;
