//! # Grow — a plant is a species, a seed and a clock
//!
//! Chisel's carving vocabulary is axis-aligned and rotates about Y only, so a
//! tree came out as a cone with horizontal sticks (2026-09-25). Image-to-3D
//! drops thin branching entirely. Branching is its own kind of geometry and
//! deserves its own generator — the same way [`flora`](crate::flora) owns
//! *where* plants stand, `grow` owns *what one plant is*.
//!
//! Three inputs, and only three:
//!
//! - **[`Species`]** is the rule set: how it branches, where leaves attach,
//!   what the bark wears, how long it takes to grow up. It is the stable
//!   identity — the thing the Quarry stores and a world manifest names.
//! - **Seed** is one number, and it decides the **whole potential plant up
//!   front**: every branch the plant could ever have, already shaped. Grove
//!   used to consume a random stream as the tree grew, so hiding the branches
//!   a sapling has not sprouted yet reshuffled everything that remained.
//!   Randomness is [addressed](crate::rand) now — `hash(seed, branch, slot)` —
//!   so a branch's shape is its own, whoever else exists.
//! - **[`Clock`]** is when you are looking: `age` in seasons since sprouting
//!   (ticks the game controls, never wall-clock days) and where the year
//!   stands. Age does exactly two things: it **filters** — which branches have
//!   emerged — and it **scales** — how far each has grown, and how thick.
//!
//! - **[`State`]** is what has *happened* to it: a short list — branches cut,
//!   sockets taken, health events — small enough to sync in a game. The
//!   Lantern Desert is the proof case: the withered lantern tree is not a
//!   second species but the lantern in a `withered` state — leaves off, fruit
//!   dropped, gnarl up, bark darkened — the same seed and the same branches on
//!   both halves of the map. Harvest, pick and chop are state, not meshes.
//!
//! So `grow(species, seed, clock, state)`: one individual at one moment, and
//! the same one in Unity, in Thread and in the Quarry. A plant's whole state is
//! `{ species, seed, age, season, state }` — small enough to sync in a game and
//! to sit in a World Manifest placement.
//!
//! What comes out:
//! - a tube mesh per LOD (fewer sides and levels as detail drops), with UVs
//!   that run around and along each branch so a bark recipe tiles cleanly;
//! - **wind in the vertex alpha**, the engine's convention (1.0 = rigid): the
//!   root is rigid, tips sway most, so the same mesh moves in Infinite and can
//!   be read by a Unity shader from the same channel;
//! - **sockets** at every tip — position, direction, radius, and the id of the
//!   branch it ends — so a fruit, a lantern, a leaf cluster or a Trellis-made
//!   hero prop can hang exactly where the plant ends, in a manifest or the
//!   Quarry's `facts`. A tip is a branch with no children *yet*: a sapling's
//!   sockets are at the ends it actually has. Sockets are **typed** — `tip`,
//!   `bloom`, `fruit`, and `cut` when step 3 lands — and the year decides which
//!   of them exist: a species in bloom carries bloom sockets, the same species
//!   in autumn carries fruit at the very same tips. Chopping leaves a `cut`
//!   socket at the joint, and the fallen part is [`fallen`] — the subtree
//!   grown on its own;
//! - **every vertex names its branch** in `TEXCOORD_1` (low 16 bits in `u`,
//!   high 16 in `v`), so a game can hit-test a swing against the wood — and
//!   the leaves on it — without asking anyone. **Cuts are at joints only**: a
//!   cut names a branch, and takes it from where it sprouts;
//! - **wind in four channels**, because the wood knows its hierarchy:
//!   `TEXCOORD_2 = (trunk sway, branch sway)` — how much of a whole-tree lean
//!   and how much of a limb's own swing reach this vertex — and
//!   `TEXCOORD_3 = (leaf flutter, phase)` — the tremble only a leaf has, and a
//!   per-branch phase so no two limbs march in step. The colour's alpha stays
//!   rigidity (`1 − sway`, TERRA §6.3) for everything that already reads it;
//!   Unity reads the four (`com.pixygon.quarry`'s Grove Wind shader).
use std::collections::HashSet;

use infinite_manifest::texture::TextureRecipe;
use serde::{Deserialize, Serialize};

use chisel::model::{Built, BuiltPart};
use chisel::MeshData;

use crate::clock::{Clock, Phase, Stage};
use crate::foliage::{leaves, LeafRecipe, LeafSite};
use crate::rand::{child_key, unit, Rnd};

// ── Addresses ───────────────────────────────────────────────────────────────
// A branch's numbers live at named slots on its own key, so adding a rule
// later cannot shift the ones already there, and nothing depends on the order
// branches are built in.

/// The trunk's key; every other key is hashed from its parent's.
const TRUNK: u32 = 0x7472_0001;
const KIND_FORK: u32 = 0;
const KIND_LATERAL: u32 = 1;
/// Leaf clusters have keys too — one for the tip, one per along-site.
pub(crate) const KIND_LEAF_TIP: u32 = 2;
pub(crate) const KIND_LEAF_ALONG: u32 = 3;

const SLOT_LEAN_DIR: u32 = 1;
const SLOT_PHASE: u32 = 2;
const SLOT_SPREAD: u32 = 3;
const SLOT_AROUND: u32 = 4;
const SLOT_LENGTH: u32 = 5;
const SLOT_BEND_AXIS: u32 = 6;
const SLOT_BEND_SIGN: u32 = 7;
/// Two wander curves, eight control values each.
const SLOT_WANDER_A: u32 = 16;
const SLOT_WANDER_B: u32 = 32;
const WANDER_CONTROLS: u32 = 8;

/// Whether a tip carries blooms, and whether it carries fruit, are two
/// separate questions asked of the same tip.
const SLOT_BLOOM_SHARE: u32 = 8;
const SLOT_FRUIT_SHARE: u32 = 9;
/// Where in its swing a branch is when the wind starts.
const SLOT_WIND_PHASE: u32 = 10;

/// A seedling is a stem, not a point: the least of its trunk a plant ever shows.
const SPROUT_EXTENSION: f32 = 0.06;

/// Everything a species is told — the rules, and nothing about which
/// individual or which moment.
///
/// This is the plant's identity: two plants with the same species are the same
/// kind of thing, and the Quarry keys a design on it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Species {
    pub name: String,
    /// Trunk length in metres up to its first fork, before any droop — at full
    /// size. A younger plant is a scaled, part-grown version of this one.
    pub height: f32,
    /// Trunk radius at the ground, at full size.
    pub trunk_radius: f32,
    /// Generations of branching beyond the trunk (0 = a bare trunk).
    pub levels: u32,
    /// Lateral children per branch, per level (last value repeats): they
    /// sprout along the parent between `sprout_from` and its tip.
    pub branches: Vec<u32>,
    /// Forks per branch, per level (last repeats): children that start AT the
    /// parent's tip and carry it on. A real tree is mostly forks — the trunk
    /// ends where it splits into limbs — so `height` is the trunk up to its
    /// first fork, not the tree's height.
    pub forks: Vec<u32>,
    /// Smooth bend along a branch, degrees over its whole length, per level
    /// (last repeats). Each branch picks its own bend plane from the seed, so
    /// limbs arc instead of jitter. `wobble` is the wander on top.
    pub curve: Vec<f32>,
    /// Smooth bend of the trunk itself, degrees.
    pub trunk_curve: f32,
    /// Tilt of the trunk at the ground, degrees from vertical.
    pub lean: f32,
    /// Branching angle from the parent, degrees, per level (last repeats).
    pub angle: Vec<f32>,
    /// Child length as a fraction of the parent, per level (last repeats).
    pub length: Vec<f32>,
    /// Radius at a branch's tip as a fraction of its base radius.
    pub taper: f32,
    /// Radius of a child at its base as a fraction of the parent's radius
    /// where it sprouts.
    pub child_radius: f32,
    /// Downward pull along a branch, 0 = straight; negative lifts (crystals grow up).
    pub gravity: f32,
    /// How far a branch wanders off its arc over its whole length, 0..1. It is
    /// a curve the seed decided, sampled along the branch, so a coarse LOD
    /// wanders the same way instead of a different way.
    pub wobble: f32,
    /// How far up its parent the first child sprouts, 0..1.
    pub sprout_from: f32,
    /// Ring vertices around a branch at LOD0.
    pub sides: u32,
    /// Segments along a branch at LOD0.
    pub segments: u32,
    /// Root flare: the trunk base is `(1 + flare)` × trunk_radius, easing to the
    /// plain radius over the first 12 % of the trunk. A tree without it is a pole.
    pub flare: f32,
    /// How much the tips sway, 0..1 (root is always rigid).
    pub sway: f32,
    /// Seasons from sprouting to full size — the life curve. Age divided by
    /// this is the plant's maturity, and it is the only place the clock meets
    /// the shape.
    pub seasons_to_grown: f32,
    /// Size at sprouting as a fraction of the grown plant.
    pub sprout_size: f32,
    /// Seasons from sprouting to the end of the arc. Growing up is a small
    /// slice of it: a plant is grown for far longer than it spends getting
    /// there, and the last stretch is where the crown thins.
    pub seasons_of_life: f32,
    /// An evergreen keeps its crown all year; a deciduous one comes and goes
    /// with the season. A species with no leaves does not care either way.
    pub evergreen: bool,
    /// What this species looks like when it dies standing.
    pub wither: WitherRecipe,
    /// Blooms: where they sit and when in the year. Absent = a species that
    /// never flowers.
    pub blooms: Option<CropRecipe>,
    /// Fruit: the same, later in the year. A lantern tree's lanterns are
    /// fruit — they hang at fruit sockets and come from Trellis, a carve or a
    /// recipe, whichever supplier the world uses.
    pub fruit: Option<CropRecipe>,
    /// Bark recipe. Absent = the flat `color`.
    pub bark: Option<TextureRecipe>,
    /// Flat colour when there is no bark recipe (RGBA).
    pub color: [f32; 4],
    /// Glow strength for the wood (crystal trees glow a little).
    pub emissive: f32,
    /// Detail fractions for LOD1, LOD2… (sides and segments scale by these).
    pub lods: Vec<f32>,
    /// Foliage: leaf clusters at the tips and along the last twigs. Absent =
    /// bare wood (a crystal tree, a dead one, winter).
    pub leaves: Option<LeafRecipe>,
}

/// The name the species went by when a recipe was the whole plant. Kept so
/// vendored callers (the Quarry) keep compiling and published recipe JSON keeps
/// its shape; new code says [`Species`].
pub type GrowRecipe = Species;

impl Default for Species {
    fn default() -> Self {
        Self {
            name: "tree".into(),
            height: 6.0,
            trunk_radius: 0.3,
            levels: 3,
            branches: vec![2, 1, 1],
            forks: vec![3, 2, 2],
            curve: vec![15.0, 20.0, 25.0],
            trunk_curve: 0.0,
            lean: 0.0,
            angle: vec![35.0, 40.0, 45.0],
            length: vec![0.6, 0.55, 0.5],
            taper: 0.6,
            child_radius: 0.55,
            gravity: 0.15,
            wobble: 0.25,
            sprout_from: 0.55,
            sides: 10,
            segments: 6,
            flare: 0.6,
            sway: 0.6,
            seasons_to_grown: 12.0,
            sprout_size: 0.06,
            seasons_of_life: 160.0,
            evergreen: false,
            wither: WitherRecipe::default(),
            blooms: None,
            fruit: None,
            bark: None,
            color: [0.4, 0.3, 0.2, 1.0],
            emissive: 0.0,
            lods: vec![0.5, 0.25],
            leaves: None,
        }
    }
}

/// One plant: a species, the individual, and the moment.
///
/// This is what a `.grow.json` file holds — the species' fields flat, plus
/// `seed` for which individual and `age` / `season` for when. It is also what
/// travels to the Quarry, which regrows the same plant from it.
#[derive(Debug, Clone)]
pub struct Planting {
    pub species: Species,
    /// Which individual of the species. One number, and it decides the whole
    /// potential plant.
    pub seed: u32,
    pub clock: Clock,
    /// What has happened to it.
    pub state: State,
}

impl Default for Planting {
    fn default() -> Self {
        Self { species: Species::default(), seed: 1, clock: Clock::default(), state: State::default() }
    }
}

/// The clock and the individual as a recipe file spells them, beside the
/// species' own fields.
#[derive(Debug, Deserialize)]
#[serde(default)]
struct Planted {
    seed: u32,
    age: Option<f32>,
    season: f32,
    /// `{"clock": {...}}` also works, for callers that hold a [`Clock`].
    clock: Option<Clock>,
    /// The state, flat: `"withered": true`, `"cut": [..]`, `"taken": [..]`.
    withered: bool,
    cut: Vec<u32>,
    taken: Vec<Pick>,
    state: Option<State>,
}

impl Default for Planted {
    fn default() -> Self {
        let c = Clock::default();
        Self { seed: 1, age: c.age, season: c.season, clock: None, withered: false, cut: Vec::new(), taken: Vec::new(), state: None }
    }
}

impl Planting {
    /// Read a recipe file: the species' fields, `seed`, `age`, `season`.
    pub fn from_json(text: &str) -> Result<Self, String> {
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
        Self::from_value(&value)
    }

    /// The same, from a value already parsed (the Quarry's submission).
    pub fn from_value(value: &serde_json::Value) -> Result<Self, String> {
        let species: Species = serde_json::from_value(value.clone())
            .map_err(|e| format!("not a grow recipe: {e}"))?;
        let planted: Planted = serde_json::from_value(value.clone())
            .map_err(|e| format!("not a planting: {e}"))?;
        let clock = planted.clock.unwrap_or(Clock { age: planted.age, season: planted.season });
        let state = planted.state.unwrap_or(State { withered: planted.withered, cut: planted.cut, taken: planted.taken });
        Ok(Self { species, seed: planted.seed, clock, state })
    }

    /// Back to one flat object — the shape a recipe file and a Quarry
    /// submission carry. Only what the planting actually says beyond a grown
    /// plant appears, so a design id never forks on a field that means nothing.
    pub fn to_value(&self) -> serde_json::Value {
        let mut v = serde_json::to_value(&self.species).unwrap_or_default();
        if let Some(obj) = v.as_object_mut() {
            obj.insert("seed".into(), serde_json::json!(self.seed));
            obj.insert("season".into(), serde_json::json!(self.clock.season));
            if let Some(age) = self.clock.age {
                obj.insert("age".into(), serde_json::json!(age));
            }
            if self.state.withered {
                obj.insert("withered".into(), serde_json::json!(true));
            }
            if !self.state.cut.is_empty() {
                obj.insert("cut".into(), serde_json::json!(self.state.cut));
            }
            if !self.state.taken.is_empty() {
                obj.insert("taken".into(), serde_json::to_value(&self.state.taken).unwrap_or_default());
            }
        }
        v
    }
}

/// What a species looks like when it dies standing.
///
/// Every field here bends or dims the plant the seed already decided — none of
/// them re-rolls it. A withered tree has the same branches in the same order
/// with the same ids as the living one; they have simply gone further along
/// their own arc and stopped catching the light.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WitherRecipe {
    /// Extra arc along every branch, degrees, bent the way that branch was
    /// already bending — the gnarl.
    pub gnarl: f32,
    /// Extra droop along every branch.
    pub sag: f32,
    /// How far the wood darkens, 0 = unchanged, 1 = black. Any glow goes with
    /// it: a crystal tree's light goes out.
    pub darken: f32,
    /// How far the colour drains towards grey, 0 = unchanged, 1 = colourless.
    /// Darkening alone cannot kill a crystal tree — a dimmer blue is a blue
    /// tree at night. What reads as dead is the colour leaving first.
    pub drain: f32,
}

impl Default for WitherRecipe {
    fn default() -> Self {
        Self { gnarl: 22.0, sag: 0.16, darken: 0.55, drain: 0.7 }
    }
}

/// Where a species carries blooms or fruit, and when in the year.
///
/// A crop is not geometry: it is **sockets**, and what hangs at them comes from
/// Trellis, a carve or a recipe. Picking one is then a note against a socket's
/// name, not a new mesh.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CropRecipe {
    /// How many at each carrying tip.
    pub per_tip: u32,
    /// What share of eligible tips carry any, 0..1. Addressed per tip, so the
    /// same tips carry it every year and a tip's answer does not depend on
    /// what its neighbours are doing.
    pub share: f32,
    /// Generations back from the outermost that carry it (1 = the twigs only).
    pub depth: u32,
    /// The window in the year it is on the plant, `[start, end]` in 0..1. It
    /// may wrap: `[0.9, 0.1]` is midwinter.
    pub window: [f32; 2],
    /// How far back along the twig a cluster of more than one spreads.
    pub along: f32,
}

impl Default for CropRecipe {
    fn default() -> Self {
        Self { per_tip: 1, share: 0.5, depth: 1, window: [0.4, 0.7], along: 0.25 }
    }
}

/// A socket that has been picked, and when it grows back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pick {
    /// The socket's name — `fruit-<branch id>-<i>`, as the plant reported it.
    pub socket: String,
    /// The age, in seasons, at which the socket is back. `None` never regrows
    /// on its own: the game strikes the entry when it decides to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<f32>,
}

/// What has happened to a plant, beyond its age and the year.
///
/// The roadmap's third input, and the whole of a plant's mutable state: a
/// short list, small enough to sync in a game and to sit in a World Manifest
/// placement. Everything here is a *filter* on the plant the seed decided —
/// none of it grows anything new.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    /// Dead but standing: no leaves, no crop, gnarled, darkened.
    pub withered: bool,
    /// Branches cut off, by id — each taken at the joint it sprouts from,
    /// with everything that grew from it. The trunk cannot be cut here; felling
    /// is a different verb. A cut leaves a `cut` socket at the joint, and the
    /// part that fell is [`fallen`].
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cut: Vec<u32>,
    /// Sockets picked: the only mesh change is one instance not being there.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub taken: Vec<Pick>,
}

impl State {
    /// A plant that nothing has happened to.
    pub fn thriving() -> Self {
        Self::default()
    }
    /// Dead standing.
    pub fn withered() -> Self {
        Self { withered: true, ..Self::default() }
    }
    /// The same state with one more branch cut.
    pub fn cut(mut self, branch: u32) -> Self {
        if !self.cut.contains(&branch) {
            self.cut.push(branch);
        }
        self
    }
    /// The same state with one more socket picked.
    pub fn take(mut self, socket: &str, until: Option<f32>) -> Self {
        self.taken.push(Pick { socket: socket.to_string(), until });
        self
    }
    /// Is this socket picked at this moment? A pick with no regrow time
    /// holds until the game strikes it; one with a time holds until the plant
    /// is that old — and a plant with no age cannot have grown it back.
    pub fn is_taken(&self, socket: &str, clock: Clock) -> bool {
        self.taken.iter().any(|p| {
            p.socket == socket
                && match (p.until, clock.age) {
                    (Some(until), Some(age)) => age < until,
                    _ => true,
                }
        })
    }
}

/// What a socket is for. `Cut` is reserved for step 3, where a chopped branch
/// leaves one behind at the joint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SocketKind {
    /// The end of a branch: foliage, a lantern, a prop.
    Tip,
    Bloom,
    Fruit,
    /// Where a branch was cut off.
    Cut,
}

/// A branch id as `TEXCOORD_1` carries it: low 16 bits in `u`, high 16 in
/// `v`, both exact in f32. Unity: `id = (uint)uv2.x | ((uint)uv2.y << 16)`.
pub fn branch_uv(key: u32) -> [f32; 2] {
    [(key & 0xffff) as f32, (key >> 16) as f32]
}

/// The id back out of the channel.
pub fn branch_from_uv(uv: [f32; 2]) -> u32 {
    (uv[0].round() as u32 & 0xffff) | ((uv[1].round() as u32 & 0xffff) << 16)
}

impl SocketKind {
    fn prefix(self) -> &'static str {
        match self {
            SocketKind::Tip => "tip",
            SocketKind::Bloom => "bloom",
            SocketKind::Fruit => "fruit",
            SocketKind::Cut => "cut",
        }
    }
}

/// An attachment point where a branch ends.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Socket {
    /// `<kind>-<branch id>` (and an index when a tip carries more than one) —
    /// the same name for the same socket at every age and every level of
    /// detail, so state can name it and Unity can find it.
    pub name: String,
    /// What it is for.
    pub kind: SocketKind,
    pub position: [f32; 3],
    /// Unit vector the branch points along at its tip.
    pub direction: [f32; 3],
    pub radius: f32,
    /// Branching generation (0 = the trunk's own tip).
    pub level: u32,
    /// The branch this tip belongs to: its identity in the potential plant.
    /// Cutting, picking and regrowth all record branches by this id.
    pub branch: u32,
}

/// One branch as the rest of the world sees it: what a vertex's id refers to.
///
/// The mesh names branches; this says what the names mean — where each one
/// sprouts, where it ends, how it hangs together — so a game can turn a hit
/// on the wood into a cut at the right joint, and a wind shader can find the
/// branch's parent, without walking any geometry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchInfo {
    /// The id every vertex of this branch carries.
    pub id: u32,
    /// The branch it sprouts from (the trunk names itself).
    pub parent: u32,
    pub level: u32,
    /// Where it sprouts — the joint a cut is made at.
    pub base: [f32; 3],
    /// Where it ends at this moment.
    pub tip: [f32; 3],
    /// Radius at the base.
    pub radius: f32,
    /// What is left of a cut branch.
    pub stump: bool,
}

/// One grown plant: the LOD0 model (wood, and leaves when the species has
/// them), the coarser LODs as whole models (nearest first), and the sockets.
pub struct Grown {
    pub built: Built,
    /// LOD1, LOD2… — each a complete model (same parts, fewer triangles).
    pub lods: Vec<Built>,
    pub sockets: Vec<Socket>,
    /// Every branch the plant has at this moment, in the order they were
    /// grown (parents before children).
    pub branches: Vec<BranchInfo>,
    pub bounds: ([f32; 3], [f32; 3]),
    /// How far through its growing life this individual is, 0..1. At 1 it is
    /// the whole potential plant the seed decided.
    pub maturity: f32,
    /// Where in the long arc it is.
    pub stage: Stage,
    /// Where the year stands.
    pub phase: Phase,
}

// ── The potential plant ─────────────────────────────────────────────────────

#[derive(Clone)]
struct Branch {
    /// This branch's identity in the potential plant.
    key: u32,
    parent: u32,
    level: u32,
    /// Where on the parent it sprouts, 0..1 — a fork sits at the tip (1.0).
    attach: f32,
    /// Maturity at which this branch starts to extend, 0..1.
    emerge: f32,
    /// What is left of a cut branch: a stub at the joint, and no tip.
    stump: bool,
    /// Where in its swing this branch is, 0..1 — its own, from the seed.
    phase: f32,
    /// Polyline of centre points.
    pts: Vec<[f32; 3]>,
    /// Radius at each point.
    radii: Vec<f32>,
    /// Sway at each point (0 rigid → species.sway at the outermost tips):
    /// the one number Infinite reads, in the colour's alpha.
    sway: Vec<f32>,
    /// Trunk sway weight at each point: 0 at the ground, 1 at the top of the
    /// trunk, and then whatever the limb left the trunk with, all the way out.
    trunk_w: Vec<f32>,
    /// Branch sway weight: 0 all along the trunk, then climbing each
    /// generation towards 1 at the outermost tips.
    branch_w: Vec<f32>,
}

fn pick<T: Copy>(v: &[T], i: usize, fallback: T) -> T {
    v.get(i).copied().or_else(|| v.last().copied()).unwrap_or(fallback)
}

fn norm(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-6);
    [v[0] / l, v[1] / l, v[2] / l]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn len(a: [f32; 3]) -> f32 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}
fn lerp3(a: [f32; 3], b: [f32; 3], u: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * u, a[1] + (b[1] - a[1]) * u, a[2] + (b[2] - a[2]) * u]
}
/// Rotate `v` about unit axis `k` by `angle` (Rodrigues).
fn rotate(v: [f32; 3], k: [f32; 3], angle: f32) -> [f32; 3] {
    let (s, c) = angle.sin_cos();
    let kv = cross(k, v);
    let kd = dot(k, v);
    [
        v[0] * c + kv[0] * s + k[0] * kd * (1.0 - c),
        v[1] * c + kv[1] * s + k[1] * kd * (1.0 - c),
        v[2] * c + kv[2] * s + k[2] * kd * (1.0 - c),
    ]
}
/// A unit vector perpendicular to `d`.
fn perp(d: [f32; 3]) -> [f32; 3] {
    let alt = if d[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] };
    norm(cross(d, alt))
}

/// Grow one branch to its full potential length, as a polyline from `origin`
/// along `dir`. Every number it needs is addressed on its own key, so this
/// branch comes out the same whatever else the plant has.
#[allow(clippy::too_many_arguments)]
fn grow_branch(
    s: &Species,
    withered: bool,
    rnd: &Rnd,
    origin: [f32; 3],
    dir: [f32; 3],
    length: f32,
    radius0: f32,
    level: u32,
    segments: u32,
    sway_from: f32,
    curve_deg: f32,
    wind_from: [f32; 2],
) -> Grew {
    let segs = segments.max(2) as usize;
    let mut pts = Vec::with_capacity(segs + 1);
    let mut radii = Vec::with_capacity(segs + 1);
    let mut sway = Vec::with_capacity(segs + 1);
    let mut trunk_w = Vec::with_capacity(segs + 1);
    let mut branch_w = Vec::with_capacity(segs + 1);
    // The trunk's own weight climbs to 1 at its top and every limb inherits
    // where it left; a limb's own weight climbs one generation's share from
    // where its parent had got to.
    let branch_to = if level == 0 { 0.0 } else { (wind_from[1] + 1.0 / s.levels.max(1) as f32).min(1.0) };
    let mut p = origin;
    let mut d = norm(dir);
    let step = length / segs as f32;
    let sway_to = (sway_from + s.sway / (s.levels as f32 + 1.0)).min(s.sway);
    // Death bends a branch further along the arc it was already drawing and
    // pulls it down: the same branch, gone further and stopped.
    let (gnarl, sag) = if withered { (s.wither.gnarl, s.wither.sag) } else { (0.0, 0.0) };
    // One bend plane per branch: a fixed perpendicular axis, a fixed sign, so
    // the branch draws an arc. The magnitude eases in (stiff near the base).
    let bend_axis = rotate(perp(d), d, rnd.at(SLOT_BEND_AXIS) * std::f32::consts::TAU);
    let bend_total = (curve_deg + gnarl).to_radians() * if rnd.at(SLOT_BEND_SIGN) < 0.5 { -1.0 } else { 1.0 };
    // The wander is spread over the whole branch, so the same branch meshed
    // with six segments or with sixty wanders the same amount.
    let wander = s.wobble * 1.2 / segs as f32;
    for i in 0..=segs {
        let t = i as f32 / segs as f32;
        pts.push(p);
        let mut rad = radius0 * (1.0 - t) + radius0 * s.taper * t;
        if level == 0 && s.flare > 0.0 && t < 0.12 {
            let f = 1.0 - t / 0.12;
            rad *= 1.0 + s.flare * f * f;
        }
        radii.push(rad);
        sway.push(sway_from + (sway_to - sway_from) * t);
        // A trunk bends like a cantilever: little at the base, most at the top.
        trunk_w.push(if level == 0 { t * t } else { wind_from[0] });
        branch_w.push(wind_from[1] + (branch_to - wind_from[1]) * t);
        if i == segs {
            break;
        }
        // The arc, then droop (or lift) a little each step, then the wander.
        if bend_total != 0.0 {
            let ease = 0.5 + t; // more bend towards the tip
            d = norm(rotate(d, bend_axis, bend_total / segs as f32 * ease));
        }
        d = norm(add(d, [0.0, -(s.gravity + sag) * step, 0.0]));
        if s.wobble > 0.0 {
            let axis = perp(d);
            let a = rnd.noise(SLOT_WANDER_A, WANDER_CONTROLS, t) * wander;
            let b = rnd.noise(SLOT_WANDER_B, WANDER_CONTROLS, t) * wander;
            d = norm(rotate(rotate(d, axis, a), norm(cross(d, axis)), b));
        }
        p = add(p, scale(d, step));
    }
    Grew { pts, radii, sway, trunk_w, branch_w }
}

/// What growing one branch yields: the per-point columns of a [`Branch`].
struct Grew {
    pts: Vec<[f32; 3]>,
    radii: Vec<f32>,
    sway: Vec<f32>,
    trunk_w: Vec<f32>,
    branch_w: Vec<f32>,
}

/// The whole potential plant the seed decided: every branch it could ever
/// have, at full size, trunk first, breadth-first by level. The clock is not
/// here — nothing in this function knows what age anything is.
fn potential(s: &Species, seed: u32, withered: bool, segments: u32) -> Vec<Branch> {
    // One generation's growing window: the trunk fills its own, then each
    // generation fills the next, so the last is complete exactly at full size.
    let span = 1.0 / (s.levels as f32 + 1.0);
    let trunk_rnd = Rnd::new(seed, TRUNK);
    let lean = s.lean.to_radians();
    let lean_dir = trunk_rnd.at(SLOT_LEAN_DIR) * std::f32::consts::TAU;
    let up = norm([lean.sin() * lean_dir.cos(), lean.cos(), lean.sin() * lean_dir.sin()]);
    let g = grow_branch(
        s, withered, &trunk_rnd, [0.0, 0.0, 0.0], up, s.height, s.trunk_radius, 0, segments, 0.0, s.trunk_curve, [0.0, 0.0],
    );
    let mut out: Vec<Branch> = vec![Branch {
        key: TRUNK,
        parent: 0,
        level: 0,
        attach: 0.0,
        emerge: 0.0,
        stump: false,
        phase: trunk_rnd.at(SLOT_WIND_PHASE),
        pts: g.pts,
        radii: g.radii,
        sway: g.sway,
        trunk_w: g.trunk_w,
        branch_w: g.branch_w,
    }];
    let mut frontier: Vec<usize> = vec![0];
    for level in 1..=s.levels {
        let li = (level - 1) as usize;
        let n_lateral = pick(&s.branches, li, 0);
        let n_forks = pick(&s.forks, li, 2);
        let angle = pick(&s.angle, li, 40.0).to_radians();
        let len_frac = pick(&s.length, li, 0.55);
        let curve = pick(&s.curve, li, 15.0);
        let mut next: Vec<usize> = Vec::new();
        for &pi in &frontier {
            let (pkey, pemerge) = (out[pi].key, out[pi].emerge);
            let parent_len: f32 = {
                let b = &out[pi];
                (1..b.pts.len()).map(|i| len(sub(b.pts[i], b.pts[i - 1]))).sum()
            };
            let phase = Rnd::new(seed, pkey).at(SLOT_PHASE) * std::f32::consts::TAU;
            // Forks: start at the tip, share its cross-section (da Vinci: the
            // children's areas sum to the parent's), spread evenly around it.
            // A fork waits for its parent to reach its tip, so it emerges one
            // whole window after the parent did.
            for c in 0..n_forks {
                let key = child_key(pkey, KIND_FORK, c);
                let rnd = Rnd::new(seed, key);
                let b = &out[pi];
                let (p, d, rad, sw) = sample(b, 1.0);
                let wind = sample_wind(b, 1.0);
                let spread = angle * (0.7 + 0.3 * rnd.at(SLOT_SPREAD));
                let around =
                    phase + std::f32::consts::TAU * c as f32 / n_forks as f32 + rnd.signed(SLOT_AROUND) * 0.25;
                let side = rotate(perp(d), d, around);
                let cdir = norm(add(scale(d, spread.cos()), scale(side, spread.sin())));
                let clen = parent_len * len_frac * (0.8 + 0.4 * rnd.at(SLOT_LENGTH));
                let crad = (rad * (1.0 / n_forks as f32).sqrt() * 1.08).min(rad * 0.98);
                // Start a little inside the parent so the joint is buried.
                let origin = sub(p, scale(d, crad * 0.8));
                let segs = child_segments(s, segments, clen);
                let g = grow_branch(s, withered, &rnd, origin, cdir, clen, crad, level, segs, sw, curve, wind);
                out.push(Branch {
                    key,
                    parent: pkey,
                    level,
                    attach: 1.0,
                    emerge: pemerge + span,
                    stump: false,
                    phase: rnd.at(SLOT_WIND_PHASE),
                    pts: g.pts,
                    radii: g.radii,
                    sway: g.sway,
                    trunk_w: g.trunk_w,
                    branch_w: g.branch_w,
                });
                next.push(out.len() - 1);
            }
            // Laterals: along the parent between `sprout_from` and just below
            // the tip, and each emerges when the parent has grown past it.
            for c in 0..n_lateral {
                let key = child_key(pkey, KIND_LATERAL, c);
                let rnd = Rnd::new(seed, key);
                let b = &out[pi];
                let t = s.sprout_from + (0.9 - s.sprout_from) * (c as f32 + 0.5) / n_lateral as f32;
                let (p, d, rad, sw) = sample(b, t);
                let wind = sample_wind(b, t);
                let around = phase
                    + 1.9
                    + std::f32::consts::TAU * c as f32 / n_lateral as f32
                    + rnd.signed(SLOT_AROUND) * 0.4;
                let side = rotate(perp(d), d, around);
                let spread = angle * (1.1 + 0.3 * rnd.at(SLOT_SPREAD));
                let cdir = norm(add(scale(d, spread.cos()), scale(side, spread.sin())));
                let clen = parent_len * len_frac * (0.6 + 0.3 * rnd.at(SLOT_LENGTH));
                let crad = rad * s.child_radius;
                let segs = child_segments(s, segments, clen);
                let g = grow_branch(s, withered, &rnd, p, cdir, clen, crad, level, segs, sw, curve, wind);
                out.push(Branch {
                    key,
                    parent: pkey,
                    level,
                    attach: t,
                    emerge: pemerge + span * t,
                    stump: false,
                    phase: rnd.at(SLOT_WIND_PHASE),
                    pts: g.pts,
                    radii: g.radii,
                    sway: g.sway,
                    trunk_w: g.trunk_w,
                    branch_w: g.branch_w,
                });
                next.push(out.len() - 1);
            }
        }
        frontier = next;
    }
    out
}

/// Height comes fast and then slows; girth keeps thickening after height has
/// almost stopped. Two curves, one exponent each — enough for a sapling to
/// read as a whippy young tree instead of a shrunken old one. Both reach 1 at
/// full size, so a grown plant is exactly the potential plant.
fn length_scale(s: &Species, m: f32) -> f32 {
    s.sprout_size + (1.0 - s.sprout_size) * m.powf(0.6)
}
fn girth_scale(s: &Species, m: f32) -> f32 {
    s.sprout_size + (1.0 - s.sprout_size) * m.powf(1.3)
}

/// The clock's whole job: which of the potential branches have emerged, how
/// far along its own line each has grown, and how big the plant is.
///
/// Nothing here invents geometry. A branch that is half grown is the first
/// half of the branch it will be, and a sapling is the grown plant scaled
/// down — which is why ageing a plant never reshuffles it.
fn at_clock(s: &Species, all: &[Branch], m: f32) -> Vec<Branch> {
    if m >= 1.0 {
        return all.to_vec();
    }
    let span = 1.0 / (s.levels as f32 + 1.0);
    let ls = length_scale(s, m);
    let gs = girth_scale(s, m);
    let mut out = Vec::with_capacity(all.len());
    for b in all {
        let mut e = ((m - b.emerge) / span).clamp(0.0, 1.0);
        if b.level == 0 {
            // Even a seedling is a stem.
            e = e.max(SPROUT_EXTENSION);
        }
        if e <= 0.0 {
            continue;
        }
        out.push(grown_to(b, e, ls, gs));
    }
    out
}

/// A branch grown `e` of the way along the line it will have, the whole plant
/// scaled to its age. A branch that has just emerged is also thinner than its
/// grown self — girth arrives with the branch, not before it.
fn grown_to(b: &Branch, e: f32, length_scale: f32, girth_scale: f32) -> Branch {
    let p = prefix(b, e);
    let girth = girth_scale * (0.35 + 0.65 * e);
    Branch {
        pts: p.pts.iter().map(|p| scale(*p, length_scale)).collect(),
        radii: p.radii.iter().map(|r| r * girth).collect(),
        ..p
    }
}

/// The first `e` of a branch — at least two points, the last one interpolated
/// so the end lands exactly where it should — every per-point column cut the
/// same way.
fn prefix(b: &Branch, e: f32) -> Branch {
    let n = b.pts.len();
    let f = e.clamp(0.0, 1.0) * (n - 1) as f32;
    let whole = (f.floor() as usize).min(n - 1);
    let u = f - whole as f32;
    let extend = whole + 1 < n && (u > 1e-4 || whole == 0);
    let u = u.max(1e-3);
    let cut = |v: &[f32]| -> Vec<f32> {
        let mut o = v[..=whole].to_vec();
        if extend {
            o.push(v[whole] + (v[whole + 1] - v[whole]) * u);
        }
        o
    };
    let mut pts: Vec<[f32; 3]> = b.pts[..=whole].to_vec();
    if extend {
        pts.push(lerp3(b.pts[whole], b.pts[whole + 1], u));
    }
    Branch {
        pts,
        radii: cut(&b.radii),
        sway: cut(&b.sway),
        trunk_w: cut(&b.trunk_w),
        branch_w: cut(&b.branch_w),
        ..b.clone()
    }
}

/// How much of a cut branch stays on the tree: enough for the wound to show
/// and the `cut` socket to sit on, and no more.
const STUMP: f32 = 0.05;

/// Which of the plant is being asked for.
#[derive(Clone, Copy, PartialEq)]
enum Part {
    /// The standing plant: everything present, minus what has been cut.
    Standing,
    /// One cut branch and everything that grew from it, brought to the origin.
    Fallen(u32),
}

/// The state's whole job, after the clock has done its own: take away what
/// has been cut — a stump where each cut branch sprouted, nothing beyond it —
/// or, for the fallen part, keep only that and stand it on its own base.
///
/// Like the clock, this only ever filters. A branch below a cut is the branch
/// it always was; a branch above one is simply not there.
fn at_state(present: Vec<Branch>, cut: &[u32], part: Part) -> Vec<Branch> {
    let parent_of: std::collections::HashMap<u32, u32> = present.iter().map(|b| (b.key, b.parent)).collect();
    // The trunk has no joint to cut at.
    let cut: Vec<u32> = cut.iter().copied().filter(|k| *k != TRUNK).collect();
    // Walk up from a branch: the first cut ancestor (or itself), if any, and
    // whether it descends from `root`.
    let lineage = |mut key: u32| -> (Option<u32>, bool) {
        let mut first_cut = None;
        let mut under_root = matches!(part, Part::Fallen(r) if r == key);
        loop {
            if first_cut.is_none() && cut.contains(&key) {
                first_cut = Some(key);
            }
            if key == TRUNK {
                break;
            }
            match parent_of.get(&key) {
                Some(&p) => {
                    key = p;
                    if matches!(part, Part::Fallen(r) if r == key) {
                        under_root = true;
                    }
                }
                None => break,
            }
        }
        (first_cut, under_root)
    };
    let base = match part {
        Part::Fallen(root) => present.iter().find(|b| b.key == root).map(|b| b.pts[0]),
        Part::Standing => None,
    };
    let mut out = Vec::with_capacity(present.len());
    for b in present {
        let (first_cut, under_root) = lineage(b.key);
        match part {
            Part::Standing => match first_cut {
                None => out.push(b),
                Some(k) if k == b.key => out.push(Branch { stump: true, ..prefix(&b, STUMP) }),
                Some(_) => {}
            },
            Part::Fallen(root) => {
                if !under_root {
                    continue;
                }
                // A cut inside the fallen part is still a cut — unless it is
                // the root itself, which is the whole point of the part.
                match first_cut {
                    Some(k) if k != root && k == b.key => out.push(Branch { stump: true, ..prefix(&b, STUMP) }),
                    Some(k) if k != root => {}
                    _ => out.push(b),
                }
            }
        }
    }
    if let Some(base) = base {
        for b in out.iter_mut() {
            for p in b.pts.iter_mut() {
                *p = sub(*p, base);
            }
        }
    }
    out
}

/// One plant at one moment: the four inputs, and the readings the clock takes
/// off them. Everything that meshes takes this rather than five loose values.
#[derive(Clone, Copy)]
struct Moment<'a> {
    s: &'a Species,
    seed: u32,
    clock: Clock,
    state: &'a State,
    part: Part,
    maturity: f32,
    stage: Stage,
}

impl<'a> Moment<'a> {
    fn new(s: &'a Species, seed: u32, clock: Clock, state: &'a State, part: Part) -> Self {
        Self {
            s,
            seed,
            clock,
            state,
            part,
            maturity: clock.maturity(s.seasons_to_grown),
            stage: clock.stage(s.seasons_to_grown, s.seasons_of_life),
        }
    }

    /// How much crown the plant is wearing, 0..1 — the year, the long arc and
    /// death, in that order. It scales the leaf count at every site, and
    /// because a cluster's leaves are addressed, thinning drops leaves rather
    /// than growing a different crown.
    fn leafiness(&self) -> f32 {
        if self.state.withered {
            return 0.0;
        }
        let year = if self.s.evergreen { 1.0 } else { self.clock.leaf_fullness() };
        year * match self.stage {
            Stage::Sapling | Stage::Mature => 1.0,
            // An old tree thins from the outside in; a dying one is mostly bare.
            Stage::Old => 0.6,
            Stage::Dying => 0.2,
        }
    }

    /// Is the plant carrying a crop at all — old enough, alive, in a season
    /// for it?
    fn bears(&self) -> bool {
        !self.state.withered && self.clock.bears(self.s.seasons_to_grown) && self.stage != Stage::Dying
    }

    /// The wood's colour and glow. Death drains the colour, then darkens what
    /// is left, and takes any glow with it.
    fn wood(&self) -> ([f32; 4], f32) {
        let c = self.s.color;
        if !self.state.withered {
            return (c, self.s.emissive);
        }
        let w = &self.s.wither;
        let k = 1.0 - w.darken.clamp(0.0, 1.0);
        let drain = w.drain.clamp(0.0, 1.0);
        // Towards the colour's own brightness, so draining does not also dim.
        // A species with a bark recipe has had this done to its texels too;
        // this is the flat colour's share of the same death.
        let grey = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
        let pull = |x: f32| (x + (grey - x) * drain) * k;
        // The factor multiplies the map, so doing the full dimming in both
        // places would square it. The bark keeps the darkening; the factor
        // only carries it when there is no bark to carry it.
        if self.s.bark.is_some() {
            return ([1.0, 1.0, 1.0, c[3]], self.s.emissive * k);
        }
        ([pull(c[0]), pull(c[1]), pull(c[2]), c[3]], self.s.emissive * k)
    }
}

/// Segments along a child scale with its length so twigs stay cheap.
fn child_segments(s: &Species, segments: u32, length: f32) -> u32 {
    let f = (length / s.height.max(0.01)).clamp(0.3, 1.0);
    ((segments as f32 * f).round() as u32).max(3)
}

/// Point, direction, radius and sway at parameter `t` (0..1) along a branch.
fn sample(b: &Branch, t: f32) -> ([f32; 3], [f32; 3], f32, f32) {
    let n = b.pts.len();
    let f = (t.clamp(0.0, 1.0) * (n - 1) as f32).min((n - 1) as f32 - 1e-4);
    let i = f.floor() as usize;
    let u = f - i as f32;
    let a = b.pts[i];
    let c = b.pts[(i + 1).min(n - 1)];
    let p = [a[0] + (c[0] - a[0]) * u, a[1] + (c[1] - a[1]) * u, a[2] + (c[2] - a[2]) * u];
    let d = norm([c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
    let rad = b.radii[i] + (b.radii[(i + 1).min(n - 1)] - b.radii[i]) * u;
    let sw = b.sway[i] + (b.sway[(i + 1).min(n - 1)] - b.sway[i]) * u;
    (p, d, rad, sw)
}

/// The wind weights at parameter `t` along a branch: `(trunk, branch)`.
fn sample_wind(b: &Branch, t: f32) -> [f32; 2] {
    let n = b.pts.len();
    let f = (t.clamp(0.0, 1.0) * (n - 1) as f32).min((n - 1) as f32 - 1e-4);
    let i = f.floor() as usize;
    let u = f - i as f32;
    let j = (i + 1).min(n - 1);
    [b.trunk_w[i] + (b.trunk_w[j] - b.trunk_w[i]) * u, b.branch_w[i] + (b.branch_w[j] - b.branch_w[i]) * u]
}

// ── Meshing ─────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
fn push_vertex(m: &mut MeshData, p: [f32; 3], n: [f32; 3], uv: [f32; 2], sway: f32, branch: u32, wind: [f32; 2], phase: f32) {
    m.positions.push(p);
    m.normals.push(n);
    m.uvs.push(uv);
    // Every vertex names its branch, so a swing can be hit-tested in a game
    // without asking anyone which branch it struck.
    m.uv2.push(branch_uv(branch));
    // …and carries its wind: how much of the trunk's lean and of its own
    // limb's swing reach it, and the limb's phase. Wood does not flutter.
    m.uv3.push(wind);
    m.uv4.push([0.0, phase]);
    let alt = if n[0].abs() > 0.9 { [0.0, 0.0, 1.0] } else { [1.0, 0.0, 0.0] };
    let d = dot(alt, n);
    let t = norm([alt[0] - n[0] * d, alt[1] - n[1] * d, alt[2] - n[2] * d]);
    m.tangents.push([t[0], t[1], t[2], 1.0]);
    // The engine's convention: alpha 1.0 is rigid, and every root is rigid.
    m.colors.push([1.0, 1.0, 1.0, 1.0 - sway.clamp(0.0, 1.0)]);
}

/// Tube a branch with parallel-transported frames so the UV seam never twists.
fn tube(m: &mut MeshData, b: &Branch, sides: u32, uv_scale: f32) {
    let sides = sides.max(3) as usize;
    let n = b.pts.len();
    let mut frame = perp(norm(sub(b.pts[1], b.pts[0])));
    let base = m.positions.len() as u32;
    let mut v_along = 0.0f32;
    for i in 0..n {
        let mut d = if i + 1 < n { norm(sub(b.pts[i + 1], b.pts[i])) } else { norm(sub(b.pts[i], b.pts[i - 1])) };
        if b.level == 0 && i == 0 {
            // The root ring lies flat on the ground however the trunk leans:
            // a tree rests on y = 0, and a layout engine files it as `base`.
            d = [0.0, 1.0, 0.0];
        }
        // Parallel transport: remove the tangent component, renormalise.
        frame = norm(sub(frame, scale(d, dot(frame, d))));
        let side2 = cross(d, frame);
        if i > 0 {
            v_along += len(sub(b.pts[i], b.pts[i - 1])) * uv_scale;
        }
        let rad = b.radii[i].max(0.004);
        for s in 0..=sides {
            let a = s as f32 / sides as f32 * std::f32::consts::TAU;
            let (sa, ca) = a.sin_cos();
            let nrm = norm(add(scale(frame, ca), scale(side2, sa)));
            let p = add(b.pts[i], scale(nrm, rad));
            push_vertex(
                m,
                p,
                nrm,
                [s as f32 / sides as f32 * (rad * 6.0).max(0.5), v_along],
                b.sway[i],
                b.key,
                [b.trunk_w[i], b.branch_w[i]],
                b.phase,
            );
        }
    }
    let ring = (sides + 1) as u32;
    for i in 0..(n as u32 - 1) {
        for s in 0..sides as u32 {
            let a = base + i * ring + s;
            let b2 = a + 1;
            let c = a + ring;
            let d = c + 1;
            m.indices.extend_from_slice(&[a, c, b2, b2, c, d]);
        }
    }
    // Close the tip with a small fan so LOD silhouettes have no hole.
    let tip_i = n - 1;
    let tip_centre = m.positions.len() as u32;
    let d = norm(sub(b.pts[tip_i], b.pts[tip_i - 1]));
    push_vertex(m, b.pts[tip_i], d, [0.5, v_along], b.sway[tip_i], b.key, [b.trunk_w[tip_i], b.branch_w[tip_i]], b.phase);
    let last_ring = base + (n as u32 - 1) * ring;
    for s in 0..sides as u32 {
        m.indices.extend_from_slice(&[last_ring + s, last_ring + s + 1, tip_centre]);
    }
}

/// The branches with no children among those drawn — the plant's real ends,
/// whatever its age and whatever this LOD keeps. A stump is not an end: it is
/// where an end used to be.
fn tips(drawn: &[&Branch]) -> Vec<u32> {
    let parents: HashSet<u32> = drawn.iter().map(|b| b.parent).collect();
    drawn.iter().filter(|b| !b.stump && !parents.contains(&b.key)).map(|b| b.key).collect()
}

/// The wood mesh, the branches this LOD drew, and the leaf mesh (when the
/// species has leaves) at one level of detail.
fn mesh_for(at: &Moment, detail: f32) -> (MeshData, Vec<Branch>, Option<MeshData>) {
    let s = at.s;
    let sides = ((s.sides as f32 * detail).round() as u32).max(3);
    let segments = ((s.segments as f32 * detail).round() as u32).max(2);
    let present = at_clock(s, &potential(s, at.seed, at.state.withered, segments), at.maturity);
    let present = at_state(present, &at.state.cut, at.part);
    let outer = present.iter().map(|b| b.level).max().unwrap_or(0);
    // Coarser LODs drop the outermost generation(s) — the silhouette keeps.
    let max_level = if detail < 0.35 { outer.saturating_sub(1) } else { outer };
    let drawn: Vec<&Branch> = present.iter().filter(|b| b.level <= max_level).collect();
    let mut mesh = MeshData::default();
    let uv_scale = 1.0 / (s.trunk_radius * 6.0).max(0.2);
    for b in &drawn {
        tube(&mut mesh, b, sides, uv_scale);
    }
    // Leaves gather where the drawn wood ends: at every real tip, and back
    // along the last generations the species carries them on. How many is the
    // year's business — and a crown thinned to nothing is no crown at all.
    let leafiness = at.leafiness();
    let leaf_mesh = s.leaves.as_ref().and_then(|lr| {
        if leafiness <= 0.0 {
            return None;
        }
        let turned = lr.turned(at.clock.autumn());
        let ends: HashSet<u32> = tips(&drawn).into_iter().collect();
        let first_level = (max_level + 1).saturating_sub(lr.depth.max(1)).min(max_level);
        let thin = |n: u32| -> u32 {
            if n == 0 {
                0
            } else {
                (n as f32 * leafiness).round() as u32
            }
        };
        let mut sites: Vec<LeafSite> = Vec::new();
        for b in drawn.iter().filter(|b| b.level >= first_level && !b.stump) {
            let n = b.pts.len();
            if ends.contains(&b.key) {
                sites.push(LeafSite {
                    key: child_key(b.key, KIND_LEAF_TIP, 0),
                    branch: b.key,
                    wind: [b.trunk_w[n - 1], b.branch_w[n - 1]],
                    phase: b.phase,
                    position: b.pts[n - 1],
                    direction: norm(sub(b.pts[n - 1], b.pts[n - 2])),
                    sway: b.sway[n - 1],
                    count: thin(lr.per_tip),
                });
            }
            if lr.along > 0.0 && lr.along_count > 0 {
                // Along the twig the clusters are smaller than at its tip.
                let per = (lr.per_tip / 2).max(1);
                for k in 0..lr.along_count {
                    let t = 1.0 - lr.along * (k as f32 + 0.5) / lr.along_count as f32;
                    let (p, d, _, sw) = sample(b, t);
                    sites.push(LeafSite {
                        key: child_key(b.key, KIND_LEAF_ALONG, k),
                        branch: b.key,
                        wind: sample_wind(b, t),
                        phase: b.phase,
                        position: p,
                        direction: d,
                        sway: sw,
                        count: thin(per),
                    });
                }
            }
        }
        let mesh = leaves(&sites, &turned, detail, at.seed);
        (!mesh.positions.is_empty()).then_some(mesh)
    });
    let drawn: Vec<Branch> = drawn.into_iter().cloned().collect();
    (mesh, drawn, leaf_mesh)
}

/// The parts of one LOD: the wood, then the leaves when there are any.
fn parts_for(
    at: &Moment,
    wood: MeshData,
    leaf_mesh: Option<MeshData>,
    bark: &Option<chisel::texture::Baked>,
    leaf_baked: &Option<chisel::texture::Baked>,
) -> Vec<BuiltPart> {
    let (color, emissive) = at.wood();
    let mut parts = vec![BuiltPart {
        name: "wood".into(),
        mesh: wood,
        baked: bark.clone(),
        color,
        emissive,
        finish: Default::default(), double_sided: false,
    }];
    if let (Some(lm), Some(lr)) = (leaf_mesh, at.s.leaves.as_ref()) {
        // Vertex colour carries the base→tip gradient, so the material is white.
        parts.push(BuiltPart {
            name: "leaves".into(),
            mesh: lm,
            baked: leaf_baked.clone(),
            color: [1.0, 1.0, 1.0, 1.0],
            emissive: lr.emissive,
            finish: Default::default(), double_sided: true,
        });
    }
    parts
}

/// The bark of a plant that died standing.
///
/// The colour factor cannot do this on its own: a grey factor over a blue
/// bark map is a darker blue, which reads as a crystal tree at night rather
/// than a dead one. So death reaches the texels — the albedo drains towards
/// its own brightness and then dims. The normal and ORM maps are left alone:
/// dead wood keeps its grain, and it is the light that has gone, not the
/// relief.
fn withered_bark(mut baked: chisel::texture::Baked, w: &WitherRecipe) -> chisel::texture::Baked {
    let k = 1.0 - w.darken.clamp(0.0, 1.0);
    let drain = w.drain.clamp(0.0, 1.0);
    for px in baked.albedo.chunks_exact_mut(4) {
        let (r, g, b) = (px[0] as f32, px[1] as f32, px[2] as f32);
        let grey = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        for (i, c) in [r, g, b].into_iter().enumerate() {
            px[i] = ((c + (grey - c) * drain) * k).clamp(0.0, 255.0) as u8;
        }
    }
    baked
}

/// Is `season` inside a window that may wrap the turn of the year?
fn in_window(season: f32, w: [f32; 2]) -> bool {
    let (s, a, b) = (Clock { age: None, season }.year(), w[0], w[1]);
    if a <= b {
        s >= a && s < b
    } else {
        s >= a || s < b
    }
}

/// The crop sockets a species carries at this moment.
///
/// None out of season, none on a plant too young to bear, none on a dying or
/// a withered one — the fruit has dropped. Which tips carry a crop is each
/// tip's own answer, addressed on its branch id, so the same tips bear every
/// year and picking one says nothing about the others.
fn crop_sockets(at: &Moment, branches: &[Branch], ends: &HashSet<u32>, kind: SocketKind) -> Vec<Socket> {
    let crop = match kind {
        SocketKind::Bloom => at.s.blooms.as_ref(),
        SocketKind::Fruit => at.s.fruit.as_ref(),
        _ => None,
    };
    let Some(crop) = crop else { return Vec::new() };
    if !at.bears() || !in_window(at.clock.season, crop.window) {
        return Vec::new();
    }
    let slot = if kind == SocketKind::Bloom { SLOT_BLOOM_SHARE } else { SLOT_FRUIT_SHARE };
    let share = crop.share.clamp(0.0, 1.0);
    let per = crop.per_tip.max(1);
    let outer = branches.iter().map(|b| b.level).max().unwrap_or(0);
    let first_level = (outer + 1).saturating_sub(crop.depth.max(1)).min(outer);
    let mut out = Vec::new();
    for b in branches.iter().filter(|b| ends.contains(&b.key) && b.level >= first_level) {
        if unit(at.seed, b.key, slot) >= share {
            continue;
        }
        for i in 0..per {
            let t = (1.0 - crop.along * i as f32 / per as f32).clamp(0.0, 1.0);
            let (p, d, rad, _) = sample(b, t);
            out.push(Socket {
                name: format!("{}-{:08x}-{i}", kind.prefix(), b.key),
                kind,
                position: p,
                direction: d,
                radius: rad,
                level: b.level,
                branch: b.key,
            });
        }
    }
    out
}

/// Grow one individual of a species at one moment, in whatever state it is in:
/// LOD0 as a [`Built`] for preview/export, every LOD, and the sockets — the
/// tips it actually has, plus whatever the year has put on them.
pub fn grow(species: &Species, seed: u32, clock: Clock, state: &State) -> Result<Grown, String> {
    grow_part(species, seed, clock, state, Part::Standing)
}

/// The part that fell when `branch` was cut: that branch and everything that
/// grew from it, grown exactly as it stands on the tree — same seed, same
/// clock, same state, the same leaves and fruit — and brought to its own base,
/// so a game spawns it at the `cut` socket's position with no rotation and it
/// lines up with the wound.
///
/// The branch must be one the plant has at this moment. It need not be in
/// `state.cut` yet — this is what a cut *would* drop.
pub fn fallen(species: &Species, seed: u32, clock: Clock, state: &State, branch: u32) -> Result<Grown, String> {
    if branch == TRUNK {
        return Err("the trunk cannot be cut at a joint — felling is a different verb".into());
    }
    let g = grow_part(species, seed, clock, state, Part::Fallen(branch))?;
    if g.built.parts[0].mesh.positions.is_empty() {
        return Err(format!("branch {branch:08x} is not on this plant at this moment"));
    }
    Ok(g)
}

fn grow_part(species: &Species, seed: u32, clock: Clock, state: &State, part: Part) -> Result<Grown, String> {
    if species.height <= 0.0 || species.trunk_radius <= 0.0 {
        return Err("height and trunk_radius must be positive".into());
    }
    let at = Moment::new(species, seed, clock, state, part);
    let bark = species
        .bark
        .as_ref()
        .map(chisel::texture::bake)
        .map(|b| if state.withered { withered_bark(b, &species.wither) } else { b });
    let leaf_baked = species.leaves.as_ref().and_then(|l| l.texture.as_ref()).map(chisel::texture::bake);
    let (lod0, branches, leaf0) = mesh_for(&at, 1.0);
    let mut lods: Vec<Built> = Vec::new();
    for &f in &species.lods {
        if f > 0.0 && f < 1.0 {
            let (wood, _, lm) = mesh_for(&at, f);
            lods.push(Built {
                name: format!("{}-lod{}", species.name, lods.len() + 1),
                parts: parts_for(&at, wood, lm, &bark, &leaf_baked),
            });
        }
    }
    // Sockets: every branch that has no children yet — the plant's real ends
    // at this age, named by the branch so state can name them back — and then
    // what the year hangs on them.
    let ends: HashSet<u32> = tips(&branches.iter().collect::<Vec<_>>()).into_iter().collect();
    let mut sockets = Vec::new();
    for b in branches.iter().filter(|b| b.stump || ends.contains(&b.key)) {
        let n = b.pts.len();
        // A stump's end is the wound: the `cut` socket sits there, facing the
        // way the branch went, so the fallen part lines up with it.
        let kind = if b.stump { SocketKind::Cut } else { SocketKind::Tip };
        sockets.push(Socket {
            name: format!("{}-{:08x}", kind.prefix(), b.key),
            kind,
            position: b.pts[n - 1],
            direction: norm(sub(b.pts[n - 1], b.pts[n - 2])),
            radius: b.radii[n - 1],
            level: b.level,
            branch: b.key,
        });
    }
    sockets.extend(crop_sockets(&at, &branches, &ends, SocketKind::Bloom));
    sockets.extend(crop_sockets(&at, &branches, &ends, SocketKind::Fruit));
    // Picked is picked: the socket is simply not there.
    sockets.retain(|k| !state.is_taken(&k.name, clock));
    sockets.sort_by(|a, b| a.name.cmp(&b.name));
    let info: Vec<BranchInfo> = branches
        .iter()
        .map(|b| BranchInfo {
            id: b.key,
            parent: if b.key == TRUNK { TRUNK } else { b.parent },
            level: b.level,
            base: b.pts[0],
            tip: *b.pts.last().unwrap(),
            radius: b.radii[0],
            stump: b.stump,
        })
        .collect();
    let built = Built { name: species.name.clone(), parts: parts_for(&at, lod0, leaf0, &bark, &leaf_baked) };
    let bounds = built.bounds();
    Ok(Grown { built, lods, sockets, branches: info, bounds, maturity: at.maturity, stage: at.stage, phase: clock.phase() })
}

/// Grow what a recipe file or a Quarry submission holds: species, seed, clock
/// and state in one value.
pub fn grow_planting(p: &Planting) -> Result<Grown, String> {
    grow(&p.species, p.seed, p.clock, &p.state)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grown(species: &Species, seed: u32) -> Grown {
        grow(species, seed, Clock::grown(), &State::thriving()).unwrap()
    }

    fn leafy() -> Species {
        Species { leaves: Some(LeafRecipe::default()), ..Default::default() }
    }

    #[test]
    fn a_default_tree_grows_branches_and_sockets() {
        let g = grown(&Species::default(), 1);
        assert!(g.built.triangles() > 500, "tris {}", g.built.triangles());
        assert_eq!(g.lods.len(), 2, "two coarser LODs");
        assert!(g.lods[0].triangles() < g.built.triangles());
        assert!(g.lods[1].triangles() < g.lods[0].triangles());
        assert!(g.sockets.len() >= 8, "sockets {}", g.sockets.len());
        assert_eq!(g.maturity, 1.0);
        // Root rigid, tips sway: the engine's alpha convention.
        let m = &g.built.parts[0].mesh;
        assert!(m.colors[0][3] > 0.999);
        let min_alpha = m.colors.iter().map(|c| c[3]).fold(1.0f32, f32::min);
        assert!(min_alpha < 0.7, "tips should sway (min alpha {min_alpha})");
        // Every socket names its branch.
        for s in &g.sockets {
            assert_eq!(s.name, format!("tip-{:08x}", s.branch));
        }
    }

    #[test]
    fn same_seed_same_plant() {
        let s = Species::default();
        let a = grown(&s, 1);
        let b = grown(&s, 1);
        assert_eq!(a.built.parts[0].mesh.positions, b.built.parts[0].mesh.positions);
        assert_eq!(
            a.sockets.iter().map(|x| x.branch).collect::<Vec<_>>(),
            b.sockets.iter().map(|x| x.branch).collect::<Vec<_>>()
        );
        let c = grown(&s, 99);
        assert_ne!(a.built.parts[0].mesh.positions, c.built.parts[0].mesh.positions);
        // …and at a young age too, which is where a consumed stream used to drift.
        let y1 = grow(&s, 1, Clock::at(3.0), &State::thriving()).unwrap();
        let y2 = grow(&s, 1, Clock::at(3.0), &State::thriving()).unwrap();
        assert_eq!(y1.built.parts[0].mesh.positions, y2.built.parts[0].mesh.positions);
    }

    #[test]
    fn the_seed_decides_the_whole_potential_plant() {
        // Every branch a young plant has is the branch the grown plant has —
        // the same identity, pointing the same way, only shorter and thinner.
        let s = Species::default();
        let all = potential(&s, 7, false, s.segments);
        for age in [1.0f32, 3.0, 6.0, 9.0, 11.5] {
            let m = Clock::at(age).maturity(s.seasons_to_grown);
            let now = at_clock(&s, &all, m);
            assert!(!now.is_empty(), "age {age} has no plant at all");
            for b in &now {
                let full = all.iter().find(|x| x.key == b.key).expect("a branch not in the potential plant");
                assert_eq!(b.level, full.level);
                assert_eq!(b.parent, full.parent);
                // Direction at the base is untouched by scaling.
                let d_now = norm(sub(b.pts[1], b.pts[0]));
                let d_full = norm(sub(full.pts[1], full.pts[0]));
                for k in 0..3 {
                    assert!((d_now[k] - d_full[k]).abs() < 1e-3, "branch {:08x} turned: {d_now:?} vs {d_full:?}", b.key);
                }
                // It is a prefix: never longer than the branch it will be.
                let l_now: f32 = (1..b.pts.len()).map(|i| len(sub(b.pts[i], b.pts[i - 1]))).sum();
                let l_full: f32 = (1..full.pts.len()).map(|i| len(sub(full.pts[i], full.pts[i - 1]))).sum();
                assert!(l_now <= l_full + 1e-4, "branch {:08x} longer than its potential", b.key);
            }
        }
    }

    #[test]
    fn age_only_ever_adds_branches() {
        let s = Species::default();
        let all = potential(&s, 12, false, s.segments);
        let keys = |age: f32| -> HashSet<u32> {
            at_clock(&s, &all, Clock::at(age).maturity(s.seasons_to_grown)).iter().map(|b| b.key).collect()
        };
        let mut prev = keys(0.0);
        for age in [1.0f32, 2.0, 4.0, 6.0, 8.0, 10.0, 12.0] {
            let now = keys(age);
            assert!(prev.is_subset(&now), "ageing to {age} lost branches");
            prev = now;
        }
        // A grown plant is the potential plant, branch for branch.
        assert_eq!(prev.len(), all.len(), "the grown plant is the whole potential plant");
    }

    #[test]
    fn a_sapling_is_smaller_and_simpler() {
        let s = Species::default();
        let young = grow(&s, 4, Clock::at(2.0), &State::thriving()).unwrap();
        let old = grown(&s, 4);
        assert!(young.built.triangles() < old.built.triangles(), "a sapling has less wood");
        let h = |g: &Grown| g.bounds.1[1] - g.bounds.0[1];
        assert!(h(&young) < h(&old) * 0.6, "sapling {:.2} m vs grown {:.2} m", h(&young), h(&old));
        assert!(h(&young) > 0.0);
        assert!(young.maturity < old.maturity);
        // Growing up never shrinks the plant.
        let mut last = 0.0;
        for age in [0.5f32, 1.0, 2.0, 4.0, 8.0, 12.0, 40.0] {
            let g = grow(&s, 4, Clock::at(age), &State::thriving()).unwrap();
            let now = h(&g);
            assert!(now >= last - 1e-4, "age {age} shrank: {now} < {last}");
            last = now;
        }
        // Past its life curve a plant is simply grown, not bigger.
        let ancient = grow(&s, 4, Clock::at(400.0), &State::thriving()).unwrap();
        assert_eq!(ancient.built.parts[0].mesh.positions, old.built.parts[0].mesh.positions);
    }

    #[test]
    fn a_sapling_still_has_tips_to_hang_things_on() {
        let s = leafy();
        for age in [0.5f32, 2.0, 5.0, 12.0] {
            let g = grow(&s, 3, Clock::at(age), &State::thriving()).unwrap();
            assert!(!g.sockets.is_empty(), "age {age} has no sockets");
            assert_eq!(g.built.parts.len(), 2, "age {age} lost its leaves");
            assert!(g.built.parts[1].mesh.positions.len() >= 5, "age {age} has no leaves");
        }
    }

    #[test]
    fn withered_is_the_same_tree_gone_further() {
        let s = leafy();
        let alive = potential(&s, 7, false, s.segments);
        let dead = potential(&s, 7, true, s.segments);
        assert_eq!(alive.len(), dead.len(), "death takes no branches away");
        let length = |b: &Branch| -> f32 { (1..b.pts.len()).map(|i| len(sub(b.pts[i], b.pts[i - 1]))).sum() };
        for (a, d) in alive.iter().zip(dead.iter()) {
            assert_eq!(a.key, d.key, "the same branches in the same order");
            assert_eq!(a.parent, d.parent);
            assert_eq!(a.level, d.level);
            assert!((length(a) - length(d)).abs() < 1e-3, "death bends a branch, it does not shorten it");
        }
        // It bends: the crown is not where it was.
        assert_ne!(alive[1].pts.last().unwrap()[1], dead[1].pts.last().unwrap()[1]);

        let g = grow(&s, 7, Clock::grown(), &State::withered()).unwrap();
        assert_eq!(g.built.parts.len(), 1, "no leaves on a withered tree");
        assert!(!g.sockets.is_empty(), "a bare tree still has ends");
        assert!(g.sockets.iter().all(|k| k.kind == SocketKind::Tip), "no crop on a withered tree");
        // Darker wood, and any glow goes out with it.
        let lit = Species { emissive: 0.8, ..s.clone() };
        let alive_wood = grown(&lit, 7).built.parts[0].color;
        let dead_built = grow(&lit, 7, Clock::grown(), &State::withered()).unwrap();
        assert!(dead_built.built.parts[0].color[0] < alive_wood[0]);
        assert!(dead_built.built.parts[0].emissive < 0.8);
    }

    #[test]
    fn death_reaches_the_bark_itself() {
        // A blue crystal bark, the Lantern Desert's own.
        let bark: infinite_manifest::texture::TextureRecipe = serde_json::from_str(
            r#"{"kind":"veins","size":64,"colors":[[0.2,0.44,0.74],[0.78,0.92,1.0],[0.32,0.62,0.88]]}"#,
        )
        .unwrap();
        let baked = chisel::texture::bake(&bark);
        let dead = withered_bark(baked.clone(), &WitherRecipe::default());
        assert_eq!(dead.normal, baked.normal, "dead wood keeps its grain");
        assert_eq!(dead.orm, baked.orm);
        let spread = |b: &chisel::texture::Baked| -> f32 {
            // Mean distance from grey: how much colour the map is carrying.
            let mut sum = 0.0;
            for px in b.albedo.chunks_exact(4) {
                let (r, g, bl) = (px[0] as f32, px[1] as f32, px[2] as f32);
                let m = (r + g + bl) / 3.0;
                sum += ((r - m).abs() + (g - m).abs() + (bl - m).abs()) / 3.0;
            }
            sum / (b.albedo.len() / 4) as f32
        };
        let brightness = |b: &chisel::texture::Baked| -> f32 {
            b.albedo.chunks_exact(4).map(|p| (p[0] as f32 + p[1] as f32 + p[2] as f32) / 3.0).sum::<f32>()
                / (b.albedo.len() / 4) as f32
        };
        assert!(spread(&dead) < spread(&baked) * 0.5, "the colour drains");
        assert!(brightness(&dead) < brightness(&baked) * 0.6, "and what is left dims");
    }

    #[test]
    fn a_crop_is_in_season_or_it_is_not() {
        let s = Species { fruit: Some(CropRecipe { share: 0.6, ..Default::default() }), ..leafy() };
        let fruit = |clock: Clock, state: State| -> Vec<String> {
            grow(&s, 5, clock, &state)
                .unwrap()
                .sockets
                .into_iter()
                .filter(|k| k.kind == SocketKind::Fruit)
                .map(|k| k.name)
                .collect()
        };
        let in_season = fruit(Clock::grown().in_season(0.5), State::thriving());
        assert!(!in_season.is_empty(), "a grown tree fruits in the fruit window");
        assert!(fruit(Clock::grown().in_season(0.2), State::thriving()).is_empty(), "not in spring");
        assert!(fruit(Clock::grown().in_season(0.95), State::thriving()).is_empty(), "not in winter");
        assert!(fruit(Clock::at(3.0).in_season(0.5), State::thriving()).is_empty(), "a sapling bears nothing");
        assert!(fruit(Clock::at(1e5).in_season(0.5), State::thriving()).is_empty(), "a dying tree bears nothing");
        assert!(fruit(Clock::grown().in_season(0.5), State::withered()).is_empty(), "the fruit has dropped");
        // The same tips bear, this year and next — and only some of them.
        assert_eq!(in_season, fruit(Clock::grown().in_season(1.5), State::thriving()), "next year is this year");
        let tips = grown(&s, 5).sockets.iter().filter(|k| k.kind == SocketKind::Tip).count();
        assert!(in_season.len() < tips, "only a share of the tips bear ({} of {tips})", in_season.len());
        // A crop is sockets, not geometry: the wood and the crown are untouched.
        let bare_season = grow(&s, 5, Clock::grown().in_season(0.5), &State::thriving()).unwrap();
        let no_fruit = grow(&Species { fruit: None, ..s.clone() }, 5, Clock::grown().in_season(0.5), &State::thriving()).unwrap();
        assert_eq!(bare_season.built.parts[0].mesh.positions, no_fruit.built.parts[0].mesh.positions);
    }

    #[test]
    fn the_year_dresses_the_crown() {
        let s = leafy();
        let crown = |season: f32| -> usize {
            grow(&s, 9, Clock::grown().in_season(season), &State::thriving())
                .unwrap()
                .built
                .parts
                .get(1)
                .map(|p| p.mesh.positions.len())
                .unwrap_or(0)
        };
        assert_eq!(crown(0.92), 0, "bare in winter");
        assert!(crown(0.05) > 0 && crown(0.05) < crown(0.35), "budding");
        assert_eq!(crown(0.35), crown(0.6), "full through bloom and fruit");
        assert!(crown(0.78) > 0 && crown(0.78) < crown(0.6), "falling");
        // Turning is colour, not geometry: the leaves that remain are the same leaves.
        let summer = grow(&s, 9, Clock::grown().in_season(0.5), &State::thriving()).unwrap();
        let turning = grow(&s, 9, Clock::grown().in_season(0.72), &State::thriving()).unwrap();
        // Every leaf still on the tree is one the full crown had, in the same
        // place: the year sheds leaves, it does not grow a different crown.
        let bits = |p: &[f32; 3]| (p[0].to_bits(), p[1].to_bits(), p[2].to_bits());
        let full: HashSet<_> = summer.built.parts[1].mesh.positions.iter().map(bits).collect();
        assert!(turning.built.parts[1].mesh.positions.iter().all(|p| full.contains(&bits(p))));
        assert_ne!(turning.built.parts[1].mesh.colors[0], summer.built.parts[1].mesh.colors[0], "and they have turned");
        // An evergreen ignores the year.
        let ever = Species { evergreen: true, ..s.clone() };
        let ever_crown = |season: f32| {
            grow(&ever, 9, Clock::grown().in_season(season), &State::thriving()).unwrap().built.parts[1].mesh.positions.len()
        };
        assert_eq!(ever_crown(0.92), ever_crown(0.35));
    }

    #[test]
    fn an_old_tree_is_the_same_tree_with_less_crown() {
        let s = leafy();
        let at = |age: f32| grow(&s, 6, Clock::at(age), &State::thriving()).unwrap();
        let (mature, old, dying) = (at(40.0), at(130.0), at(158.0));
        assert_eq!(mature.stage, Stage::Mature);
        assert_eq!(old.stage, Stage::Old);
        assert_eq!(dying.stage, Stage::Dying);
        // The wood is untouched — age past full size takes leaves, not branches.
        assert_eq!(mature.built.parts[0].mesh.positions, old.built.parts[0].mesh.positions);
        assert_eq!(mature.sockets.len(), dying.sockets.len());
        let crown = |g: &Grown| g.built.parts[1].mesh.positions.len();
        assert!(crown(&old) < crown(&mature), "an old tree thins");
        assert!(crown(&dying) < crown(&old), "a dying one is mostly bare");
        assert!(crown(&dying) > 0);
    }

    /// The branch ids a mesh's vertices name.
    fn named(m: &MeshData) -> HashSet<u32> {
        m.uv2.iter().map(|uv| branch_from_uv(*uv)).collect()
    }

    /// A level-1 fork of the default tree at seed 5: a limb worth chopping.
    fn a_limb(s: &Species) -> u32 {
        potential(s, 5, false, s.segments).iter().find(|b| b.level == 1 && b.attach >= 1.0).unwrap().key
    }

    #[test]
    fn every_vertex_names_its_branch() {
        let s = leafy();
        let g = grown(&s, 5);
        let wood = &g.built.parts[0].mesh;
        assert_eq!(wood.uv2.len(), wood.positions.len(), "one id per wood vertex");
        let keys: HashSet<u32> = potential(&s, 5, false, s.segments).iter().map(|b| b.key).collect();
        let wood_keys = named(wood);
        assert!(wood_keys.is_subset(&keys), "a vertex names a branch the plant has");
        assert_eq!(wood_keys.len(), keys.len(), "every branch is named by its vertices");
        for k in [0u32, 1, 0xffff, 0x1_0000, 0xdead_beef, u32::MAX] {
            assert_eq!(branch_from_uv(branch_uv(k)), k, "{k:#x} survives the channel");
        }
        // Leaves name the twig they grow on, and nothing else.
        let leaves = &g.built.parts[1].mesh;
        assert_eq!(leaves.uv2.len(), leaves.positions.len());
        assert!(named(leaves).is_subset(&keys));
        // The channel reaches the LODs too.
        for lod in &g.lods {
            assert_eq!(lod.parts[0].mesh.uv2.len(), lod.parts[0].mesh.positions.len());
        }
    }

    #[test]
    fn the_wind_rides_in_four_channels() {
        let s = leafy();
        let g = grown(&s, 5);
        let wood = &g.built.parts[0].mesh;
        assert_eq!(wood.uv3.len(), wood.positions.len());
        assert_eq!(wood.uv4.len(), wood.positions.len());
        // Alpha is still rigidity for everything that reads it.
        assert!(wood.colors[0][3] > 0.999);
        // Wood does not flutter; every phase is a fraction of a swing.
        assert!(wood.uv4.iter().all(|w| w[0] == 0.0));
        assert!(wood.uv4.iter().all(|w| (0.0..=1.0).contains(&w[1])));
        // The trunk: its own weight climbs from the ground, its branch weight is nil.
        let trunk: Vec<usize> = (0..wood.positions.len()).filter(|i| branch_from_uv(wood.uv2[*i]) == TRUNK).collect();
        let lowest = trunk.iter().copied().min_by(|a, b| wood.positions[*a][1].total_cmp(&wood.positions[*b][1])).unwrap();
        let highest = trunk.iter().copied().max_by(|a, b| wood.positions[*a][1].total_cmp(&wood.positions[*b][1])).unwrap();
        assert!(wood.uv3[lowest][0] < 0.05, "rigid at the ground");
        assert!(wood.uv3[highest][0] > 0.9, "leaning at the top");
        assert!(trunk.iter().all(|i| wood.uv3[*i][1] == 0.0), "a trunk has no limb to swing");
        // The tips: they carry the trunk's lean and the most limb swing.
        let tips: Vec<usize> = (0..wood.positions.len())
            .filter(|i| g.sockets.iter().any(|k| k.kind == SocketKind::Tip && k.branch == branch_from_uv(wood.uv2[*i])))
            .collect();
        let max_branch = tips.iter().map(|i| wood.uv3[*i][1]).fold(0.0f32, f32::max);
        assert!(max_branch > 0.9, "the outermost twigs swing most ({max_branch})");
        assert!(tips.iter().all(|i| wood.uv3[*i][0] > 0.0), "and every limb leans with the trunk");
        // Two limbs, two phases.
        let phases: HashSet<u32> = wood.uv4.iter().map(|w| (w[1] * 1000.0) as u32).collect();
        assert!(phases.len() > 4);
        // Leaves flutter, and carry their twig's wind and phase.
        let leaves = &g.built.parts[1].mesh;
        assert_eq!(leaves.uv3.len(), leaves.positions.len());
        assert!(leaves.uv4.iter().all(|w| w[0] > 0.0), "a leaf flutters");
        // Ageing and cutting keep every column the same length.
        for age in [1.0f32, 4.0] {
            let y = grow(&s, 5, Clock::at(age), &State::thriving()).unwrap();
            let m = &y.built.parts[0].mesh;
            assert_eq!(m.uv3.len(), m.positions.len());
            assert_eq!(m.uv4.len(), m.positions.len());
        }
        let limb = a_limb(&s);
        let c = grow(&s, 5, Clock::grown(), &State::thriving().cut(limb)).unwrap();
        assert_eq!(c.built.parts[0].mesh.uv3.len(), c.built.parts[0].mesh.positions.len());
        // Same seed, same wind.
        assert_eq!(grown(&s, 5).built.parts[0].mesh.uv4, wood.uv4);
    }

    #[test]
    fn a_cut_takes_the_limb_and_leaves_a_socket_at_the_joint() {
        let s = leafy();
        let limb = a_limb(&s);
        let whole = grown(&s, 5);
        let cut = grow(&s, 5, Clock::grown(), &State::thriving().cut(limb)).unwrap();
        assert!(cut.built.triangles() < whole.built.triangles(), "less wood");
        assert!(cut.built.parts[1].mesh.positions.len() < whole.built.parts[1].mesh.positions.len(), "fewer leaves");
        // Everything that grew from the limb is gone; the limb itself is a stump.
        let all = potential(&s, 5, false, s.segments);
        let under: HashSet<u32> = {
            let parent: std::collections::HashMap<u32, u32> = all.iter().map(|b| (b.key, b.parent)).collect();
            all.iter()
                .map(|b| b.key)
                .filter(|&k| {
                    let mut c = k;
                    loop {
                        if c == limb {
                            break true;
                        }
                        match parent.get(&c) {
                            Some(&p) if c != TRUNK => c = p,
                            _ => break false,
                        }
                    }
                })
                .collect()
        };
        let left = named(&cut.built.parts[0].mesh);
        assert!(left.contains(&limb), "the stump still names the limb");
        assert!(under.iter().filter(|k| **k != limb).all(|k| !left.contains(k)), "nothing beyond the joint");
        assert!(named(&cut.built.parts[1].mesh).is_disjoint(&under), "no leaves on what fell");
        // One `cut` socket, at the joint, named for the branch; no tips under it.
        let cuts: Vec<&Socket> = cut.sockets.iter().filter(|k| k.kind == SocketKind::Cut).collect();
        assert_eq!(cuts.len(), 1);
        assert_eq!(cuts[0].name, format!("cut-{limb:08x}"));
        assert_eq!(cuts[0].branch, limb);
        let base = all.iter().find(|b| b.key == limb).unwrap().pts[0];
        assert!(len(sub(cuts[0].position, base)) < 0.3, "the socket sits at the joint");
        assert!(cut.sockets.iter().filter(|k| k.kind == SocketKind::Tip).all(|k| !under.contains(&k.branch)));
        // The trunk cannot be cut at a joint: asking changes nothing.
        let trunk = grow(&s, 5, Clock::grown(), &State::thriving().cut(TRUNK)).unwrap();
        assert_eq!(trunk.built.parts[0].mesh.positions, whole.built.parts[0].mesh.positions);
        // Same seed, same cut, same wound.
        let again = grow(&s, 5, Clock::grown(), &State::thriving().cut(limb)).unwrap();
        assert_eq!(again.built.parts[0].mesh.positions, cut.built.parts[0].mesh.positions);
    }

    #[test]
    fn the_fallen_part_is_the_limb_grown_on_its_own() {
        let s = leafy();
        let limb = a_limb(&s);
        let whole = grown(&s, 5);
        let cut = grow(&s, 5, Clock::grown(), &State::thriving().cut(limb)).unwrap();
        let part = fallen(&s, 5, Clock::grown(), &State::thriving(), limb).unwrap();
        // What fell plus what stands is the tree, branch for branch (the stump
        // is named twice: once as the wound, once as the base of what fell).
        let standing = named(&cut.built.parts[0].mesh);
        let fell = named(&part.built.parts[0].mesh);
        let all = named(&whole.built.parts[0].mesh);
        assert!(fell.contains(&limb));
        assert_eq!(&standing | &fell, all);
        assert_eq!(&standing & &fell, [limb].into_iter().collect::<HashSet<u32>>());
        // It stands on its own base, and it is the same wood, moved: put it
        // back at the joint and every vertex lands on the whole tree.
        let base = potential(&s, 5, false, s.segments).iter().find(|b| b.key == limb).unwrap().pts[0];
        let bits = |p: [f32; 3]| (p[0].to_bits(), p[1].to_bits(), p[2].to_bits());
        let tree: HashSet<_> = whole.built.parts[0].mesh.positions.iter().map(|p| bits(*p)).collect();
        let mut off = 0;
        for p in &part.built.parts[0].mesh.positions {
            let back = add(*p, base);
            // Float noise from the subtraction: compare to a hair.
            if !tree.contains(&bits(back)) {
                let near = whole.built.parts[0].mesh.positions.iter().any(|q| len(sub(*q, back)) < 1e-4);
                if !near {
                    off += 1;
                }
            }
        }
        assert_eq!(off, 0, "{off} vertices of the fallen part are not where the tree had them");
        let (min, _) = part.bounds;
        assert!(min[0] <= 0.0 && min[1] <= 0.0 && min[2] <= 0.0, "the base is at the origin");
        // The fruit went with it, and so did the leaves.
        assert_eq!(part.built.parts.len(), 2);
        assert!(part.sockets.iter().any(|k| k.kind == SocketKind::Tip));
        // A cut above the joint is still a cut inside the fallen part.
        let twig = potential(&s, 5, false, s.segments)
            .iter()
            .find(|b| b.level == 2 && b.parent == limb)
            .unwrap()
            .key;
        let part2 = fallen(&s, 5, Clock::grown(), &State::thriving().cut(twig), limb).unwrap();
        assert!(part2.built.triangles() < part.built.triangles());
        assert!(part2.sockets.iter().any(|k| k.kind == SocketKind::Cut && k.branch == twig));
        // Asking for what is not there is an error, not an empty file.
        assert!(fallen(&s, 5, Clock::grown(), &State::thriving(), TRUNK).is_err());
        assert!(fallen(&s, 5, Clock::at(0.5), &State::thriving(), limb).is_err(), "a seedling has no limb to lose");
    }

    #[test]
    fn a_taken_socket_is_gone_until_it_regrows() {
        let s = Species { fruit: Some(CropRecipe { share: 0.6, ..Default::default() }), ..leafy() };
        let summer = Clock::at(30.0).in_season(0.5);
        let first = grow(&s, 5, summer, &State::thriving())
            .unwrap()
            .sockets
            .into_iter()
            .find(|k| k.kind == SocketKind::Fruit)
            .unwrap()
            .name;
        let has = |clock: Clock, state: &State| grow(&s, 5, clock, state).unwrap().sockets.iter().any(|k| k.name == first);
        assert!(has(summer, &State::thriving()));
        let picked = State::thriving().take(&first, Some(34.0));
        assert!(!has(summer, &picked), "picked");
        assert!(has(Clock::at(34.0).in_season(0.5), &picked), "and back when the plant is old enough");
        assert!(!has(Clock::grown().in_season(0.5), &picked), "a plant with no age cannot have grown it back");
        let forever = State::thriving().take(&first, None);
        assert!(!has(Clock::at(1000.0).in_season(0.5), &forever), "no regrow time: the game strikes it");
        // The other sockets are none the wiser.
        let n = |state: &State| grow(&s, 5, summer, state).unwrap().sockets.len();
        assert_eq!(n(&picked) + 1, n(&State::thriving()));
    }

    #[test]
    fn a_planting_round_trips_flat() {
        let text = r#"{"name":"oak","seed":5,"height":2.2,"levels":2,"age":4.5}"#;
        let p = Planting::from_json(text).unwrap();
        assert_eq!(p.species.name, "oak");
        assert_eq!(p.seed, 5);
        assert_eq!(p.clock.age, Some(4.5));
        assert_eq!(p.species.levels, 2);
        let v = p.to_value();
        assert_eq!(v["seed"], 5);
        assert_eq!(v["age"], 4.5);
        let back = Planting::from_value(&v).unwrap();
        assert_eq!(back.seed, 5);
        assert_eq!(back.clock.age, Some(4.5));
        // A file with no clock is a grown plant, and says nothing about age.
        let grown_p = Planting::from_json(r#"{"name":"oak","seed":5}"#).unwrap();
        assert!(grown_p.clock.age.is_none());
        assert!(grown_p.to_value().get("age").is_none(), "a grown plant must not carry an age");
        // The default planting's flat form is what a design id strips against.
        let d = Planting::default().to_value();
        assert_eq!(d["seed"], 1);
        assert!(d.get("age").is_none());
        assert!(d.get("withered").is_none(), "a thriving plant must not carry a state");
        // A state is one flat key, and it survives the round trip.
        let dead = Planting::from_json(r#"{"name":"lantern-tree","seed":7,"withered":true}"#).unwrap();
        assert!(dead.state.withered);
        assert_eq!(dead.to_value()["withered"], true);
        assert!(Planting::from_value(&dead.to_value()).unwrap().state.withered);
        // Cuts and picks ride the same flat object.
        let hurt = Planting::from_json(
            r#"{"name":"oak","seed":5,"cut":[3735928559],"taken":[{"socket":"fruit-0000beef-0","until":20}]}"#,
        )
        .unwrap();
        assert_eq!(hurt.state.cut, vec![0xdead_beef]);
        assert_eq!(hurt.state.taken[0].until, Some(20.0));
        let v = hurt.to_value();
        assert_eq!(v["cut"][0], 3735928559u64);
        assert_eq!(v["taken"][0]["socket"], "fruit-0000beef-0");
        assert_eq!(Planting::from_value(&v).unwrap().state, hurt.state);
    }

    #[test]
    fn lods_are_the_same_plant() {
        let s = leafy();
        let g = grown(&s, 8);
        let (min, max) = g.bounds;
        for (i, lod) in g.lods.iter().enumerate() {
            let (lmin, lmax) = lod.bounds();
            for k in 0..3 {
                let span = max[k] - min[k];
                assert!(
                    (lmax[k] - max[k]).abs() < span * 0.35 && (lmin[k] - min[k]).abs() < span * 0.35,
                    "lod{} axis {k} silhouette drifted: {:?}..{:?} vs {:?}..{:?}",
                    i + 1,
                    lmin,
                    lmax,
                    min,
                    max
                );
            }
        }
    }
}
