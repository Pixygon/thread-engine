//! The one-call embedder facade: [`Loom`].
//!
//! [`crate::navigator::Navigator`] is the engine, but it asks you to thread a
//! [`WorldLoader`] through every call. `Loom` owns the loader for you: construct
//! it once with your renderer's loader, `open()` a Locator, and pump
//! `update(dt, player_pos)` each frame. That's the whole browser loop. Everything
//! a browser needs to *decide* — resolve, arm, veilwalk, focus — is inside; you
//! supply only how to *draw* (the loader) and where the player is.
//!
//! ```no_run
//! use thread_engine::{Loom, WorldLoader};
//! use thread_engine::glam::Vec3;
//! # fn demo<L: WorldLoader>(my_loader: L) {
//! let mut loom = Loom::new("worlds", my_loader);
//! loom.open("thread://pixygon.io", Vec3::ZERO);
//! loop {
//!     if let Some(spawn) = loom.update(1.0 / 60.0, /* player_pos */ Vec3::ZERO) {
//!         // teleport the player to `spawn`
//!     }
//!     // draw loom.portals(), react to loom.focused(pos), etc.
//! }
//! # }
//! ```
//!
//! Use this when your renderer's loader can be *owned* by the engine (an embedded
//! view, a headless tool, a from-scratch browser). When the loader must borrow
//! shared renderer state per frame — as Infinite's does, since its `SceneRenderer`
//! is shared with the rest of the app — drive [`Navigator`] directly instead and
//! pass a fresh loader each frame.

use std::path::{Path, PathBuf};

use glam::Vec3;

use crate::navigator::{Focus, Navigator, PortalNav, SkyNav, WorldLoader};

/// A ready-to-pump browser core: a [`Navigator`] wired to an owned [`WorldLoader`].
pub struct Loom<L: WorldLoader> {
    nav: Navigator,
    loader: L,
}

impl<L: WorldLoader> Loom<L> {
    /// Build a browser core over `loader`. `worlds_root` is the local dev-resolver
    /// root (where `thread://host/path` falls back to `<root>/<path>/world.json`).
    pub fn new(worlds_root: impl Into<PathBuf>, loader: L) -> Self {
        Self {
            nav: Navigator::new(worlds_root),
            loader,
        }
    }

    /// Open a world by Locator (the entry to the live Thread). The world loads on
    /// a subsequent [`update`](Self::update) once the async resolve+fetch lands.
    pub fn open(&mut self, locator: &str, anchor: Vec3) {
        self.nav.open(locator, anchor);
    }

    /// Load a local `world.json` directly (the offline / startup path). Returns
    /// the spawn to place the player at.
    pub fn load_file(&mut self, path: &Path, anchor: Vec3) -> Option<Vec3> {
        self.nav.load_file(&mut self.loader, path, anchor)
    }

    /// Advance one frame: finish an in-flight veilwalk, or begin one if the player
    /// stepped into a veil. Returns `Some(spawn)` when the player should be moved.
    pub fn update(&mut self, dt: f32, player_pos: Vec3) -> Option<Vec3> {
        self.nav.update(dt, &mut self.loader, player_pos)
    }

    // --- read-through accessors (the browser's HUD/UI reads these) ---

    /// Title of the active world.
    pub fn title(&self) -> Option<&str> {
        self.nav.title()
    }
    /// The active world's one-line description (may be empty) — arrival cards.
    pub fn description(&self) -> Option<&str> {
        self.nav.world_description()
    }
    /// The active world's stable id (its presence room key).
    pub fn world_id(&self) -> &str {
        self.nav.world_id()
    }
    /// The active world's sky/atmosphere.
    pub fn sky(&self) -> Option<SkyNav> {
        self.nav.sky()
    }
    /// The veils in the active world (render as doorways / minimap / walk toward).
    pub fn portals(&self) -> &[PortalNav] {
        self.nav.portals()
    }
    /// The active world's presence relays (primary first, then fallbacks).
    pub fn presence_relays(&self) -> &[String] {
        self.nav.presence_relays()
    }
    /// The inspectable placement the player is standing at, if any.
    pub fn focused(&self, player_pos: Vec3) -> Option<Focus> {
        self.nav.focused(player_pos)
    }
    /// The veil the player is standing in front of, as `(label, destination)`.
    pub fn near_portal(&self, player_pos: Vec3) -> Option<(&str, &str)> {
        self.nav.near_portal(player_pos)
    }
    /// Whether a veilwalk is in flight (show a spinner).
    pub fn is_traveling(&self) -> bool {
        self.nav.is_traveling()
    }
    /// Whether the engine is idle + armed (a veil will fire on contact).
    pub fn armed(&self) -> bool {
        self.nav.armed()
    }
    /// The most recent veilwalk failure, if any.
    pub fn last_error(&self) -> Option<&str> {
        self.nav.last_error()
    }
    /// The active world's Locator (the address bar), if it arrived by one.
    pub fn locator(&self) -> Option<&str> {
        self.nav.locator()
    }
    /// The active world's raw manifest text (view-source).
    pub fn world_source(&self) -> Option<&str> {
        self.nav.world_source()
    }

    // --- behaviors (the sandboxed interactivity layer) ---

    /// The player interacted with whatever they're standing at → the [`Action`]s
    /// (see [`crate::behavior`]) its bound behavior module asks the browser to
    /// perform. Empty when nothing is bound (fall back to Codex/web focus).
    pub fn interact(&mut self, player_pos: Vec3) -> Vec<crate::behavior::Action> {
        self.nav.interact(player_pos)
    }
    /// Advance the active world's ticking behavior modules — the heartbeat.
    /// Returns the Actions ticking Weft behaviors request.
    pub fn tick_behaviors(&mut self, dt: f32) -> Vec<crate::behavior::Action> {
        self.nav.tick_behaviors(dt)
    }

    // --- session history (the web's back/forward/home, engine-level) ---

    /// Veilwalk back to the previous world. Returns whether a walk began.
    pub fn back(&mut self, anchor: Vec3) -> bool {
        self.nav.back(anchor)
    }
    /// Veilwalk forward again after `back()`.
    pub fn forward(&mut self, anchor: Vec3) -> bool {
        self.nav.forward(anchor)
    }
    /// Veilwalk to the session's home world (by default, where it opened).
    pub fn go_home(&mut self, anchor: Vec3) -> bool {
        self.nav.go_home(anchor)
    }

    /// Borrow the loader (e.g. to read what it recorded) …
    pub fn loader(&self) -> &L {
        &self.loader
    }
    /// … or the navigator, for the full read API.
    pub fn navigator(&self) -> &Navigator {
        &self.nav
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::navigator::LoadedWorldMeta;
    use infinite_manifest::WorldManifest;

    struct CountLoader(u32);
    impl WorldLoader for CountLoader {
        fn load(
            &mut self,
            m: &WorldManifest,
            _assets: &crate::assets::AssetSource,
            anchor: Vec3,
        ) -> LoadedWorldMeta {
            self.0 += 1;
            LoadedWorldMeta {
                title: m.world.title.clone(),
                world_id: m.world.id.clone(),
                spawn: anchor,
                portals: vec![],
                interactables: vec![],
                sky: SkyNav::default(),
                presence_relays: vec![],
                colliders: vec![],
            }
        }
    }

    #[test]
    fn facade_loads_a_local_world_through_the_owned_loader() {
        // Author a tiny world on disk in a temp dir, then open it via the facade.
        let dir = std::env::temp_dir().join("loom_facade_test");
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("world.json");
        std::fs::write(
            &file,
            r#"{ "thread": "thread/0.1", "world": { "id": "w", "title": "Facade World" },
                "spawns": [{ "name": "entry", "position": [0, 0, 2] }] }"#,
        )
        .unwrap();

        let mut loom = Loom::new(&dir, CountLoader(0));
        let spawn = loom.load_file(&file, Vec3::new(1.0, 0.0, 1.0));
        // Arrivals land STANDING: floor anchor + the capsule's feet drop.
        assert_eq!(
            spawn,
            Some(Vec3::new(1.0, crate::navigator::FEET_DROP, 1.0))
        );
        assert_eq!(loom.title(), Some("Facade World"));
        assert_eq!(loom.loader().0, 1, "the facade drove the owned loader");
    }
}
