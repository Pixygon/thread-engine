//! # Grow — trees and other branching things, from a recipe
//!
//! Chisel's carving vocabulary is axis-aligned and rotates about Y only, so
//! a tree came out as a cone with horizontal sticks (2026-09-25). Image-to-3D
//! drops thin branching entirely. Branching is its own kind of geometry and
//! deserves its own generator — the same way [`flora`](crate::flora) owns
//! *where* plants stand, `grow` owns *what one plant is*.
//!
//! A [`GrowRecipe`] is a bill of rules, not a list of vertices: how many
//! levels, how many children per branch, at what angle, how much shorter and
//! thinner each generation, how much they droop and twist, and what the bark
//! wears. From one recipe and one seed the same tree grows every time; a new
//! seed is a new individual of the same species. That is what makes trees
//! Quarry-publishable and re-derivable like every other model.
//!
//! What comes out:
//! - a tube mesh per LOD (fewer sides and levels as detail drops), with UVs
//!   that run around and along each branch so a bark recipe tiles cleanly;
//! - **wind in the vertex alpha**, the engine's convention (1.0 = rigid): the
//!   root is rigid, tips sway most, so the same mesh moves in Infinite and can
//!   be read by a Unity shader from the same channel;
//! - **sockets** at every terminal tip — position, direction, radius — so a
//!   fruit, a lantern, a leaf cluster, a Trellis-made hero prop can hang
//!   exactly where the tree ends, in a manifest or the Quarry's `facts`.
//!
//! Foliage (leaf cards, clusters, needles) is the next layer and will attach
//! at the same sockets; this file grows the skeleton and the wood.
use infinite_manifest::texture::TextureRecipe;
use serde::{Deserialize, Serialize};

use crate::model::{Built, BuiltPart};
use crate::MeshData;

/// Everything a tree needs to be told.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GrowRecipe {
    pub name: String,
    /// Determinism: same recipe + seed = same tree, everywhere.
    pub seed: u32,
    /// Trunk length in metres up to its first fork, before any droop.
    pub height: f32,
    /// Trunk radius at the ground.
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
    /// limbs arc instead of jitter. `wobble` is the noise on top.
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
    /// Random bend per segment, 0..1.
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
    /// Bark recipe. Absent = the flat `color`.
    pub bark: Option<TextureRecipe>,
    /// Flat colour when there is no bark recipe (RGBA).
    pub color: [f32; 4],
    /// Glow strength for the wood (crystal trees glow a little).
    pub emissive: f32,
    /// Detail fractions for LOD1, LOD2… (sides and segments scale by these).
    pub lods: Vec<f32>,
}

impl Default for GrowRecipe {
    fn default() -> Self {
        Self {
            name: "tree".into(),
            seed: 1,
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
            bark: None,
            color: [0.4, 0.3, 0.2, 1.0],
            emissive: 0.0,
            lods: vec![0.5, 0.25],
        }
    }
}

/// An attachment point at a branch tip.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Socket {
    pub name: String,
    pub position: [f32; 3],
    /// Unit vector the branch points along at its tip.
    pub direction: [f32; 3],
    pub radius: f32,
    /// Branching generation (0 = the trunk's own tip).
    pub level: u32,
}

/// One grown tree: the LOD0 model (for preview and single-mesh export), the
/// LOD meshes in order, and the sockets.
pub struct Grown {
    pub built: Built,
    pub lods: Vec<MeshData>,
    pub sockets: Vec<Socket>,
    pub bounds: ([f32; 3], [f32; 3]),
}

// ── Branch graph ────────────────────────────────────────────────────────────

struct Branch {
    level: u32,
    /// Polyline of centre points.
    pts: Vec<[f32; 3]>,
    /// Radius at each point.
    radii: Vec<f32>,
    /// Sway at each point (0 rigid → recipe.sway at the outermost tips).
    sway: Vec<f32>,
}

fn pick<T: Copy>(v: &[T], i: usize, fallback: T) -> T {
    v.get(i).copied().or_else(|| v.last().copied()).unwrap_or(fallback)
}

/// Deterministic PCG-style hash → 0..1.
fn rnd(state: &mut u32) -> f32 {
    *state = state.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    let mut x = *state;
    x = ((x >> ((x >> 28).wrapping_add(4))) ^ x).wrapping_mul(277_803_737);
    x = (x >> 22) ^ x;
    (x as f32) / (u32::MAX as f32)
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

/// Grow one branch as a polyline from `origin` along `dir`.
fn grow_branch(
    r: &GrowRecipe,
    rng: &mut u32,
    origin: [f32; 3],
    dir: [f32; 3],
    length: f32,
    radius0: f32,
    level: u32,
    segments: u32,
    sway_from: f32,
    curve_deg: f32,
) -> Branch {
    let segs = segments.max(2) as usize;
    let mut pts = Vec::with_capacity(segs + 1);
    let mut radii = Vec::with_capacity(segs + 1);
    let mut sway = Vec::with_capacity(segs + 1);
    let mut p = origin;
    let mut d = norm(dir);
    let step = length / segs as f32;
    let sway_to = (sway_from + r.sway / (r.levels as f32 + 1.0)).min(r.sway);
    // One bend plane per branch: a fixed perpendicular axis, a fixed sign, so
    // the branch draws an arc. The magnitude eases in (stiff near the base).
    let bend_axis = rotate(perp(d), d, rnd(rng) * std::f32::consts::TAU);
    let bend_total = curve_deg.to_radians() * if rnd(rng) < 0.5 { -1.0 } else { 1.0 };
    for i in 0..=segs {
        let t = i as f32 / segs as f32;
        pts.push(p);
        let mut rad = radius0 * (1.0 - t) + radius0 * r.taper * t;
        if level == 0 && r.flare > 0.0 && t < 0.12 {
            let f = 1.0 - t / 0.12;
            rad *= 1.0 + r.flare * f * f;
        }
        radii.push(rad);
        sway.push(sway_from + (sway_to - sway_from) * t);
        if i == segs {
            break;
        }
        // The arc, then droop (or lift) a little each step, then a small random bend.
        if bend_total != 0.0 {
            let ease = 0.5 + t; // more bend towards the tip
            d = norm(rotate(d, bend_axis, bend_total / segs as f32 * ease));
        }
        d = norm(add(d, [0.0, -r.gravity * step, 0.0]));
        if r.wobble > 0.0 {
            let axis = perp(d);
            let a = (rnd(rng) - 0.5) * r.wobble * 0.6;
            let b = (rnd(rng) - 0.5) * r.wobble * 0.6;
            d = norm(rotate(rotate(d, axis, a), norm(cross(d, axis)), b));
        }
        p = add(p, scale(d, step));
    }
    Branch { level, pts, radii, sway }
}

/// The full graph, trunk first, breadth-first by level.
fn graph(r: &GrowRecipe, segments: u32) -> Vec<Branch> {
    let mut rng = r.seed.wrapping_mul(2_654_435_761).wrapping_add(17);
    let mut out: Vec<Branch> = Vec::new();
    let lean = r.lean.to_radians();
    let lean_dir = rnd(&mut rng) * std::f32::consts::TAU;
    let up = norm([lean.sin() * lean_dir.cos(), lean.cos(), lean.sin() * lean_dir.sin()]);
    let trunk = grow_branch(r, &mut rng, [0.0, 0.0, 0.0], up, r.height, r.trunk_radius, 0, segments, 0.0, r.trunk_curve);
    out.push(trunk);
    let mut frontier: Vec<usize> = vec![0];
    for level in 1..=r.levels {
        let li = (level - 1) as usize;
        let n_lateral = pick(&r.branches, li, 0);
        let n_forks = pick(&r.forks, li, 2);
        let angle = pick(&r.angle, li, 40.0).to_radians();
        let len_frac = pick(&r.length, li, 0.55);
        let curve = pick(&r.curve, li, 15.0);
        let mut next: Vec<usize> = Vec::new();
        for &pi in &frontier {
            let parent_len: f32 = {
                let b = &out[pi];
                (1..b.pts.len()).map(|i| len(sub(b.pts[i], b.pts[i - 1]))).sum()
            };
            let phase = rnd(&mut rng) * std::f32::consts::TAU;
            // Forks: start at the tip, share its cross-section (da Vinci: the
            // children's areas sum to the parent's), spread evenly around it.
            for c in 0..n_forks {
                let b = &out[pi];
                let (p, d, rad, sw) = sample(b, 1.0);
                let spread = angle * (0.7 + 0.3 * rnd(&mut rng));
                let around = phase + std::f32::consts::TAU * c as f32 / n_forks as f32 + (rnd(&mut rng) - 0.5) * 0.5;
                let side = rotate(perp(d), d, around);
                let cdir = norm(add(scale(d, spread.cos()), scale(side, spread.sin())));
                let clen = parent_len * len_frac * (0.8 + 0.4 * rnd(&mut rng));
                let crad = (rad * (1.0 / n_forks as f32).sqrt() * 1.08).min(rad * 0.98);
                // Start a little inside the parent so the joint is buried.
                let origin = sub(p, scale(d, crad * 0.8));
                let segs = child_segments(r, segments, clen);
                let child = grow_branch(r, &mut rng, origin, cdir, clen, crad, level, segs, sw, curve);
                out.push(child);
                next.push(out.len() - 1);
            }
            // Laterals: along the parent between `sprout_from` and just below the tip.
            for c in 0..n_lateral {
                let b = &out[pi];
                let t = r.sprout_from + (0.9 - r.sprout_from) * (c as f32 + 0.5) / n_lateral as f32;
                let (p, d, rad, sw) = sample(b, t);
                let around = phase + 1.9 + std::f32::consts::TAU * c as f32 / n_lateral as f32 + (rnd(&mut rng) - 0.5) * 0.8;
                let side = rotate(perp(d), d, around);
                let spread = angle * (1.1 + 0.3 * rnd(&mut rng));
                let cdir = norm(add(scale(d, spread.cos()), scale(side, spread.sin())));
                let clen = parent_len * len_frac * (0.6 + 0.3 * rnd(&mut rng));
                let crad = rad * r.child_radius;
                let segs = child_segments(r, segments, clen);
                let child = grow_branch(r, &mut rng, p, cdir, clen, crad, level, segs, sw, curve);
                out.push(child);
                next.push(out.len() - 1);
            }
        }
        frontier = next;
    }
    out
}

/// Segments along a child scale with its length so twigs stay cheap.
fn child_segments(r: &GrowRecipe, segments: u32, length: f32) -> u32 {
    let f = (length / r.height.max(0.01)).clamp(0.3, 1.0);
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

// ── Meshing ─────────────────────────────────────────────────────────────────

fn push_vertex(m: &mut MeshData, p: [f32; 3], n: [f32; 3], uv: [f32; 2], sway: f32) {
    m.positions.push(p);
    m.normals.push(n);
    m.uvs.push(uv);
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
        let d = if i + 1 < n { norm(sub(b.pts[i + 1], b.pts[i])) } else { norm(sub(b.pts[i], b.pts[i - 1])) };
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
            push_vertex(m, p, nrm, [s as f32 / sides as f32 * (rad * 6.0).max(0.5), v_along], b.sway[i]);
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
    push_vertex(m, b.pts[tip_i], d, [0.5, v_along], b.sway[tip_i]);
    let last_ring = base + (n as u32 - 1) * ring;
    for s in 0..sides as u32 {
        m.indices.extend_from_slice(&[last_ring + s, last_ring + s + 1, tip_centre]);
    }
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn len(a: [f32; 3]) -> f32 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}

fn mesh_for(r: &GrowRecipe, detail: f32) -> (MeshData, Vec<Branch>) {
    let sides = ((r.sides as f32 * detail).round() as u32).max(3);
    let segments = ((r.segments as f32 * detail).round() as u32).max(2);
    let branches = graph(r, segments);
    let mut m = MeshData::default();
    // Coarser LODs drop the outermost generation(s) — the silhouette keeps.
    let max_level = if detail < 0.35 { r.levels.saturating_sub(1) } else { r.levels };
    let uv_scale = 1.0 / (r.trunk_radius * 6.0).max(0.2);
    for b in branches.iter().filter(|b| b.level <= max_level) {
        tube(&mut m, b, sides, uv_scale);
    }
    (m, branches)
}

/// Grow the tree: LOD0 as a [`Built`] for preview/export, every LOD mesh, sockets.
pub fn grow(r: &GrowRecipe) -> Result<Grown, String> {
    if r.height <= 0.0 || r.trunk_radius <= 0.0 {
        return Err("height and trunk_radius must be positive".into());
    }
    let (lod0, branches) = mesh_for(r, 1.0);
    let mut lods = vec![lod0.clone()];
    for &f in &r.lods {
        if f > 0.0 && f < 1.0 {
            lods.push(mesh_for(r, f).0);
        }
    }
    // Sockets: every branch tip at the outermost level (and the trunk's own
    // tip when it has no children).
    let outer = branches.iter().map(|b| b.level).max().unwrap_or(0);
    let mut sockets = Vec::new();
    for (i, b) in branches.iter().enumerate() {
        if b.level != outer && !(b.level == 0 && outer == 0) {
            continue;
        }
        let n = b.pts.len();
        let d = norm(sub(b.pts[n - 1], b.pts[n - 2]));
        sockets.push(Socket {
            name: format!("tip-{i}"),
            position: b.pts[n - 1],
            direction: d,
            radius: b.radii[n - 1],
            level: b.level,
        });
    }
    let baked = r.bark.as_ref().map(crate::texture::bake);
    let built = Built {
        name: r.name.clone(),
        parts: vec![BuiltPart { name: "wood".into(), mesh: lod0, baked, color: r.color, emissive: r.emissive }],
    };
    let bounds = built.bounds();
    Ok(Grown { built, lods, sockets, bounds })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_default_tree_grows_branches_and_sockets() {
        let g = grow(&GrowRecipe::default()).unwrap();
        assert!(g.built.triangles() > 500, "tris {}", g.built.triangles());
        assert_eq!(g.lods.len(), 3, "LOD0 + two coarser");
        assert!(g.lods[1].indices.len() < g.lods[0].indices.len());
        assert!(g.sockets.len() >= 8, "sockets {}", g.sockets.len());
        // Root rigid, tips sway: the engine's alpha convention.
        let m = &g.built.parts[0].mesh;
        assert!(m.colors[0][3] > 0.999);
        let min_alpha = m.colors.iter().map(|c| c[3]).fold(1.0f32, f32::min);
        assert!(min_alpha < 0.7, "tips should sway (min alpha {min_alpha})");
    }

    #[test]
    fn same_seed_same_tree() {
        let a = grow(&GrowRecipe::default()).unwrap();
        let b = grow(&GrowRecipe::default()).unwrap();
        assert_eq!(a.built.parts[0].mesh.positions, b.built.parts[0].mesh.positions);
        let c = grow(&GrowRecipe { seed: 99, ..Default::default() }).unwrap();
        assert_ne!(a.built.parts[0].mesh.positions, c.built.parts[0].mesh.positions);
    }
}
