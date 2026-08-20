//! Scripted walks — the engine as its own conformance oracle.
//!
//! A [`Script`] is a recorded path: start at a Locator, then step through named
//! veils. [`walk`] drives a *real* [`Navigator`](crate::navigator::Navigator)
//! (resolve → load → veilwalk, background fetch and all) through that script with a
//! renderer-free loader, and reports the worlds it actually landed in. That makes
//! it a deterministic, headless check that the engine navigates a corpus the way it
//! was authored — dogfooding Loom as the oracle, no GPU and no browser required.
//!
//! ```no_run
//! use thread_engine::walk::{walk, Script, Step};
//! let script = Script {
//!     start: "thread://nexus.pixygon.io/nexus".into(),
//!     steps: vec![Step::ByLabel("The Forge".into())],
//! };
//! let outcome = walk("worlds", &script, 4000);
//! assert_eq!(outcome.visited, ["The Nexus", "The Forge"]);
//! ```

use std::path::PathBuf;
use std::time::Duration;

use glam::Vec3;
use infinite_manifest::{Locator, WorldManifest};

use crate::assets::AssetSource;
use crate::engine::Loom;
use crate::navigator::{InteractableNav, LoadedWorldMeta, PortalNav, SkyNav, WorldLoader};

/// How to choose which veil to step through at a given point in a [`Script`].
#[derive(Debug, Clone)]
pub enum Step {
    /// The veil whose label matches (case-insensitive).
    ByLabel(String),
    /// The Nth veil in the active world (author order).
    ByIndex(usize),
    /// The veil whose destination Locator is on this host.
    ToHost(String),
}

/// A recorded walk: where to start, then the veils to take.
#[derive(Debug, Clone)]
pub struct Script {
    pub start: String,
    pub steps: Vec<Step>,
}

/// What actually happened when the engine walked a [`Script`].
#[derive(Debug, Clone)]
pub struct WalkOutcome {
    /// Titles of the worlds landed in, in order (including the start world).
    pub visited: Vec<String>,
    /// The step that couldn't be completed, if any (the walk stops there).
    pub error: Option<String>,
}

impl WalkOutcome {
    /// Whether the whole script completed with no failed step.
    pub fn ok(&self) -> bool {
        self.error.is_none()
    }
}

/// A renderer-free [`WorldLoader`]: maps a manifest to the navigation metadata the
/// engine needs, drawing nothing. The whole "renderer" of this oracle.
struct HeadlessLoader;

impl WorldLoader for HeadlessLoader {
    fn load(
        &mut self,
        manifest: &WorldManifest,
        _assets: &AssetSource,
        _anchor: Vec3,
    ) -> LoadedWorldMeta {
        LoadedWorldMeta {
            title: manifest.world.title.clone(),
            world_id: manifest.world.id.clone(),
            spawn: Vec3::from_array(manifest.default_spawn().position),
            portals: manifest
                .portals
                .iter()
                .map(|p| PortalNav {
                    id: p.id.clone(),
                    position: Vec3::from_array(p.position),
                    to: p.to.clone(),
                    label: p.label.clone(),
                    preview: p.preview != infinite_manifest::PreviewPolicy::None,
                    yaw: 0.0, // headless walker never renders windows
                })
                .collect(),
            interactables: manifest
                .placements
                .iter()
                .filter(|pl| {
                    pl.codex.is_some() || pl.behavior.is_some() || pl.interaction.is_some()
                })
                .map(|pl| InteractableNav {
                    position: Vec3::from_array(pl.position),
                    label: pl.name.clone(),
                    codex: pl.codex.clone(),
                    web: pl
                        .data
                        .get("url")
                        .and_then(|u| u.as_str())
                        .map(str::to_string),
                    name: pl.name.clone(),
                    behavior: pl.behavior.clone(),
                    interaction: pl.interaction.clone(),
                    hits_done: 0,
                    data: pl.data.clone(),
                })
                .collect(),
            sky: SkyNav::default(),
            presence_relays: manifest
                .presence
                .as_ref()
                .map(|p| p.relay_list())
                .unwrap_or_default(),
            colliders: vec![],
        }
    }
}

const DT: f32 = 1.0 / 30.0;

/// Drive the engine through `script` over the corpus at `worlds_root`, spending at
/// most `max_frames` update ticks total. Returns where it landed.
pub fn walk(worlds_root: impl Into<PathBuf>, script: &Script, max_frames: usize) -> WalkOutcome {
    let mut loom = Loom::new(worlds_root, HeadlessLoader);
    let mut visited = Vec::new();
    let mut player = Vec3::ZERO;
    let mut budget = max_frames;

    loom.open(&script.start, Vec3::ZERO);
    if !pump_until_arrival(&mut loom, &mut player, &mut visited, &mut budget) {
        return WalkOutcome {
            visited,
            error: Some(format!("could not open '{}'", script.start)),
        };
    }

    for (i, step) in script.steps.iter().enumerate() {
        if !pump_until_armed(&mut loom, &mut player, &mut budget) {
            return WalkOutcome {
                visited,
                error: Some(format!("step {i}: engine never armed (out of budget)")),
            };
        }
        let Some(dest) = choose(loom.portals(), step) else {
            return WalkOutcome {
                visited,
                error: Some(format!(
                    "step {i}: no veil matching {step:?} in '{}'",
                    loom.title().unwrap_or("?")
                )),
            };
        };
        // Step onto the chosen veil; the engine owns the traversal from here.
        player = dest;
        if !pump_until_arrival(&mut loom, &mut player, &mut visited, &mut budget) {
            return WalkOutcome {
                visited,
                error: Some(format!("step {i}: veilwalk did not complete")),
            };
        }
    }

    WalkOutcome {
        visited,
        error: None,
    }
}

/// The position of the veil a step selects, if present.
fn choose(veils: &[PortalNav], step: &Step) -> Option<Vec3> {
    let veil = match step {
        Step::ByIndex(n) => veils.get(*n),
        Step::ByLabel(l) => veils.iter().find(|p| p.label.eq_ignore_ascii_case(l)),
        Step::ToHost(h) => veils
            .iter()
            .find(|p| Locator::parse(&p.to).is_some_and(|loc| loc.host == *h)),
    };
    veil.map(|v| v.position)
}

/// Pump `update` until the engine reports an arrival (records its title) or the
/// frame budget runs out. Handles the in-flight (background fetch) window.
fn pump_until_arrival(
    loom: &mut Loom<HeadlessLoader>,
    player: &mut Vec3,
    visited: &mut Vec<String>,
    budget: &mut usize,
) -> bool {
    while *budget > 0 {
        *budget -= 1;
        if let Some(spawn) = loom.update(DT, *player) {
            *player = spawn;
            visited.push(loom.title().unwrap_or("?").to_string());
            return true;
        }
        if loom.is_traveling() {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    false
}

/// Pump `update` until the engine is armed, or budget runs out. A spawn may
/// legitimately sit inside a veil's trigger zone (the engine stays safely
/// disarmed there), so — like a real traveler — the oracle steps clear of the
/// nearest veil while it waits.
fn pump_until_armed(
    loom: &mut Loom<HeadlessLoader>,
    player: &mut Vec3,
    budget: &mut usize,
) -> bool {
    const WALK_SPEED: f32 = 4.0;
    while *budget > 0 {
        if loom.armed() {
            return true;
        }
        *budget -= 1;
        loom.update(DT, *player);
        if let Some(dir) = away_from_nearest_veil(loom.portals(), *player) {
            *player += dir * (WALK_SPEED * DT);
        }
    }
    false
}

/// The horizontal direction that steps `player` out of the nearest veil's trigger
/// zone, if it's standing in one.
fn away_from_nearest_veil(veils: &[PortalNav], player: Vec3) -> Option<Vec3> {
    let clearance = crate::navigator::ENTER_RADIUS + 0.5;
    let nearest = veils.iter().min_by(|a, b| {
        a.position
            .distance(player)
            .total_cmp(&b.position.distance(player))
    })?;
    if nearest.position.distance(player) >= clearance {
        return None;
    }
    let mut dir = player - nearest.position;
    dir.y = 0.0;
    Some(dir.try_normalize().unwrap_or(Vec3::X))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worlds_root() -> String {
        format!("{}/../../worlds", env!("CARGO_MANIFEST_DIR"))
    }

    /// The reference corpus is content and travels with the browser that serves
    /// it, not with the engine. Published on its own, these tests report that
    /// they had nothing to walk rather than failing over files that were never
    /// this crate's.
    fn corpus_present() -> bool {
        let there = std::path::Path::new(&worlds_root()).is_dir();
        if !there {
            eprintln!("no reference corpus beside this crate — skipping");
        }
        there
    }

    #[test]
    fn walks_the_constellation_by_label() {
        if !corpus_present() {
            return;
        }
        let script = Script {
            start: "thread://nexus.pixygon.io/nexus".into(),
            steps: vec![
                Step::ByLabel("The Forge".into()),
                Step::ByLabel("The Nexus".into()),
            ],
        };
        let out = walk(worlds_root(), &script, 8000);
        assert!(out.ok(), "walk failed: {:?}", out.error);
        assert_eq!(out.visited, ["The Nexus", "The Forge", "The Nexus"]);
    }

    #[test]
    fn walking_by_index_lands_somewhere_real() {
        if !corpus_present() {
            return;
        }
        let script = Script {
            start: "thread://nexus.pixygon.io/nexus".into(),
            steps: vec![Step::ByIndex(0)],
        };
        let out = walk(worlds_root(), &script, 8000);
        assert!(out.ok(), "walk failed: {:?}", out.error);
        assert_eq!(out.visited.first().map(String::as_str), Some("The Nexus"));
        assert_eq!(out.visited.len(), 2, "start + one hop");
    }

    #[test]
    fn a_missing_veil_is_reported_not_panicked() {
        if !corpus_present() {
            return;
        }
        let script = Script {
            start: "thread://nexus.pixygon.io/nexus".into(),
            steps: vec![Step::ByLabel("No Such Veil".into())],
        };
        let out = walk(worlds_root(), &script, 8000);
        assert!(!out.ok());
        assert!(out.error.as_deref().unwrap().contains("No Such Veil"));
        assert_eq!(
            out.visited,
            ["The Nexus"],
            "we still recorded the start world"
        );
    }
}
