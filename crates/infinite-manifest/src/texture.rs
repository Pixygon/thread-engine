//! # Texture — procedural PBR materials as manifest data
//!
//! The texturing side of the carving language: a [`TextureRecipe`] is one
//! small block that a browser bakes locally into the full PBR set — albedo,
//! tangent-space normal map, and the packed occlusion-roughness-metallic
//! map — from a deterministic, seamlessly tiling pattern. No image files, no
//! UV unwrapping session, no upload step: the material is a sentence.
//!
//! The baker lives in the `chisel` crate; this module is the shared
//! vocabulary (types + validation).

use serde::{Deserialize, Serialize};

/// The pattern kinds this spec version knows.
pub const KINDS: &[&str] = &[
    "fbm", "voronoi", "bricks", "checker", "wood", "veins", "flat",
];

/// Texture side-length cap — a manifest cannot make a browser bake 4K maps.
pub const MAX_SIZE: u32 = 512;

/// One procedural material: a pattern drives everything. The pattern's value
/// (0–1 per texel) indexes the color ramp, lerps roughness/metallic between
/// their two ends, acts as a height field for the normal map, and darkens
/// crevices for ambient occlusion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextureRecipe {
    /// Pattern: `fbm` (organic noise) · `voronoi` (stone cells + cracks) ·
    /// `bricks` · `wood` (warped plank grain) · `veins` (marble) ·
    /// `checker` · `flat`.
    pub kind: String,
    /// Pattern repeats across the tile (features per tile edge). Default 4.
    #[serde(default = "four")]
    pub scale: f32,
    /// fbm octaves (detail). Default 4.
    #[serde(default = "four_u")]
    pub octaves: u32,
    /// Determinism seed — same recipe, same texels, everywhere. Default 0.
    #[serde(default)]
    pub seed: u32,
    /// Color ramp stops, evenly spaced over the pattern value. 1+ colors.
    #[serde(default = "grey")]
    pub colors: Vec<[f32; 3]>,
    /// Roughness at pattern value 0 → 1. Default `[0.9, 0.9]`.
    #[serde(default = "rough_default")]
    pub roughness: [f32; 2],
    /// Unity's spelling of the same dial, if you think that way: smoothness
    /// = 1 − roughness. When present it WINS over `roughness` (see
    /// [`TextureRecipe::rough_band`]) — one material vocabulary, two accents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smoothness: Option<[f32; 2]>,
    /// Metallic at pattern value 0 → 1. Default `[0, 0]`.
    #[serde(default)]
    pub metallic: [f32; 2],
    /// Normal-map strength — the pattern read as a height field. 0 = flat.
    #[serde(default)]
    pub height: f32,
    /// Crevice darkening strength (bakes into the occlusion channel). 0 = none.
    #[serde(default)]
    pub ao: f32,
    /// Baked texture side in texels. Default 256, clamped to [`MAX_SIZE`].
    #[serde(default = "size_default")]
    pub size: u32,
    /// A second recipe layered OVER this one, blended by an fbm mask —
    /// moss over granite, rust over iron, plaster over brick. One level deep.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub over: Option<Box<TextureRecipe>>,
    /// How much of the overlay shows (0 none → 1 full). Default 0.5.
    #[serde(default = "half_f")]
    pub mix: f32,
    /// Feature scale of the blend mask (default 3).
    #[serde(default = "three")]
    pub mask_scale: f32,
    /// Seed of the blend mask.
    #[serde(default)]
    pub mask_seed: u32,
    /// World-space triplanar sampling in the browser (uv repeats per metre;
    /// 0 = classic UV sampling). Kills projection seams on carved shapes —
    /// set ~0.5 for architecture. Exported GLBs keep UVs (glTF has no
    /// triplanar), so exports still look right, just with seams.
    #[serde(default)]
    pub triplanar: f32,
}

fn half_f() -> f32 {
    0.5
}
fn three() -> f32 {
    3.0
}

fn four() -> f32 {
    4.0
}
fn four_u() -> u32 {
    4
}
fn grey() -> Vec<[f32; 3]> {
    vec![[0.6, 0.6, 0.6]]
}
fn rough_default() -> [f32; 2] {
    [0.9, 0.9]
}
fn size_default() -> u32 {
    256
}

impl TextureRecipe {
    /// The effective roughness band, honouring `smoothness` when given.
    pub fn rough_band(&self) -> [f32; 2] {
        match self.smoothness {
            Some([a, b]) => [1.0 - a, 1.0 - b],
            None => self.roughness,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if !KINDS.contains(&self.kind.as_str()) {
            return Err(format!(
                "unknown texture kind '{}' — one of {KINDS:?}",
                self.kind
            ));
        }
        if self.colors.is_empty() {
            return Err("texture needs at least one color".into());
        }
        if self.colors.len() > 8 {
            return Err("texture ramp: at most 8 colors".into());
        }
        if let Some(over) = &self.over {
            if over.over.is_some() {
                return Err(
                    "texture layering is one level deep (the overlay cannot itself have `over`)"
                        .into(),
                );
            }
            over.validate()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_and_full_forms_parse_and_validate() {
        let t: TextureRecipe = serde_json::from_str(r#"{ "kind": "fbm" }"#).unwrap();
        t.validate().unwrap();
        assert_eq!(t.size, 256);
        let t: TextureRecipe = serde_json::from_str(
            r#"{ "kind": "voronoi", "scale": 6, "seed": 3,
                 "colors": [[0.4,0.4,0.42],[0.55,0.53,0.5]],
                 "roughness": [0.95, 0.8], "height": 0.6, "ao": 0.5 }"#,
        )
        .unwrap();
        t.validate().unwrap();
        let bad: TextureRecipe = serde_json::from_str(r#"{ "kind": "plaid" }"#).unwrap();
        assert!(bad.validate().is_err());
    }
}
