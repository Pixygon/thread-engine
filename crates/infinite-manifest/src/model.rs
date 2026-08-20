//! # Model — the agent-native 3D format of the Thread
//!
//! A model is a **flat sequence of carving operations** plus the PBR
//! materials its parts wear. Not a scene graph, not a mesh: the *steps* —
//! "start with a box, blend a sphere, cut a cylinder" — which is how modeling
//! actually reads, and (crucially) the only shape of program a
//! [Weft](../../weft) value can express: Weft has no recursive types, so a
//! tree cannot cross the wire, but a **list of uniform records** can. That
//! constraint is a gift: it makes models generable by loops
//! (`Iota`/`Map`/`Fold`), diffable line by line, and streamable.
//!
//! ```jsonc
//! { "name": "amphora",
//!   "nodes": [
//!     { "prim": "lathe", "profile": [0,0, 0.32,0, 0.5,0.9, 0.3,1.66, 0,1.66] },
//!     { "prim": "torus", "mode": "blend", "k": 0.06, "y": 1.5, "r": 0.36, "r2": 0.05 }
//!   ],
//!   "materials": [ { "kind": "fbm", "colors": [0.5,0.27,0.16, 0.68,0.42,0.26] } ] }
//! ```
//!
//! **PBR is the default, not an upgrade.** Every part carries a material
//! recipe that bakes the full set — base color, tangent-space normal,
//! metallic, roughness (or `smoothness`, the Unity spelling), and ambient
//! occlusion — so an exported `.glb` is complete without an artist ever
//! opening a texture editor.

use serde::{Deserialize, Serialize};

use crate::shape::{Group, Lathe, Prim, Shape};
use crate::texture::TextureRecipe;

/// A complete model: named, in parts, PBR-ready.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Model {
    #[serde(default = "unnamed")]
    pub name: String,
    /// The carving steps, in order.
    pub nodes: Vec<Node>,
    /// One entry per part (`node.part` indexes here). An empty list means a
    /// single default-stone part — a model is never material-less.
    #[serde(default)]
    pub materials: Vec<PartMaterial>,
}

fn unnamed() -> String {
    "model".to_string()
}

/// One carving step. Uniform on purpose: every field exists for every node so
/// the whole list is one Weft record type (and one JSON shape to learn).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    /// `sphere` · `box` · `cylinder` · `capsule` · `cone` · `torus` · `lathe`.
    /// (`cyl` and `cube` are accepted spellings.)
    pub prim: String,
    /// How this step combines with what's carved so far: `add` (union),
    /// `blend` (smooth union over `k` metres), `cut`, `intersect`. The first
    /// step of a part is the seed whatever its mode.
    #[serde(default = "add_mode")]
    pub mode: String,
    /// Which part (material group) this step belongs to.
    #[serde(default)]
    pub part: u32,
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub y: f32,
    #[serde(default)]
    pub z: f32,
    /// Y-rotation in degrees.
    #[serde(default)]
    pub rot: f32,
    /// Long axis for the rotational prims: `y` (default), `x`, `z`.
    #[serde(default = "axis_y")]
    pub axis: String,
    /// Radius (sphere/cylinder/capsule/cone/torus-major).
    #[serde(default = "half")]
    pub r: f32,
    /// Secondary radius (cone top / torus minor).
    #[serde(default)]
    pub r2: f32,
    /// Height (cylinder/capsule/cone) and box Y.
    #[serde(default = "one")]
    pub h: f32,
    /// Box X.
    #[serde(default = "one")]
    pub w: f32,
    /// Box Z.
    #[serde(default = "one")]
    pub d: f32,
    /// Box corner rounding.
    #[serde(default)]
    pub round: f32,
    /// Blend radius for `mode: "blend"`.
    #[serde(default = "quarter")]
    pub k: f32,
    /// Lathe profile as flat `r, y` pairs (`prim: "lathe"` only).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profile: Vec<f32>,
}

fn add_mode() -> String {
    "add".to_string()
}
fn axis_y() -> String {
    "y".to_string()
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

/// A part's PBR material: a procedural recipe plus the knobs a renderer
/// applies on top of the baked maps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PartMaterial {
    /// The bakeable recipe (albedo ramp, normal height, roughness/metallic
    /// bands, AO). Absent → a plain tinted surface.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture: Option<TextureRecipe>,
    /// Multiplied over the baked base color — one recipe, many tints.
    #[serde(default = "white4")]
    pub color: [f32; 4],
    /// Glow strength (0 = lit normally).
    #[serde(default)]
    pub emissive: f32,
    /// Meshing density for this part's carve (default 40, cap 96).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<u32>,
    /// UV projection: `auto` (default — cylindrical for lathes and rotational
    /// prims, spherical for spheres, box otherwise), `box`, `cylindrical`,
    /// `spherical`.
    #[serde(default = "auto_uv")]
    pub uv: String,
    /// Texture repeats per metre (UV scale). Default 0.5.
    #[serde(default = "half")]
    pub uv_scale: f32,
    #[serde(default = "unnamed_part")]
    pub name: String,
}

fn white4() -> [f32; 4] {
    [1.0, 1.0, 1.0, 1.0]
}
fn auto_uv() -> String {
    "auto".to_string()
}
fn unnamed_part() -> String {
    "part".to_string()
}

impl Default for PartMaterial {
    fn default() -> Self {
        PartMaterial {
            texture: None,
            color: white4(),
            emissive: 0.0,
            resolution: None,
            uv: auto_uv(),
            uv_scale: half(),
            name: unnamed_part(),
        }
    }
}

/// One resolved part: the shape its steps carved, and how it looks.
#[derive(Debug, Clone)]
pub struct ResolvedPart {
    pub name: String,
    pub shape: Shape,
    pub material: PartMaterial,
}

impl Model {
    /// Validate the whole model (known words, sane steps, materials present).
    pub fn validate(&self) -> Result<(), String> {
        if self.nodes.is_empty() {
            return Err("a model needs at least one carving step".into());
        }
        for (i, n) in self.nodes.iter().enumerate() {
            if !matches!(
                n.prim.as_str(),
                "sphere"
                    | "box"
                    | "cube"
                    | "cylinder"
                    | "cyl"
                    | "capsule"
                    | "cone"
                    | "torus"
                    | "lathe"
            ) {
                return Err(format!("step {i}: unknown prim '{}'", n.prim));
            }
            if !matches!(
                n.mode.as_str(),
                "add" | "union" | "blend" | "cut" | "intersect"
            ) {
                return Err(format!("step {i}: unknown mode '{}'", n.mode));
            }
            if n.prim == "lathe" && n.profile.len() < 6 {
                return Err(format!("step {i}: a lathe profile needs 3+ `r y` pairs"));
            }
        }
        for m in &self.materials {
            if let Some(t) = &m.texture {
                t.validate()?;
            }
        }
        self.resolve()?;
        Ok(())
    }

    /// Fold the flat steps into one [`Shape`] per part, in part order.
    pub fn resolve(&self) -> Result<Vec<ResolvedPart>, String> {
        let mut parts: Vec<(u32, Option<Shape>)> = Vec::new();
        for n in &self.nodes {
            let leaf = node_shape(n);
            let slot = match parts.iter_mut().find(|(p, _)| *p == n.part) {
                Some(s) => s,
                None => {
                    parts.push((n.part, None));
                    parts.last_mut().expect("just pushed")
                }
            };
            slot.1 = Some(match slot.1.take() {
                // The first step of a part is the seed, whatever its mode.
                None => leaf,
                Some(acc) => {
                    let op = match n.mode.as_str() {
                        "blend" => "blend",
                        "cut" => "cut",
                        "intersect" => "intersect",
                        _ => "union",
                    };
                    Shape::Group(Group {
                        op: op.to_string(),
                        k: n.k,
                        at: [0.0; 3],
                        rot: 0.0,
                        parts: vec![acc, leaf],
                    })
                }
            });
        }
        let mut out = Vec::new();
        for (part, shape) in parts {
            let Some(shape) = shape else { continue };
            let material = self
                .materials
                .get(part as usize)
                .cloned()
                .unwrap_or_default();
            let name = if material.name == "part" && !self.name.is_empty() {
                if self.materials.len() > 1 {
                    format!("{}-{}", self.name, part)
                } else {
                    self.name.clone()
                }
            } else {
                material.name.clone()
            };
            out.push(ResolvedPart {
                name,
                shape,
                material,
            });
        }
        if out.is_empty() {
            return Err("no parts resolved".into());
        }
        Ok(out)
    }
}

/// One step's own volume, positioned.
fn node_shape(n: &Node) -> Shape {
    let at = [n.x, n.y, n.z];
    if n.prim == "lathe" {
        let pts: Vec<[f32; 2]> = n.profile.chunks_exact(2).map(|c| [c[0], c[1]]).collect();
        return Shape::Lathe(Lathe { lathe: pts, at });
    }
    let prim = match n.prim.as_str() {
        "cube" => "box",
        "cyl" => "cylinder",
        other => other,
    };
    Shape::Prim(Prim {
        prim: prim.to_string(),
        at,
        rot: n.rot,
        r: n.r,
        size: (prim == "box").then_some([n.w, n.h, n.d]),
        h: n.h,
        r2: n.r2,
        rounded: n.round,
        axis: n.axis.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flat_op_sequence_folds_into_carved_parts() {
        let m: Model = serde_json::from_str(
            r#"{ "name": "bell",
                 "nodes": [
                   { "prim": "sphere", "r": 1.0, "y": 1.0 },
                   { "prim": "box", "mode": "cut", "w": 4, "h": 2, "d": 4, "y": 0.0 },
                   { "prim": "cylinder", "mode": "blend", "k": 0.2, "part": 1,
                     "r": 0.1, "h": 2.0, "y": 1.0 }
                 ],
                 "materials": [
                   { "name": "shell", "color": [0.8,0.7,0.3,1] },
                   { "name": "clapper" }
                 ] }"#,
        )
        .unwrap();
        m.validate().unwrap();
        let parts = m.resolve().unwrap();
        assert_eq!(parts.len(), 2, "two material groups → two parts");
        assert_eq!(parts[0].name, "shell");
        assert_eq!(parts[0].material.color[0], 0.8);
        // Part 0 is a cut group over the seed sphere.
        match &parts[0].shape {
            Shape::Group(g) => {
                assert_eq!(g.op, "cut");
                assert_eq!(g.parts.len(), 2);
            }
            other => panic!("expected a cut group, got {other:?}"),
        }
        // A single-step part is just its primitive (no wrapper).
        assert!(matches!(parts[1].shape, Shape::Prim(_)));
    }

    #[test]
    fn defaults_make_the_minimal_model_legal_and_bad_words_fail() {
        let m: Model = serde_json::from_str(r#"{ "nodes": [ { "prim": "sphere" } ] }"#).unwrap();
        m.validate().unwrap();
        let parts = m.resolve().unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].material.uv, "auto", "PBR defaults are present");

        let bad: Model = serde_json::from_str(r#"{ "nodes": [ { "prim": "blob" } ] }"#).unwrap();
        assert!(bad.validate().is_err());
        let bad: Model =
            serde_json::from_str(r#"{ "nodes": [ { "prim": "sphere", "mode": "smoosh" } ] }"#)
                .unwrap();
        assert!(bad.validate().is_err());
        let bad: Model =
            serde_json::from_str(r#"{ "nodes": [ { "prim": "lathe", "profile": [0,0] } ] }"#)
                .unwrap();
        assert!(bad.validate().is_err());
    }
}
