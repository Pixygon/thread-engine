//! Leaves — the foliage layer that hangs on the wood.
//!
//! A leaf is geometry, not a textured card: a folded lozenge (five vertices,
//! four triangles) that reads as a leaf from every angle, needs no alpha
//! mask, casts a real shadow and takes the same wind channel as the twig it
//! grows from. Clusters gather at every terminal tip and, when asked, along
//! the last generation of twigs, so a crown fills in the way a real one does
//! — at the ends, where the light is.
//!
//! The recipe is rules: leaves per tip, how far back along the twig, size,
//! how wide the cluster fans, how much it droops, its colours base→tip (in
//! vertex colour, which every PBR renderer multiplies into albedo). One seed,
//! one crown, everywhere. Coarser LODs thin the count and keep the silhouette.
use infinite_manifest::texture::TextureRecipe;
use serde::{Deserialize, Serialize};

use chisel::MeshData;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LeafRecipe {
    /// Leaves in the cluster at each terminal tip.
    pub per_tip: u32,
    /// Also leaves along the last generation of twigs: this fraction of the
    /// twig, measured back from its tip (0 = tips only).
    pub along: f32,
    /// Leaves per twig along that stretch.
    pub along_count: u32,
    /// How many generations back from the outermost carry leaves (1 = the
    /// twigs only; 2 = the branches they grow from as well, along their
    /// length). A full crown is 2; a sparse or young one is 1.
    pub depth: u32,
    /// Leaf length and width in metres.
    pub length: f32,
    pub width: f32,
    /// Fold across the midrib, metres the centre rises.
    pub fold: f32,
    /// Cluster cone half-angle around the twig direction, degrees.
    pub spread: f32,
    /// 0 = leaves follow the twig; 1 = they hang straight down.
    pub droop: f32,
    /// ± fraction of random size per leaf.
    pub jitter: f32,
    /// Colour at the leaf base and at its tip (RGBA); the gradient lives in
    /// vertex colour so it costs no texture.
    pub color: [f32; 4],
    pub color_tip: [f32; 4],
    /// Optional surface recipe (veins, speckle); absent = flat colour.
    pub texture: Option<TextureRecipe>,
    pub emissive: f32,
    /// Extra sway on top of the twig's, 0..1.
    pub sway: f32,
}

impl Default for LeafRecipe {
    fn default() -> Self {
        Self {
            per_tip: 6,
            along: 0.5,
            along_count: 6,
            depth: 1,
            length: 0.12,
            width: 0.07,
            fold: 0.012,
            spread: 55.0,
            droop: 0.35,
            jitter: 0.3,
            color: [0.16, 0.36, 0.12, 1.0],
            color_tip: [0.42, 0.62, 0.2, 1.0],
            texture: None,
            emissive: 0.0,
            sway: 0.25,
        }
    }
}

/// Where a cluster grows: a point on a twig, the twig's direction there, and
/// how much that spot already sways.
pub struct LeafSite {
    pub position: [f32; 3],
    pub direction: [f32; 3],
    pub sway: f32,
    /// How many leaves this site carries (tips carry `per_tip`, along-sites fewer).
    pub count: u32,
}

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
fn perp(d: [f32; 3]) -> [f32; 3] {
    let alt = if d[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] };
    norm(cross(d, alt))
}

/// Grow every leaf into one mesh. `detail` thins the count for coarser LODs
/// (never below one leaf per site) and `seed` keeps the crown the same.
pub fn leaves(sites: &[LeafSite], r: &LeafRecipe, detail: f32, seed: u32) -> MeshData {
    let mut m = MeshData::default();
    let mut rng = seed.wrapping_mul(1_597_334_677).wrapping_add(101);
    let spread = r.spread.to_radians();
    for site in sites {
        let n = ((site.count as f32 * detail).round() as u32).max(1);
        let d = norm(site.direction);
        let side0 = perp(d);
        for _ in 0..n {
            // Direction: inside the cone around the twig, then pulled down by droop.
            let around = rnd(&mut rng) * std::f32::consts::TAU;
            let tilt = spread * rnd(&mut rng).sqrt();
            let side = rotate(side0, d, around);
            let mut axis = norm(add(scale(d, tilt.cos()), scale(side, tilt.sin())));
            if r.droop > 0.0 {
                axis = norm(add(scale(axis, 1.0 - r.droop), scale([0.0, -1.0, 0.0], r.droop)));
            }
            let s = 1.0 + (rnd(&mut rng) * 2.0 - 1.0) * r.jitter;
            let len = r.length * s;
            let wid = r.width * s;
            // Leaf frame: axis along the midrib, `flat` across it, `up` out of the blade.
            let roll = rnd(&mut rng) * std::f32::consts::TAU;
            let flat = rotate(perp(axis), axis, roll);
            let up = norm(cross(axis, flat));
            let sway = (site.sway + r.sway).clamp(0.0, 1.0);
            leaf(&mut m, site.position, axis, flat, up, len, wid, r.fold * s, r.color, r.color_tip, sway);
        }
    }
    m
}

/// One folded lozenge: base, left, right, tip, and a raised centre.
#[allow(clippy::too_many_arguments)]
fn leaf(
    m: &mut MeshData,
    base: [f32; 3],
    axis: [f32; 3],
    flat: [f32; 3],
    up: [f32; 3],
    len: f32,
    wid: f32,
    fold: f32,
    c0: [f32; 4],
    c1: [f32; 4],
    sway: f32,
) {
    let mid = add(base, scale(axis, len * 0.45));
    let pts = [
        base,
        add(mid, scale(flat, -wid * 0.5)),
        add(mid, scale(flat, wid * 0.5)),
        add(base, scale(axis, len)),
        add(mid, scale(up, fold)),
    ];
    let uvs = [[0.5, 0.0], [0.0, 0.45], [1.0, 0.45], [0.5, 1.0], [0.5, 0.45]];
    let tints = [0.0f32, 0.45, 0.45, 1.0, 0.45];
    let i0 = m.positions.len() as u32;
    let tangent = [flat[0], flat[1], flat[2], 1.0];
    for k in 0..5 {
        // Normals lean with the fold so the two halves catch light differently.
        let n = match k {
            1 => norm(add(up, scale(flat, -0.35))),
            2 => norm(add(up, scale(flat, 0.35))),
            _ => up,
        };
        m.positions.push(pts[k]);
        m.normals.push(n);
        m.uvs.push(uvs[k]);
        m.tangents.push(tangent);
        let t = tints[k];
        m.colors.push([
            c0[0] + (c1[0] - c0[0]) * t,
            c0[1] + (c1[1] - c0[1]) * t,
            c0[2] + (c1[2] - c0[2]) * t,
            1.0 - sway,
        ]);
    }
    // Four triangles around the raised centre, wound so `up` is the front.
    m.indices.extend_from_slice(&[
        i0, i0 + 1, i0 + 4, //
        i0, i0 + 4, i0 + 2, //
        i0 + 1, i0 + 3, i0 + 4, //
        i0 + 4, i0 + 3, i0 + 2,
    ]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_site_grows_its_leaves_and_thins_by_detail() {
        let sites = vec![LeafSite { position: [0.0, 2.0, 0.0], direction: [0.0, 1.0, 0.0], sway: 0.3, count: 8 }];
        let r = LeafRecipe::default();
        let full = leaves(&sites, &r, 1.0, 3);
        assert_eq!(full.positions.len(), 8 * 5);
        assert_eq!(full.indices.len(), 8 * 12);
        let half = leaves(&sites, &r, 0.5, 3);
        assert_eq!(half.positions.len(), 4 * 5);
        let tiny = leaves(&sites, &r, 0.05, 3);
        assert_eq!(tiny.positions.len(), 5, "never below one leaf per site");
        // Wind: leaf alpha carries site sway + leaf sway.
        assert!((full.colors[0][3] - (1.0 - 0.55)).abs() < 1e-5);
        assert_eq!(leaves(&sites, &r, 1.0, 3).positions, full.positions, "same seed, same crown");
    }
}
