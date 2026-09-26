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
//! vertex colour, which every PBR renderer multiplies into albedo).
//!
//! Every cluster carries a **key** — the twig's identity, not its position in
//! a list — and each leaf is [addressed](crate::rand) on it. So one seed grows
//! one crown, everywhere: a coarser LOD thins the count and the leaves that
//! remain are the very same leaves, and a sapling's crown is the crown its
//! grown self will have on the twigs it already has.
use infinite_manifest::texture::TextureRecipe;
use serde::{Deserialize, Serialize};

use chisel::MeshData;

use crate::rand::Rnd;

/// Slots per leaf on its cluster's key: which way it points, how far it tilts,
/// how big it is, how it is rolled. A stride, so leaf 9 keeps its address
/// whether the cluster grows eight leaves or eighty.
const PER_LEAF: u32 = 4;
const SLOT_AROUND: u32 = 0;
const SLOT_TILT: u32 = 1;
const SLOT_SIZE: u32 = 2;
const SLOT_ROLL: u32 = 3;

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
    /// Leaf length and width in metres. A leaf is its own size from the start —
    /// a sapling's leaves are not miniatures — so this does not scale with age.
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
    /// The cluster's identity — hashed from the twig it grows on, so this
    /// cluster's leaves are the same leaves at every age and every LOD.
    pub key: u32,
    pub position: [f32; 3],
    pub direction: [f32; 3],
    pub sway: f32,
    /// How many leaves this site carries (tips carry `per_tip`, along-sites fewer).
    pub count: u32,
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
/// (never below one leaf per site) and `seed`, with each site's key, keeps the
/// crown the same.
pub fn leaves(sites: &[LeafSite], r: &LeafRecipe, detail: f32, seed: u32) -> MeshData {
    let mut m = MeshData::default();
    let spread = r.spread.to_radians();
    for site in sites {
        let rnd = Rnd::new(seed, site.key);
        let n = ((site.count as f32 * detail).round() as u32).max(1);
        let d = norm(site.direction);
        let side0 = perp(d);
        for i in 0..n {
            let slot = i * PER_LEAF;
            // Direction: inside the cone around the twig, then pulled down by droop.
            let around = rnd.at(slot + SLOT_AROUND) * std::f32::consts::TAU;
            let tilt = spread * rnd.at(slot + SLOT_TILT).sqrt();
            let side = rotate(side0, d, around);
            let mut axis = norm(add(scale(d, tilt.cos()), scale(side, tilt.sin())));
            if r.droop > 0.0 {
                axis = norm(add(scale(axis, 1.0 - r.droop), scale([0.0, -1.0, 0.0], r.droop)));
            }
            let s = 1.0 + rnd.signed(slot + SLOT_SIZE) * r.jitter;
            let len = r.length * s;
            let wid = r.width * s;
            // Leaf frame: axis along the midrib, `flat` across it, `up` out of the blade.
            let roll = rnd.at(slot + SLOT_ROLL) * std::f32::consts::TAU;
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

    fn site(key: u32, count: u32) -> LeafSite {
        LeafSite { key, position: [0.0, 2.0, 0.0], direction: [0.0, 1.0, 0.0], sway: 0.3, count }
    }

    #[test]
    fn a_site_grows_its_leaves_and_thins_by_detail() {
        let sites = vec![site(11, 8)];
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

    #[test]
    fn thinning_keeps_the_leaves_it_keeps() {
        // A coarser LOD drops leaves; it does not grow a different cluster.
        let sites = vec![site(11, 8)];
        let r = LeafRecipe::default();
        let full = leaves(&sites, &r, 1.0, 3);
        let half = leaves(&sites, &r, 0.5, 3);
        assert_eq!(half.positions, full.positions[..half.positions.len()].to_vec());
    }

    #[test]
    fn a_clusters_leaves_are_its_own() {
        // Two clusters in the same place with different keys are different
        // clusters; the same key anywhere in a list is the same cluster.
        let r = LeafRecipe::default();
        let a = leaves(&[site(11, 6)], &r, 1.0, 3);
        let b = leaves(&[site(12, 6)], &r, 1.0, 3);
        assert_ne!(a.positions, b.positions);
        let pair = leaves(&[site(12, 6), site(11, 6)], &r, 1.0, 3);
        assert_eq!(pair.positions[..b.positions.len()].to_vec(), b.positions);
        assert_eq!(pair.positions[b.positions.len()..].to_vec(), a.positions);
    }
}
