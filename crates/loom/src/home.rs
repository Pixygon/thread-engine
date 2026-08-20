//! The home-space — the browser's `about:home`, except it's a world you own.
//!
//! A browser shouldn't greet you with a launcher dialog; it should open
//! *somewhere*. On the Thread that somewhere is yours: a small local world,
//! materialized on first launch, that grows with you. Starred worlds
//! ([`crate::bookmarks`]) appear around it as veils — your personal
//! constellation, walkable instead of listed — and build mode adds blocks that
//! persist. It is a plain `world.json` in the standard manifest format:
//! view-source culture applies to your own home too, and because it *is* a
//! real world, one day you can host it and receive visitors.
//!
//! The home has a well-known Locator, [`HOME_LOCATOR`] (`thread://home`) —
//! the Thread's `localhost`. The resolver maps it straight to the local file,
//! never the network, which makes home first-class everywhere a Locator goes:
//! the address bar, back/forward history, the H key, even a portal's `to`.

use std::collections::HashSet;
use std::f32::consts::PI;
use std::path::{Path, PathBuf};

use infinite_manifest::{
    Environment, Interaction, InteractionEffect, LightEmitter, MaterialRef, MeshRef, Placement,
    Portal, Prefab, PreviewPolicy, Sky, Spawn, TextLink, TextPanel, WorldManifest, WorldMeta,
    THREAD_VERSION,
};
use thread_id::StructuredId;

use crate::bookmarks::Bookmarks;
use crate::recents::Recents;

/// The home-space's well-known Locator — resolved to the local home file,
/// never the network (the Thread's `localhost`).
pub const HOME_LOCATOR: &str = "thread://home";

/// The Thread's public front door — the home's one permanent veil, so a fresh
/// home is never a dead end.
pub const FRONT_DOOR: &str = "thread://pixygon.io";

/// Where the home world lives: `~/.config/infinite/home/` (beside
/// `bookmarks.json`, via the same `XDG_CONFIG_HOME` convention).
pub fn default_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("infinite")
        .join("home")
}

/// The home world's manifest file within its directory.
pub fn world_path(dir: &Path) -> PathBuf {
    dir.join("world.json")
}

/// Whether a Locator addresses the traveler's own home-space (`thread://home`,
/// with or without a `#place` / `@when` suffix). The resolver uses this to keep
/// home off the network; the browser uses it to gate build mode.
pub fn is_home(locator: &str) -> bool {
    infinite_manifest::Locator::parse(locator)
        .is_some_and(|l| l.host == "home" && l.path.is_empty())
}

/// One block kind build mode can place. `scale` is the placed size (builtin
/// primitives are unit-sized) and `rest_y` the height its center sits at so it
/// rests on the floor.
pub struct PaletteEntry {
    pub id: StructuredId,
    pub label: &'static str,
    pub scale: [f32; 3],
    pub rest_y: f32,
}

/// The build-mode palette. Prefab ids live in the home's own 60.10.* range so
/// they never collide with a world's authored prefabs.
pub const PALETTE: &[PaletteEntry] = &[
    PaletteEntry {
        id: StructuredId(60_10_0001),
        label: "Stone",
        scale: [1.0, 1.0, 1.0],
        rest_y: 0.5,
    },
    PaletteEntry {
        id: StructuredId(60_10_0002),
        label: "Wood",
        scale: [1.0, 1.0, 1.0],
        rest_y: 0.5,
    },
    PaletteEntry {
        id: StructuredId(60_10_0003),
        label: "Moss",
        scale: [1.0, 1.0, 1.0],
        rest_y: 0.5,
    },
    PaletteEntry {
        id: StructuredId(60_10_0004),
        label: "Brass",
        scale: [1.0, 1.0, 1.0],
        rest_y: 0.5,
    },
    PaletteEntry {
        id: StructuredId(60_10_0005),
        label: "Pillar",
        scale: [0.6, 3.0, 0.6],
        rest_y: 1.5,
    },
    PaletteEntry {
        id: StructuredId(60_10_0006),
        label: "Slab",
        scale: [2.0, 0.25, 2.0],
        rest_y: 0.125,
    },
    PaletteEntry {
        id: StructuredId(60_10_0007),
        label: "Lantern",
        scale: [0.35, 0.35, 0.35],
        rest_y: 1.8,
    },
];

/// The prefab definition behind a palette id (mesh + material).
fn palette_prefab(entry: &PaletteEntry) -> Prefab {
    let (builtin, base_color, metallic, roughness) = match entry.label {
        "Stone" => ("cube", [0.55, 0.55, 0.60, 1.0], 0.0, 0.9),
        "Wood" => ("cube", [0.45, 0.30, 0.18, 1.0], 0.0, 0.8),
        "Moss" => ("cube", [0.25, 0.45, 0.20, 1.0], 0.0, 1.0),
        "Brass" => ("cube", [0.75, 0.60, 0.25, 1.0], 0.8, 0.35),
        "Pillar" => ("cylinder", [0.85, 0.82, 0.78, 1.0], 0.0, 0.6),
        "Slab" => ("cube", [0.35, 0.34, 0.38, 1.0], 0.0, 0.85),
        "Lantern" => ("cube", [1.0, 0.85, 0.50, 1.0], 0.0, 0.2),
        _ => ("cube", [1.0, 1.0, 1.0, 1.0], 0.0, 1.0),
    };
    Prefab {
        id: entry.id,
        mesh: MeshRef {
            asset: None,
            builtin: Some(builtin.to_string()),
            shape: None,
            resolution: None,
        },
        material: Some(MaterialRef {
            base_color,
            metallic,
            roughness,
            ..Default::default()
        }),
        extra: Default::default(),
    }
}

// ── The estate: the seeded heart of every home ──────────────────────────
//
// A home is a homepage: the first thing a traveler sees, every session. The
// estate is the browser-seeded furniture that makes it one — a dais, a
// waystone that searches the Thread, a keeper who explains the place, lamps
// so it reads warm, and a ring of stones so it reads *held*. Everything is
// marked `estate:` so syncs and upgrades never touch what the user built.

const DAIS_ID: StructuredId = StructuredId(60_10_0201);
const WAYSTONE_ID: StructuredId = StructuredId(60_10_0202);
const KEEPER_ID: StructuredId = StructuredId(60_10_0203);
const LAMP_POST_ID: StructuredId = StructuredId(60_10_0204);
const LAMP_HEAD_ID: StructuredId = StructuredId(60_10_0205);
const RING_STONE_ID: StructuredId = StructuredId(60_10_0206);
const BOARD_ID: StructuredId = StructuredId(60_10_0207);

fn estate_prefabs() -> Vec<Prefab> {
    let pf = |id, builtin: &str, base_color, metallic, roughness| Prefab {
        id,
        mesh: MeshRef {
            asset: None,
            builtin: Some(builtin.to_string()),
            shape: None,
            resolution: None,
        },
        material: Some(MaterialRef {
            base_color,
            metallic,
            roughness,
            ..Default::default()
        }),
        extra: Default::default(),
    };
    vec![
        pf(DAIS_ID, "cylinder", [0.32, 0.28, 0.26, 1.0], 0.0, 0.85),
        pf(WAYSTONE_ID, "cube", [0.16, 0.30, 0.38, 1.0], 0.25, 0.35),
        pf(KEEPER_ID, "capsule", [0.92, 0.88, 0.80, 1.0], 0.0, 0.6),
        pf(LAMP_POST_ID, "cylinder", [0.20, 0.19, 0.22, 1.0], 0.6, 0.5),
        pf(LAMP_HEAD_ID, "cube", [1.0, 0.85, 0.55, 1.0], 0.0, 0.2),
        pf(RING_STONE_ID, "cube", [0.34, 0.40, 0.32, 1.0], 0.0, 0.95),
        pf(BOARD_ID, "quad", [0.96, 0.93, 0.86, 1.0], 0.0, 0.9),
    ]
}

/// A placement with the estate's defaults; callers adjust what differs.
fn place(prefab: StructuredId, name: &str, position: [f32; 3], scale: [f32; 3]) -> Placement {
    Placement {
        prefab,
        name: name.to_string(),
        kind: None,
        class: vec![],
        position,
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale,
        codex: None,
        animate: None,
        solid: None,
        light: None,
        text: None,
        behavior: None,
        interaction: None,
        data: serde_json::Value::Null,
        children: vec![],
        extra: Default::default(),
    }
}

/// The keeper's welcome board — the home's guide, links and all.
fn guide_text() -> TextPanel {
    TextPanel {
        content: "YOUR HOME ON THE THREAD\n\
                  \n\
                  This place is yours: a local world,\n\
                  kept on your own machine.\n\
                  \n\
                  Walk with W A S D; hold right-click\n\
                  to look. Walk into a framed veil to\n\
                  cross worlds; H brings you back here.\n\
                  \n\
                  Star a world with M — its door joins\n\
                  the west side. Your recent walks\n\
                  gather on the east.\n\
                  \n\
                  Ask the waystone to search, or\n\
                  press Tab wherever you stand.\n\
                  \n\
                  Press F to build — one day you'll\n\
                  open your door and receive guests.\n\
                  \n\
                  Begin at The Nexus, meet travelers\n\
                  in The Commons, or find your way\n\
                  at The Waystone."
            .to_string(),
        size: 0.029,
        color: [0.13, 0.11, 0.09],
        background: [0.96, 0.93, 0.86],
        links: vec![
            TextLink {
                text: "The Nexus".into(),
                to: "thread://pixygon.io#entry".into(),
            },
            TextLink {
                text: "The Commons".into(),
                to: "thread://commons.pixygon.io#entry".into(),
            },
            TextLink {
                text: "The Waystone".into(),
                to: "thread://waystone.pixygon.io#entry".into(),
            },
        ],
    }
}

fn estate_placements() -> Vec<Placement> {
    let mut out = Vec::new();

    // The dais — a low warm disc the spawn looks across. Visual only: a
    // 12 cm lip under a capsule collider is a stumble, not a step.
    let mut dais = place(DAIS_ID, "estate:dais", [0.0, 0.06, 0.0], [6.4, 0.12, 6.4]);
    dais.solid = Some(false);
    out.push(dais);

    // The waystone — search, standing left of the walk line. Interacting
    // opens the browser's worlds panel (`data.panel`); other browsers see a
    // plain interaction whose message still teaches the key.
    let mut waystone = place(
        WAYSTONE_ID,
        "estate:waystone",
        [-2.0, 1.05, 1.2],
        [0.7, 2.1, 0.45],
    );
    waystone.light = Some(LightEmitter {
        color: [0.35, 0.85, 1.0],
        intensity: 1.5,
        range: 9.0,
    });
    waystone.interaction = Some(Interaction {
        label: "Search the Thread".to_string(),
        hits: 1,
        effects: vec![InteractionEffect::Message(
            "The waystone listens. (Tab opens the search anywhere.)".to_string(),
        )],
    });
    waystone.data = serde_json::json!({ "panel": "worlds" });
    out.push(waystone);

    // The keeper — the home's guide, standing to the right of the walk line.
    // A figure, not a wall of text: inspecting them reads the Codex; the
    // board beside them carries the practical welcome.
    let mut keeper = place(
        KEEPER_ID,
        "estate:keeper",
        [2.0, 0.85, 1.2],
        [0.57, 0.57, 0.57],
    );
    keeper.solid = Some(false); // capsule colliders bounce jumps — decorative
    keeper.codex = Some("the-veilwalkers".to_string());
    keeper.interaction = Some(Interaction {
        label: "Greet the keeper".to_string(),
        hits: 1,
        effects: vec![InteractionEffect::Message(
            "Welcome home, traveler. Walk into a veil to cross; I keep the doors while you're away.".to_string(),
        )],
    });
    out.push(keeper);

    // The welcome board beside the keeper, angled toward the spawn.
    let mut board = place(BOARD_ID, "estate:guide", [3.4, 1.55, 1.6], [2.4, 2.6, 1.0]);
    let a = (-14.0f32).to_radians() / 2.0;
    board.rotation = [0.0, a.sin(), 0.0, a.cos()];
    board.solid = Some(false);
    board.text = Some(guide_text());
    out.push(board);

    // Four lamps at the quarter points between the arcs — warmth and depth.
    for (i, az) in [45.0f32, 135.0, 225.0, 315.0].iter().enumerate() {
        let (dx, dz) = (az.to_radians().sin(), -az.to_radians().cos());
        let (x, z) = (dx * 7.0, dz * 7.0);
        out.push(place(
            LAMP_POST_ID,
            &format!("estate:lamp-post:{i}"),
            [x, 1.0, z],
            [0.16, 2.0, 0.16],
        ));
        let mut head = place(
            LAMP_HEAD_ID,
            &format!("estate:lamp:{i}"),
            [x, 2.2, z],
            [0.32, 0.32, 0.32],
        );
        head.light = Some(LightEmitter {
            color: [1.0, 0.78, 0.45],
            intensity: 1.3,
            range: 8.0,
        });
        head.solid = Some(false);
        out.push(head);
    }

    // A broken ring of low stones past the veils — the home reads held,
    // not walled. Heights vary a little so it reads grown, not stamped.
    for (i, az) in [20.0f32, 65.0, 110.0, 155.0, 205.0, 250.0, 295.0, 340.0]
        .iter()
        .enumerate()
    {
        let (dx, dz) = (az.to_radians().sin(), -az.to_radians().cos());
        let h = 0.7 + 0.15 * ((i * 3 % 4) as f32);
        out.push(place(
            RING_STONE_ID,
            &format!("estate:stone:{i}"),
            [dx * 11.0, h / 2.0, dz * 11.0],
            [1.1, h, 0.9],
        ));
    }
    out
}

/// The seeded sky (kept in one place so [`upgrade`] can tell "still the
/// default" from "the user painted their own").
fn estate_sky() -> Sky {
    Sky {
        zenith: [0.07, 0.09, 0.20],
        horizon: [0.80, 0.42, 0.24],
        sun_dir: Some([0.30, 0.35, 0.25]),
    }
}

/// Bring an existing home up to the current estate — additive and marked:
/// only `estate:` pieces are ever added, nothing the user placed is touched,
/// and a home that already has its estate is left alone. Also warms the
/// hearth (a light on the seeded hearthstone) and refreshes the seeded sky
/// and description — but only when they still carry their original values.
pub fn upgrade(dir: &Path) -> bool {
    edit_manifest(dir, |m| {
        if m.placements.iter().any(|p| p.name == "estate:waystone") {
            // Estate already seeded. The guide board's copy is browser-owned
            // (it teaches THIS browser's keys) — keep it current across
            // releases; everything else estate stays as first seeded.
            if let Some(board) = m.placements.iter_mut().find(|p| p.name == "estate:guide") {
                let fresh = guide_text();
                if board.text.as_ref() != Some(&fresh) {
                    board.text = Some(fresh);
                    return true;
                }
            }
            return false;
        }
        for pf in estate_prefabs() {
            if !m.prefabs.iter().any(|p| p.id == pf.id) {
                m.prefabs.push(pf);
            }
        }
        m.placements.extend(estate_placements());
        // Warm the seeded hearth if it never had a glow.
        if let Some(hearth) = m
            .placements
            .iter_mut()
            .find(|p| p.name == "hearth" && p.light.is_none())
        {
            hearth.light = Some(LightEmitter {
                color: [1.0, 0.45, 0.20],
                intensity: 1.4,
                range: 7.0,
            });
        }
        // Refresh seeded defaults only (a hand-painted sky or description stays).
        let old_seed_sky = Sky {
            zenith: [0.10, 0.12, 0.22],
            horizon: [0.55, 0.35, 0.28],
            sun_dir: Some([0.35, 0.45, 0.30]),
        };
        if m.environment.sky.as_ref() == Some(&old_seed_sky) {
            m.environment.sky = Some(estate_sky());
        }
        if m.world
            .description
            .starts_with("Your own corner of the Thread")
        {
            m.world.description = home_description();
        }
        true
    })
}

fn home_description() -> String {
    "Your own corner of the Thread — the place every session begins. Starred worlds \
     gather as veils to the west, your recent walks to the east; the waystone searches \
     everything, and build mode (F) grows the place block by block."
        .to_string()
}

/// Create the starter home if it doesn't exist yet; returns the manifest path.
/// Never overwrites — the home is the user's, we only seed it once.
pub fn ensure(dir: &Path) -> PathBuf {
    let path = world_path(dir);
    if path.exists() {
        return path;
    }
    let _ = std::fs::create_dir_all(dir);
    match serde_json::to_string_pretty(&starter_world()) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                tracing::warn!("home: cannot write {}: {e}", path.display());
            } else {
                tracing::info!("home: seeded starter home at {}", path.display());
            }
        }
        Err(e) => tracing::warn!("home: serialize failed: {e}"),
    }
    path
}

/// The first-launch home: a warm little island under a dawn sky, one hearth
/// stone, and a single veil to the Thread's front door — never a dead end,
/// never someone else's lobby.
fn starter_world() -> WorldManifest {
    let unit_quat = [0.0, 0.0, 0.0, 1.0];
    let floor_id = StructuredId(60_10_0100);
    let mut prefabs = vec![Prefab {
        id: floor_id,
        mesh: MeshRef {
            asset: None,
            builtin: Some("plane".to_string()),
            shape: None,
            resolution: None,
        },
        material: Some(MaterialRef {
            base_color: [0.16, 0.13, 0.11, 1.0],
            roughness: 0.95,
            ..Default::default()
        }),
        extra: Default::default(),
    }];
    prefabs.extend(PALETTE.iter().map(palette_prefab));
    prefabs.extend(estate_prefabs());

    WorldManifest {
        thread: THREAD_VERSION.to_string(),
        world: WorldMeta {
            id: "home".to_string(),
            title: "Home".to_string(),
            description: home_description(),
            author: None,
            codex: vec!["the-thread".to_string()],
            license: None,
            extra: Default::default(),
        },
        environment: Environment {
            year: None,
            sky: Some(estate_sky()),
            bounds: None,
            rules: Default::default(), // home is a home, not a game
            extra: Default::default(),
        },
        spawns: vec![Spawn {
            name: "entry".to_string(),
            position: [0.0, 0.0, 5.0],
            yaw: PI,
        }],
        assets: vec![],
        prefabs,
        placements: {
            let mut v = vec![
                Placement {
                    prefab: floor_id,
                    name: "floor".to_string(),
                    kind: None,
                    class: vec![],
                    position: [0.0, 0.0, 0.0],
                    rotation: unit_quat,
                    scale: [22.0, 1.0, 22.0],
                    codex: None,
                    animate: None,
                    solid: None,
                    light: None,
                    text: None,
                    behavior: None,
                    interaction: None,
                    data: serde_json::Value::Null,
                    children: vec![],
                    extra: Default::default(),
                },
                Placement {
                    prefab: StructuredId(60_10_0004), // a brass hearthstone, off the walk line
                    name: "hearth".to_string(),
                    kind: None,
                    class: vec![],
                    position: [2.5, 0.25, 0.0],
                    rotation: unit_quat,
                    scale: [1.2, 0.5, 1.2],
                    codex: None,
                    animate: None,
                    solid: None,
                    light: Some(LightEmitter {
                        color: [1.0, 0.45, 0.20],
                        intensity: 1.4,
                        range: 7.0,
                    }),
                    text: None,
                    behavior: None,
                    interaction: None,
                    data: serde_json::Value::Null,
                    children: vec![],
                    extra: Default::default(),
                },
            ];
            v.extend(estate_placements());
            v
        },
        portals: vec![Portal {
            id: "front-door".to_string(),
            position: [0.0, 1.4, -8.0],
            rotation: unit_quat,
            scale: [2.0, 3.0, 0.2],
            to: FRONT_DOOR.to_string(),
            label: "The Nexus".to_string(),
            preview: PreviewPolicy::Live,
            extra: Default::default(),
        }],
        behaviors: vec![],
        styles: vec![],
        presence: None,
        extra: Default::default(),
    }
}

/// The home manifest as served to GUESTS (`presence` injected): an owner-tied
/// P2P room at `room_url`. The disk copy is untouched — the owner's own walk
/// through their home never sees this block; only the invite host serves it.
pub fn served_manifest(dir: &Path, room_url: &str) -> Option<String> {
    let text = std::fs::read_to_string(world_path(dir)).ok()?;
    let mut m = WorldManifest::from_json(&text).ok()?;
    m.presence = Some(infinite_manifest::Presence {
        mode: Some("p2p".to_string()),
        relay: None,
        relays: vec![room_url.to_string()],
        rendezvous: None,
        max_occupants: Some(8),
        voice: true,
        owner_required: true,
        extra: Default::default(),
    });
    serde_json::to_string_pretty(&m).ok()
}

/// Read + parse the home manifest, hand it to `mutate`, and write it back.
/// A missing or unparseable file is left untouched (never clobber the user's
/// home over a transient problem). Returns whether a write happened.
fn edit_manifest(dir: &Path, mutate: impl FnOnce(&mut WorldManifest) -> bool) -> bool {
    let path = world_path(dir);
    let Ok(text) = std::fs::read_to_string(&path) else {
        tracing::warn!("home: cannot read {}", path.display());
        return false;
    };
    let mut manifest = match WorldManifest::from_json(&text) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!(
                "home: {} is not a valid manifest — left untouched: {e}",
                path.display()
            );
            return false;
        }
    };
    if !mutate(&mut manifest) {
        return false;
    }
    match serde_json::to_string_pretty(&manifest) {
        Ok(json) => std::fs::write(&path, json)
            .map_err(|e| tracing::warn!("home: cannot write {}: {e}", path.display()))
            .is_ok(),
        Err(e) => {
            tracing::warn!("home: serialize failed: {e}");
            false
        }
    }
}

/// Regenerate the home's bookmark veils (`fav:` portals) from the current
/// bookmark list. Star a world → a veil appears at home; unstar → it fades.
///
/// Only `fav:` portals are touched: the front door and anything the user
/// authored by hand survive every sync. Bookmarks already reachable through a
/// non-fav portal (e.g. the front door itself) are skipped, as is the home's
/// own Locator. The constellation re-lays itself out on a ring each sync,
/// leaving the northern arc clear for the front door.
pub fn sync_veils(dir: &Path, bookmarks: &Bookmarks) -> bool {
    edit_manifest(dir, |m| {
        let before = serde_json::to_string(&m.portals).unwrap_or_default();
        m.portals.retain(|p| !p.id.starts_with("fav:"));
        // Recents don't block a star — a starred world OUTRANKS its recent
        // veil (the duplicate recent is dropped below).
        let existing: HashSet<String> = m
            .portals
            .iter()
            .filter(|p| !p.id.starts_with("recent:"))
            .map(|p| p.to.clone())
            .collect();
        let favs: Vec<_> = bookmarks
            .entries()
            .iter()
            .filter(|b| b.locator != HOME_LOCATOR && !existing.contains(&b.locator))
            .collect();
        let n = favs.len();
        for (i, b) in favs.iter().enumerate() {
            // The western arc (195°–345°) is the starred constellation; the
            // east belongs to recents, and north (±15°) to the front door.
            let theta = (195.0 + (i as f32 + 0.5) * 150.0 / n as f32).to_radians();
            m.portals.push(Portal {
                id: format!("fav:{}", b.locator),
                position: [8.5 * theta.sin(), 1.4, -8.5 * theta.cos()],
                rotation: [0.0, 0.0, 0.0, 1.0],
                scale: [2.0, 3.0, 0.2],
                to: b.locator.clone(),
                label: b.label.clone(),
                preview: PreviewPolicy::Live,
                extra: Default::default(),
            });
        }
        // Drop any recent veil a fresh star now covers.
        let starred: HashSet<String> = m
            .portals
            .iter()
            .filter(|p| p.id.starts_with("fav:"))
            .map(|p| p.to.clone())
            .collect();
        m.portals
            .retain(|p| !p.id.starts_with("recent:") || !starred.contains(&p.to));
        serde_json::to_string(&m.portals).unwrap_or_default() != before
    })
}

/// Regenerate the home's recent-walks veils (`recent:` portals) from the
/// traveler's [`Recents`] — the eastern arc, freshest nearest the front door.
/// Mirrors [`sync_veils`]'s contract: only `recent:` portals are touched, and
/// places already reachable through another home veil (a starred world, the
/// front door) are skipped rather than doubled.
pub fn sync_recents(dir: &Path, recents: &Recents) -> bool {
    edit_manifest(dir, |m| {
        let before = serde_json::to_string(&m.portals).unwrap_or_default();
        m.portals.retain(|p| !p.id.starts_with("recent:"));
        let existing: HashSet<String> = m.portals.iter().map(|p| p.to.clone()).collect();
        let fresh: Vec<_> = recents
            .entries()
            .iter()
            .filter(|r| r.locator != HOME_LOCATOR && !existing.contains(&r.locator))
            .take(crate::recents::CAP)
            .collect();
        let n = fresh.len();
        for (i, r) in fresh.iter().enumerate() {
            // The eastern arc (15°–165°), freshest closest to the front door.
            let theta = (15.0 + (i as f32 + 0.5) * 150.0 / n as f32).to_radians();
            m.portals.push(Portal {
                id: format!("recent:{}", r.locator),
                position: [8.5 * theta.sin(), 1.4, -8.5 * theta.cos()],
                rotation: [0.0, 0.0, 0.0, 1.0],
                scale: [2.0, 3.0, 0.2],
                to: r.locator.clone(),
                label: r.label.clone(),
                preview: PreviewPolicy::Live,
                extra: Default::default(),
            });
        }
        serde_json::to_string(&m.portals).unwrap_or_default() != before
    })
}

/// Place one block from the palette (build mode). Ensures the palette prefab
/// exists in the manifest (homes seeded before a palette addition still work),
/// then appends a uniquely-named `built:` placement. Returns success.
pub fn add_placement(dir: &Path, entry: &PaletteEntry, position: [f32; 3]) -> bool {
    edit_manifest(dir, |m| {
        if !m.prefabs.iter().any(|p| p.id == entry.id) {
            m.prefabs.push(palette_prefab(entry));
        }
        let next = m
            .placements
            .iter()
            .filter_map(|p| p.name.strip_prefix("built:")?.parse::<u64>().ok())
            .max()
            .map_or(1, |n| n + 1);
        m.placements.push(Placement {
            prefab: entry.id,
            name: format!("built:{next}"),
            kind: None,
            class: vec![],
            position,
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: entry.scale,
            codex: None,
            animate: None,
            solid: None,
            light: None,
            text: None,
            behavior: None,
            interaction: None,
            data: serde_json::Value::Null,
            children: vec![],
            extra: Default::default(),
        });
        true
    })
}

/// Remove the built block nearest `position` (within `radius`). Only touches
/// `built:` placements — the floor, the hearth, and anything hand-authored are
/// not removable by accident. Returns whether a block was removed.
pub fn remove_placement_near(dir: &Path, position: [f32; 3], radius: f32) -> bool {
    edit_manifest(dir, |m| {
        let dist_sq = |p: &Placement| -> f32 {
            let d = [
                p.position[0] - position[0],
                p.position[1] - position[1],
                p.position[2] - position[2],
            ];
            d[0] * d[0] + d[1] * d[1] + d[2] * d[2]
        };
        let nearest = m
            .placements
            .iter()
            .enumerate()
            .filter(|(_, p)| p.name.starts_with("built:") && dist_sq(p) <= radius * radius)
            .min_by(|(_, a), (_, b)| dist_sq(a).total_cmp(&dist_sq(b)))
            .map(|(i, _)| i);
        match nearest {
            Some(i) => {
                m.placements.remove(i);
                true
            }
            None => false,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_home(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("loom-home-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn read(dir: &Path) -> WorldManifest {
        WorldManifest::from_json(&std::fs::read_to_string(world_path(dir)).unwrap()).unwrap()
    }

    #[test]
    fn ensure_seeds_a_valid_walkable_starter_home_once() {
        let dir = temp_home("seed");
        let path = ensure(&dir);
        assert!(path.exists());

        let m = read(&dir);
        assert_eq!(m.world.id, "home");
        assert!(!m.spawns.is_empty(), "a home you can arrive in");
        assert!(
            m.placements.iter().any(|p| p.name == "floor"),
            "a floor to stand on"
        );
        assert!(
            m.portals.iter().any(|p| p.to == FRONT_DOOR),
            "never a dead end — the front door veil is there"
        );

        // A second ensure must not clobber (the home is the user's).
        let marker = m.placements.len();
        ensure(&dir);
        assert_eq!(read(&dir).placements.len(), marker);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn starring_and_unstarring_worlds_grows_and_prunes_the_veil_ring() {
        let dir = temp_home("veils");
        ensure(&dir);
        let store = dir.join("bookmarks.json");
        let mut b = Bookmarks::load(&store);
        b.add("The Forge", "thread://pixiel.ai/forge");
        b.add("The Grove", "thread://garden.pixygon.io/grove");
        // Neither of these should become a veil:
        b.add("Home itself", HOME_LOCATOR);
        b.add("Front door", FRONT_DOOR); // already reachable through the permanent veil

        assert!(sync_veils(&dir, &b));
        let m = read(&dir);
        let favs: Vec<_> = m
            .portals
            .iter()
            .filter(|p| p.id.starts_with("fav:"))
            .collect();
        assert_eq!(favs.len(), 2);
        assert!(favs.iter().any(|p| p.to == "thread://pixiel.ai/forge"));
        assert_eq!(m.portals.iter().filter(|p| p.to == FRONT_DOOR).count(), 1);

        // Unstar → the veil fades on the next sync; the front door survives.
        b.remove("thread://pixiel.ai/forge");
        assert!(sync_veils(&dir, &b));
        let m = read(&dir);
        assert!(!m.portals.iter().any(|p| p.to == "thread://pixiel.ai/forge"));
        assert!(m.portals.iter().any(|p| p.to == FRONT_DOOR));

        // No change → no rewrite.
        assert!(!sync_veils(&dir, &b));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn served_manifest_is_an_owner_tied_p2p_room_and_disk_stays_clean() {
        let dir = temp_home("served");
        ensure(&dir);
        let room = "ws://192.0.2.17:4200/thread/home";
        let text = served_manifest(&dir, room).expect("serves");
        let m = WorldManifest::from_json(&text).unwrap();
        let p = m.presence.expect("presence injected");
        assert_eq!(p.mode.as_deref(), Some("p2p"));
        assert!(p.owner_required, "the room dies with its owner");
        assert_eq!(p.relays, vec![room.to_string()]);
        // The disk copy is the owner's own home — no presence block on it.
        assert!(read(&dir).presence.is_none(), "local home untouched");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fresh_homes_carry_the_estate_and_old_homes_upgrade_once() {
        let dir = temp_home("estate");
        ensure(&dir);
        let m = read(&dir);
        // The homepage essentials: a guide with links, a search stone, warmth.
        let guide = m
            .placements
            .iter()
            .find(|p| p.name == "estate:guide")
            .expect("guide board");
        let text = guide.text.as_ref().expect("guide has text");
        assert!(text.content.contains("YOUR HOME"), "welcome copy present");
        assert_eq!(text.links.len(), 3, "guide links to Nexus/Commons/Waystone");
        let stone = m
            .placements
            .iter()
            .find(|p| p.name == "estate:waystone")
            .expect("waystone");
        assert_eq!(
            stone.data.get("panel").and_then(|v| v.as_str()),
            Some("worlds")
        );
        assert!(stone.light.is_some(), "the waystone glows");
        assert!(
            m.placements
                .iter()
                .filter(|p| p.name.starts_with("estate:lamp:"))
                .count()
                >= 4
        );
        // A fresh home is already current — upgrade is a no-op.
        assert!(!upgrade(&dir));

        // A pre-estate home (simulate by stripping estate pieces) upgrades once,
        // and the upgrade never touches user-built blocks.
        let mut old = read(&dir);
        old.placements.retain(|p| !p.name.starts_with("estate:"));
        std::fs::write(
            world_path(&dir),
            serde_json::to_string_pretty(&old).unwrap(),
        )
        .unwrap();
        assert!(add_placement(&dir, &PALETTE[0], [4.0, 0.5, 4.0]));
        assert!(upgrade(&dir));
        let m = read(&dir);
        assert!(m.placements.iter().any(|p| p.name == "estate:waystone"));
        assert!(
            m.placements.iter().any(|p| p.name == "built:1"),
            "user blocks survive"
        );
        assert!(!upgrade(&dir), "second upgrade is a no-op");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn recent_walks_become_veils_on_the_east_and_never_double_a_fav() {
        let dir = temp_home("recents");
        ensure(&dir);
        let store = dir.join("recents.json");
        let mut r = Recents::load(&store);
        r.visit("The Weavery", "thread://weft.pixygon.io");
        r.visit("The Commons", "thread://commons.pixygon.io");
        r.visit("Front door", FRONT_DOOR); // already a permanent veil → skipped

        assert!(sync_recents(&dir, &r));
        let m = read(&dir);
        let recents: Vec<_> = m
            .portals
            .iter()
            .filter(|p| p.id.starts_with("recent:"))
            .collect();
        assert_eq!(recents.len(), 2);
        // Eastern arc: every recent veil sits at positive x.
        for p in &recents {
            assert!(
                p.position[0] > 0.0,
                "recent veil {} on the east (x {})",
                p.id,
                p.position[0]
            );
        }
        assert_eq!(
            m.portals.iter().filter(|p| p.to == FRONT_DOOR).count(),
            1,
            "no doubles"
        );

        // A world both starred and recent appears once (the star wins).
        let bstore = dir.join("bookmarks.json");
        let mut b = Bookmarks::load(&bstore);
        b.add("The Commons", "thread://commons.pixygon.io");
        assert!(sync_veils(&dir, &b));
        assert!(sync_recents(&dir, &r));
        let m = read(&dir);
        assert_eq!(
            m.portals
                .iter()
                .filter(|p| p.to == "thread://commons.pixygon.io")
                .count(),
            1,
            "starred+recent world keeps a single veil"
        );
        // And favs stay west.
        for p in m.portals.iter().filter(|p| p.id.starts_with("fav:")) {
            assert!(p.position[0] < 0.0, "fav veil {} on the west", p.id);
        }
        // No change → no rewrite.
        assert!(!sync_recents(&dir, &r));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_mode_places_and_removes_only_built_blocks() {
        let dir = temp_home("build");
        ensure(&dir);

        assert!(add_placement(&dir, &PALETTE[0], [3.0, 0.5, 3.0]));
        assert!(add_placement(&dir, &PALETTE[4], [5.0, 1.5, 3.0]));
        let m = read(&dir);
        assert_eq!(
            m.placements
                .iter()
                .filter(|p| p.name.starts_with("built:"))
                .count(),
            2
        );
        // Names stay unique even after removals in between.
        assert!(m.placements.iter().any(|p| p.name == "built:1"));
        assert!(m.placements.iter().any(|p| p.name == "built:2"));

        // Removal near the hearth (not built:) must not touch it.
        assert!(!remove_placement_near(&dir, [2.5, 0.25, 0.0], 0.6));
        assert!(read(&dir).placements.iter().any(|p| p.name == "hearth"));

        // Remove the stone block; the pillar survives.
        assert!(remove_placement_near(&dir, [3.0, 0.5, 3.0], 1.5));
        let m = read(&dir);
        assert_eq!(
            m.placements
                .iter()
                .filter(|p| p.name.starts_with("built:"))
                .count(),
            1
        );

        // The manifest is still valid and the palette prefabs are present.
        assert!(m.prefabs.iter().any(|p| p.id == PALETTE[4].id));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
