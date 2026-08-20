//! Building models: carve every part, bake every material, hand back
//! something a renderer or an exporter can use directly.
//!
//! This is the join between the [format](infinite_manifest::model) an agent
//! writes (or a Weft program computes) and the geometry a machine draws. It
//! is deliberately the *only* place that decides defaults, so a model made
//! by any route — hand-written JSON, a Weft program, a generator — comes out
//! PBR-complete: base color, normal, metallic, roughness, occlusion.

use infinite_manifest::model::{Model, PartMaterial};

use crate::texture::Baked;
use crate::{MeshData, MeshOptions, UvMode};

/// One carved, textured part.
pub struct BuiltPart {
    pub name: String,
    pub mesh: MeshData,
    /// The baked PBR set. `None` only when the part declared no recipe.
    pub baked: Option<Baked>,
    pub color: [f32; 4],
    pub emissive: f32,
}

/// A finished model: named parts, each with geometry and materials.
pub struct Built {
    pub name: String,
    pub parts: Vec<BuiltPart>,
}

impl Built {
    pub fn triangles(&self) -> usize {
        self.parts.iter().map(|p| p.mesh.indices.len() / 3).sum()
    }
    pub fn vertices(&self) -> usize {
        self.parts.iter().map(|p| p.mesh.positions.len()).sum()
    }
    /// World bounds over every part (for framing a camera).
    pub fn bounds(&self) -> ([f32; 3], [f32; 3]) {
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        for part in &self.parts {
            for p in &part.mesh.positions {
                for c in 0..3 {
                    min[c] = min[c].min(p[c]);
                    max[c] = max[c].max(p[c]);
                }
            }
        }
        if min[0] > max[0] {
            return ([0.0; 3], [1.0; 3]);
        }
        (min, max)
    }
}

/// Carve + bake a model.
pub fn build(model: &Model) -> Result<Built, String> {
    model.validate()?;
    let parts = model
        .resolve()?
        .into_iter()
        .map(|rp| {
            let m: &PartMaterial = &rp.material;
            let mesh = mesh_components(
                &rp.shape,
                MeshOptions {
                    resolution: m.resolution,
                    uv: UvMode::parse(&m.uv),
                    uv_scale: if m.uv_scale > 0.0 { m.uv_scale } else { 0.5 },
                },
            );
            BuiltPart {
                name: rp.name,
                mesh,
                baked: m.texture.as_ref().map(crate::texture::bake),
                color: m.color,
                emissive: m.emissive,
            }
        })
        .collect();
    Ok(Built {
        name: model.name.clone(),
        parts,
    })
}

/// Mesh a shape, splitting plain unions into their own grids first.
///
/// This matters more than it sounds. A carve is sampled over one grid across
/// the whole shape's bounds, so a staircase four metres long gets centimetre
/// cells and mushy treads. But the steps of a staircase are a *union* — they
/// don't melt into each other — so each can be carved on its own tight grid
/// and the results concatenated. Crisper geometry, less of it, and faster:
/// twelve small grids are far cheaper than one big one. Blends, cuts and
/// intersections still mesh together, because there the parts genuinely
/// interact.
fn mesh_components(shape: &infinite_manifest::shape::Shape, opts: MeshOptions) -> MeshData {
    use infinite_manifest::shape::Shape;
    let splittable = match shape {
        Shape::Group(g) => {
            (g.op == "union") && g.at == [0.0; 3] && g.rot == 0.0 && g.parts.len() > 1
        }
        _ => false,
    };
    if !splittable {
        return crate::mesh_with(shape, opts);
    }
    let Shape::Group(g) = shape else {
        unreachable!()
    };
    let mut out = MeshData::default();
    for part in &g.parts {
        let piece = mesh_components(part, opts);
        let base = out.positions.len() as u32;
        out.positions.extend(piece.positions);
        out.normals.extend(piece.normals);
        out.uvs.extend(piece.uvs);
        out.tangents.extend(piece.tangents);
        out.colors.extend(piece.colors);
        out.indices.extend(piece.indices.iter().map(|i| i + base));
    }
    out
}

/// Export a built model as one self-contained binary glTF — every part a
/// mesh, every material complete.
pub fn export_glb(built: &Built) -> Result<Vec<u8>, String> {
    let meshes: Vec<crate::gltf::SceneMesh> = built
        .parts
        .iter()
        .map(|p| crate::gltf::SceneMesh {
            name: p.name.clone(),
            mesh: &p.mesh,
            baked: p.baked.as_ref(),
            base_color: p.color,
            emissive: p.emissive,
        })
        .collect();
    let nodes: Vec<crate::gltf::SceneNode> = built
        .parts
        .iter()
        .enumerate()
        .map(|(i, p)| crate::gltf::SceneNode {
            name: p.name.clone(),
            mesh: i,
            translation: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0; 3],
        })
        .collect();
    crate::gltf::write_glb_scene(&meshes, &nodes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LANTERN: &str = r#"{
      "name": "lantern",
      "nodes": [
        { "prim": "box", "w": 0.4, "h": 0.5, "d": 0.4, "y": 0.6, "round": 0.06 },
        { "prim": "box", "mode": "cut", "w": 0.3, "h": 0.36, "d": 0.6, "y": 0.6 },
        { "prim": "cylinder", "mode": "add", "r": 0.05, "h": 0.3, "y": 1.0 },
        { "prim": "sphere", "part": 1, "r": 0.12, "y": 0.6 }
      ],
      "materials": [
        { "name": "iron", "texture": { "kind": "fbm", "colors": [[0.2,0.19,0.22],[0.35,0.33,0.36]],
                                        "smoothness": [0.5, 0.7], "metallic": [0.9, 1.0], "height": 0.2, "size": 64 } },
        { "name": "flame", "color": [1.0, 0.7, 0.3, 1.0], "emissive": 1.0 }
      ]
    }"#;

    #[test]
    fn a_model_builds_pbr_complete_parts_and_exports() {
        let model: Model = serde_json::from_str(LANTERN).unwrap();
        let built = build(&model).unwrap();
        assert_eq!(built.parts.len(), 2, "two materials → two parts");
        assert!(
            built.triangles() > 100,
            "geometry: {} tris",
            built.triangles()
        );

        let iron = &built.parts[0];
        let baked = iron.baked.as_ref().expect("recipe baked");
        // Smoothness (Unity spelling) inverted into roughness: 0.5→0.5, 0.7→0.3.
        let roughs: Vec<u8> = baked.orm.chunks(4).map(|p| p[1]).collect();
        let min = *roughs.iter().min().unwrap() as f32 / 255.0;
        let max = *roughs.iter().max().unwrap() as f32 / 255.0;
        assert!(
            min > 0.25 && max < 0.55,
            "smoothness → roughness band: {min}..{max}"
        );
        // Metallic band came through.
        assert!(baked.orm.chunks(4).all(|p| p[2] > 200), "metal");
        // The emissive part carries its glow.
        assert_eq!(built.parts[1].emissive, 1.0);

        let glb = export_glb(&built).unwrap();
        assert_eq!(&glb[0..4], b"glTF");
        // The glTF declares a complete PBR material for the textured part.
        let json = String::from_utf8_lossy(&glb[20..(20 + 4000).min(glb.len())]).to_string();
        for key in [
            "baseColorTexture",
            "metallicRoughnessTexture",
            "occlusionTexture",
            "normalTexture",
            "emissiveFactor",
        ] {
            assert!(json.contains(key), "glTF declares {key}");
        }
    }

    #[test]
    fn the_smallest_legal_model_still_comes_out_textured() {
        let model: Model =
            serde_json::from_str(r#"{ "nodes": [ { "prim": "sphere", "r": 0.5 } ] }"#).unwrap();
        let built = build(&model).unwrap();
        assert_eq!(built.parts.len(), 1);
        assert!(!built.parts[0].mesh.uvs.is_empty(), "UVs regardless");
        assert!(
            !built.parts[0].mesh.tangents.is_empty(),
            "tangents regardless"
        );
        assert!(export_glb(&built).is_ok());
    }
}
