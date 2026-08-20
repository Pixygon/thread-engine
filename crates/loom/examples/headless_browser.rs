//! A second browser for the Thread — in ~130 lines, with **no renderer**.
//!
//! This exists to prove the engine/UI split is real: everything that makes a
//! Thread browser a *browser* (resolving Locators, loading worlds, veilwalking
//! between them, knowing what's around you) lives in [`loom`], behind the
//! [`thread_engine::WorldLoader`] seam. Infinite fills that seam with wgpu; here we fill it
//! with `println!`. Both are browsers; only the rendering differs.
//!
//! It boots into a world, then auto-walks: each "frame" it steps the player toward
//! the nearest veil, through it, into the next world — printing each place it
//! enters. Run against the committed local constellation:
//!
//! ```text
//! cargo run -p loom --example headless_browser
//! cargo run -p loom --example headless_browser -- thread://nexus.pixygon.io/nexus
//! ```

use thread_engine::glam::Vec3;
use thread_engine::infinite_manifest::WorldManifest;
use thread_engine::navigator::{InteractableNav, LoadedWorldMeta, PortalNav, SkyNav, WorldLoader};
use thread_engine::{AssetSource, Loom};

/// A renderer-free loader: it "renders" a world by remembering its layout and
/// printing a line. This is the whole of what this browser adds on top of Loom.
struct TextLoader;

impl WorldLoader for TextLoader {
    fn load(
        &mut self,
        manifest: &WorldManifest,
        _assets: &AssetSource,
        anchor: Vec3,
    ) -> LoadedWorldMeta {
        let spawn = manifest.default_spawn();
        let portals: Vec<PortalNav> = manifest
            .portals
            .iter()
            .map(|p| PortalNav {
                id: p.id.clone(),
                position: Vec3::from_array(p.position),
                to: p.to.clone(),
                label: p.label.clone(),
                preview: p.preview != infinite_manifest::PreviewPolicy::None,
                yaw: 0.0,
            })
            .collect();
        let interactables: Vec<InteractableNav> = manifest
            .placements
            .iter()
            .filter(|pl| pl.codex.is_some() || pl.behavior.is_some())
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
            .collect();

        println!(
            "  ┌─ entered '{}' — {} placement(s), {} veil(s)",
            manifest.world.title,
            manifest.placements.len(),
            portals.len()
        );
        for p in &portals {
            println!("  │   veil '{}' → {}", p.label, p.to);
        }

        // Arrive at the world's declared spawn (ignore the caller's anchor here —
        // a real renderer might use it for a void floor; a text browser needn't).
        let _ = anchor;
        LoadedWorldMeta {
            title: manifest.world.title.clone(),
            world_id: manifest.world.id.clone(),
            spawn: Vec3::from_array(spawn.position),
            portals,
            interactables,
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

fn main() {
    // The local constellation the repo ships (dev resolver: thread://…/x → worlds/x).
    let worlds_root = format!("{}/../../worlds", env!("CARGO_MANIFEST_DIR"));
    let start = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "thread://nexus.pixygon.io/nexus".to_string());

    // One owned loader + the facade = a browser. `Loom` holds the loader, so the
    // whole loop is just `open` then `update` each frame.
    let mut loom = Loom::new(&worlds_root, TextLoader);
    let mut visited: std::collections::HashSet<String> = std::collections::HashSet::new();

    println!("headless browser · opening {start}\n");
    loom.open(&start, Vec3::ZERO);

    // The "game loop": fixed dt, a player that walks toward the nearest veil.
    let dt = 1.0 / 30.0;
    let mut player = Vec3::ZERO;
    let mut hops = 0;
    let max_hops = 6; // stop after touring a few worlds
    let mut idle = 0;

    for _frame in 0..100_000 {
        // Repositioned on arrival (spawn) or after stepping through a veil.
        if let Some(spawn) = loom.update(dt, player) {
            player = spawn;
            hops += 1;
            if hops > max_hops {
                println!("\n  └─ toured {max_hops} worlds. The engine drove them all; this browser only printed.");
                break;
            }
            idle = 0;
            continue;
        }

        // Waiting on a resolve/fetch — let the background thread work.
        if loom.is_traveling() {
            std::thread::sleep(std::time::Duration::from_millis(5));
            continue;
        }

        // Respect the engine's arming: on arrival the navigator cools down and
        // stays disarmed until the player has stepped clear of every veil. Idle
        // (keep pumping `update`, which does the arming) until it's ready.
        if !loom.armed() {
            continue;
        }

        // Armed: walk toward a veil so we actually step through one. Prefer veils
        // leading somewhere new so the tour visits distinct worlds; fall back to
        // the nearest if every onward veil has been walked.
        let veils = loom.portals();
        let nearest = |p: &&PortalNav| (p.position - player).length();
        let pick = veils
            .iter()
            .filter(|p| !visited.contains(&p.to))
            .min_by(|a, b| nearest(a).total_cmp(&nearest(b)))
            .or_else(|| {
                veils
                    .iter()
                    .min_by(|a, b| nearest(a).total_cmp(&nearest(b)))
            });
        match pick {
            Some(target) => {
                // Step onto the chosen veil. (A real browser walks the player here
                // over many frames; headless, we go straight to it so the veil we
                // pick is the one that fires.) The engine still owns the traversal.
                visited.insert(target.to.clone());
                player = target.position;
                idle = 0;
            }
            None => {
                idle += 1;
                if idle > 4 {
                    println!("\n  └─ this world has no onward veils — end of the walk.");
                    break;
                }
            }
        }
    }
}
