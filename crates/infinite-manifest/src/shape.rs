//! # Shape — procedural geometry as manifest data
//!
//! The Thread ships *recipes*, not vertex soup: a [`Shape`] is a small
//! composable tree of signed-distance parts — primitives, smooth blends,
//! carves, lathed profiles — that a browser meshes locally at load time
//! (never in the frame loop). It is the modeling language for agents: every
//! node is a word, every parameter a number, and the whole thing serializes
//! as ordinary manifest JSON, so a "model" is diffable, promptable, and tiny
//! on the wire.
//!
//! The mesher lives in the `chisel` crate; this module is only the shared
//! vocabulary (types + validation + conservative bounds).

use serde::{Deserialize, Serialize};

/// One node of a shape tree. Untagged: the discriminating field names the
/// variant (`prim` / `op` / `lathe`), which keeps authored JSON minimal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Shape {
    /// A primitive volume.
    Prim(Prim),
    /// A boolean/blend over children.
    Group(Group),
    /// A 2D profile revolved around the local Y axis.
    Lathe(Lathe),
}

/// A primitive: `sphere` (r) · `box` (size, rounded) · `cylinder` (r, h) ·
/// `capsule` (r, h) · `cone` (r → r2 over h) · `torus` (r major, r2 minor).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Prim {
    pub prim: String,
    /// Centre offset in the parent's frame.
    #[serde(default)]
    pub at: [f32; 3],
    /// Y-rotation, degrees.
    #[serde(default)]
    pub rot: f32,
    /// Primary radius (sphere/cylinder/capsule/cone/torus major).
    #[serde(default = "half")]
    pub r: f32,
    /// Box full extents.
    #[serde(default)]
    pub size: Option<[f32; 3]>,
    /// Height (cylinder/capsule/cone) — full, not half.
    #[serde(default = "one")]
    pub h: f32,
    /// Secondary radius (cone top; torus minor). Cone default 0 = a point.
    #[serde(default)]
    pub r2: f32,
    /// Corner rounding (box).
    #[serde(default)]
    pub rounded: f32,
    /// Long axis for cylinder/capsule/cone/torus: `"y"` (default), `"x"`, `"z"`.
    #[serde(default = "axis_y", skip_serializing_if = "is_axis_y")]
    pub axis: String,
}

fn axis_y() -> String {
    "y".into()
}
fn is_axis_y(a: &String) -> bool {
    a == "y"
}

/// A combining node: `union` · `blend` (smooth union, `k` metres of meld) ·
/// `cut` (first part minus the rest) · `intersect`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Group {
    pub op: String,
    /// Blend radius for `blend` (ignored elsewhere).
    #[serde(default = "quarter")]
    pub k: f32,
    #[serde(default)]
    pub at: [f32; 3],
    #[serde(default)]
    pub rot: f32,
    pub parts: Vec<Shape>,
}

/// A polyline profile in the (radius, height) plane, revolved around local Y.
/// The profile is closed automatically. Vases, columns, domes, rims.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lathe {
    /// `[radius, height]` points, in order. At least 3.
    pub lathe: Vec<[f32; 2]>,
    #[serde(default)]
    pub at: [f32; 3],
}

fn half() -> f32 {
    0.5
}
fn one() -> f32 {
    1.0
}
fn quarter() -> f32 {
    0.25
}

/// Map a Y-long extent to the prim's declared axis.
fn swizzle_extent(e: [f32; 3], axis: &str) -> [f32; 3] {
    match axis {
        "x" => [e[1], e[0], e[2]],
        "z" => [e[0], e[2], e[1]],
        _ => e,
    }
}

/// The primitive names this spec version knows.
pub const PRIMS: &[&str] = &["sphere", "box", "cylinder", "capsule", "cone", "torus"];
/// The combining ops this spec version knows.
pub const OPS: &[&str] = &["union", "blend", "cut", "intersect"];

impl Shape {
    /// Validate the tree: known words, sane parameters, bounded depth.
    pub fn validate(&self) -> Result<(), String> {
        self.validate_at(0)
    }

    fn validate_at(&self, depth: usize) -> Result<(), String> {
        if depth > 24 {
            return Err("shape tree deeper than 24".into());
        }
        match self {
            Shape::Prim(p) => {
                if !PRIMS.contains(&p.prim.as_str()) {
                    return Err(format!("unknown prim '{}' — one of {PRIMS:?}", p.prim));
                }
                if p.prim == "box" && p.size.is_none() {
                    return Err("box needs a `size`".into());
                }
                Ok(())
            }
            Shape::Group(g) => {
                if !OPS.contains(&g.op.as_str()) {
                    return Err(format!("unknown op '{}' — one of {OPS:?}", g.op));
                }
                if g.parts.is_empty() {
                    return Err(format!("'{}' needs at least one part", g.op));
                }
                for part in &g.parts {
                    part.validate_at(depth + 1)?;
                }
                Ok(())
            }
            Shape::Lathe(l) => {
                if l.lathe.len() < 3 {
                    return Err("lathe needs at least 3 profile points".into());
                }
                Ok(())
            }
        }
    }

    /// Conservative axis-aligned bounds of the surface, in the shape's own
    /// frame (the mesher's sampling volume).
    pub fn bounds(&self) -> ([f32; 3], [f32; 3]) {
        match self {
            Shape::Prim(p) => {
                let e = match p.prim.as_str() {
                    "sphere" => [p.r; 3],
                    "box" => {
                        let s = p.size.unwrap_or([1.0; 3]);
                        [
                            s[0] / 2.0 + p.rounded,
                            s[1] / 2.0 + p.rounded,
                            s[2] / 2.0 + p.rounded,
                        ]
                    }
                    "cylinder" | "cone" => {
                        let r = p.r.max(p.r2);
                        swizzle_extent([r, p.h / 2.0, r], &p.axis)
                    }
                    "capsule" => swizzle_extent([p.r, p.h / 2.0 + p.r, p.r], &p.axis),
                    "torus" => swizzle_extent([p.r + p.r2, p.r2, p.r + p.r2], &p.axis),
                    _ => [p.r; 3],
                };
                // Y-rotation can grow the xz footprint up to the diagonal.
                let d = (e[0] * e[0] + e[2] * e[2]).sqrt();
                let e = if p.rot != 0.0 { [d, e[1], d] } else { e };
                (
                    [p.at[0] - e[0], p.at[1] - e[1], p.at[2] - e[2]],
                    [p.at[0] + e[0], p.at[1] + e[1], p.at[2] + e[2]],
                )
            }
            Shape::Group(g) => {
                let mut it = g.parts.iter().map(|s| s.bounds());
                let (mut min, mut max) = it.next().unwrap_or(([0.0; 3], [0.0; 3]));
                match g.op.as_str() {
                    "cut" => {} // the carve only removes — the first part bounds it
                    "intersect" => {
                        for (bmin, bmax) in it {
                            for i in 0..3 {
                                min[i] = min[i].max(bmin[i]);
                                max[i] = max[i].min(bmax[i]);
                            }
                        }
                    }
                    _ => {
                        for (bmin, bmax) in it {
                            for i in 0..3 {
                                min[i] = min[i].min(bmin[i]);
                                max[i] = max[i].max(bmax[i]);
                            }
                        }
                        if g.op == "blend" {
                            for i in 0..3 {
                                min[i] -= g.k;
                                max[i] += g.k;
                            }
                        }
                    }
                }
                let d = ((max[0] - min[0]).max(max[2] - min[2])) / 2.0;
                let (cx, cz) = ((min[0] + max[0]) / 2.0, (min[2] + max[2]) / 2.0);
                let (mut min, mut max) = if g.rot != 0.0 {
                    // Rotated group: widen xz to the rotation-safe square.
                    ([cx - d, min[1], cz - d], [cx + d, max[1], cz + d])
                } else {
                    (min, max)
                };
                for i in 0..3 {
                    min[i] += g.at[i];
                    max[i] += g.at[i];
                }
                (min, max)
            }
            Shape::Lathe(l) => {
                let rmax = l.lathe.iter().map(|p| p[0].abs()).fold(0.0f32, f32::max);
                let (ymin, ymax) = l
                    .lathe
                    .iter()
                    .fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), p| {
                        (a.min(p[1]), b.max(p[1]))
                    });
                (
                    [l.at[0] - rmax, l.at[1] + ymin, l.at[2] - rmax],
                    [l.at[0] + rmax, l.at[1] + ymax, l.at[2] + rmax],
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_minimal_forms_and_validates() {
        let s: Shape = serde_json::from_str(
            r#"{ "op": "blend", "k": 0.3, "parts": [
                 { "prim": "sphere", "r": 1.0, "at": [0, 0.6, 0] },
                 { "prim": "box", "size": [2, 0.4, 2] },
                 { "lathe": [[0,0],[0.5,0],[0.6,0.8],[0.2,1.2]] }
               ] }"#,
        )
        .unwrap();
        s.validate().unwrap();
        let (min, max) = s.bounds();
        assert!(
            min[0] < -1.0 && max[1] > 1.0,
            "bounds cover the parts: {min:?} {max:?}"
        );
        // Round-trips as the same tree.
        let json = serde_json::to_string(&s).unwrap();
        let back: Shape = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn bad_trees_fail_loudly() {
        let bad: Shape = serde_json::from_str(r#"{ "prim": "blob", "r": 1.0 }"#).unwrap();
        assert!(bad.validate().is_err());
        let bad: Shape =
            serde_json::from_str(r#"{ "op": "melt", "parts": [{ "prim": "sphere" }] }"#).unwrap();
        assert!(bad.validate().is_err());
        let bad: Shape = serde_json::from_str(r#"{ "prim": "box" }"#).unwrap();
        assert!(bad.validate().is_err(), "box without size");
    }
}
