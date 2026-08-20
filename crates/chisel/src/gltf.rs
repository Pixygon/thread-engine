//! GLB export — carved shapes as standard glTF 2.0 binaries.
//!
//! A creation shouldn't be trapped in its editor: `write_glb` packs a meshed
//! [`crate::MeshData`] (positions, normals, UVs, tangents, indices) and an
//! optionally baked PBR set ([`crate::texture::Baked`], embedded as PNGs)
//! into one self-contained `.glb` any tool opens — Blender, Unity, a web
//! viewer, another Thread browser. The exporter mirrors the loader's
//! conventions (CCW-outside winding, glTF ORM packing), and the round-trip is
//! tested against the engine's own glTF reader.

use std::io::Cursor;

use crate::texture::Baked;
use crate::MeshData;

/// One mesh entry of an exported scene.
pub struct SceneMesh<'a> {
    pub name: String,
    pub mesh: &'a MeshData,
    pub baked: Option<&'a Baked>,
    /// Flat base color factor (used alone when no baked maps; multiplies when
    /// there are — the same semantics the browser renders).
    pub base_color: [f32; 4],
    /// Glow strength; >0 writes an `emissiveFactor` (and the strength
    /// extension when it exceeds 1) so the glow survives the export.
    pub emissive: f32,
}

/// One placed node of an exported scene (TRS, glTF conventions).
pub struct SceneNode {
    pub name: String,
    /// Index into the meshes slice.
    pub mesh: usize,
    pub translation: [f32; 3],
    /// Quaternion `[x y z w]`.
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

/// Pack a single mesh (+ optional baked material) into a binary glTF.
pub fn write_glb(mesh: &MeshData, baked: Option<&Baked>, name: &str) -> Result<Vec<u8>, String> {
    write_glb_scene(
        &[SceneMesh {
            name: name.into(),
            mesh,
            baked,
            base_color: [1.0; 4],
            emissive: 0.0,
        }],
        &[SceneNode {
            name: name.into(),
            mesh: 0,
            translation: [0.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0; 3],
        }],
    )
}

/// Pack a whole scene — many meshes, many placed nodes — into one `.glb`.
/// Meshes are shared across nodes (instancing by reference, the glTF way).
pub fn write_glb_scene(meshes: &[SceneMesh], nodes: &[SceneNode]) -> Result<Vec<u8>, String> {
    let mut bin: Vec<u8> = Vec::new();
    let mut views: Vec<serde_json::Value> = Vec::new();
    let mut push_view = |bin: &mut Vec<u8>, bytes: &[u8]| -> usize {
        while bin.len() % 4 != 0 {
            bin.push(0);
        }
        let offset = bin.len();
        bin.extend_from_slice(bytes);
        views.push(serde_json::json!({
            "buffer": 0, "byteOffset": offset, "byteLength": bytes.len(),
        }));
        views.len() - 1
    };

    let f32s = |v: &[f32]| -> Vec<u8> { v.iter().flat_map(|f| f.to_le_bytes()).collect() };
    let mut accessors: Vec<serde_json::Value> = Vec::new();
    let mut images: Vec<serde_json::Value> = Vec::new();
    let mut textures: Vec<serde_json::Value> = Vec::new();
    let mut materials: Vec<serde_json::Value> = Vec::new();
    let mut gltf_meshes: Vec<serde_json::Value> = Vec::new();

    for sm in meshes {
        let mesh = sm.mesh;
        let iv = push_view(
            &mut bin,
            &mesh
                .indices
                .iter()
                .flat_map(|i| i.to_le_bytes())
                .collect::<Vec<_>>(),
        );
        let pv = push_view(
            &mut bin,
            &f32s(&mesh.positions.iter().flatten().copied().collect::<Vec<_>>()),
        );
        let nv = push_view(
            &mut bin,
            &f32s(&mesh.normals.iter().flatten().copied().collect::<Vec<_>>()),
        );
        let uv = push_view(
            &mut bin,
            &f32s(&mesh.uvs.iter().flatten().copied().collect::<Vec<_>>()),
        );
        let tv = push_view(
            &mut bin,
            &f32s(&mesh.tangents.iter().flatten().copied().collect::<Vec<_>>()),
        );
        let cv = push_view(
            &mut bin,
            &f32s(&mesh.colors.iter().flatten().copied().collect::<Vec<_>>()),
        );

        // Position bounds (required by the spec for POSITION accessors).
        let mut pmin = [f32::INFINITY; 3];
        let mut pmax = [f32::NEG_INFINITY; 3];
        for p in &mesh.positions {
            for c in 0..3 {
                pmin[c] = pmin[c].min(p[c]);
                pmax[c] = pmax[c].max(p[c]);
            }
        }
        let vcount = mesh.positions.len();
        let a0 = accessors.len();
        accessors.push(serde_json::json!({ "bufferView": iv, "componentType": 5125, "count": mesh.indices.len(), "type": "SCALAR" }));
        accessors.push(serde_json::json!({ "bufferView": pv, "componentType": 5126, "count": vcount, "type": "VEC3", "min": pmin, "max": pmax }));
        accessors.push(serde_json::json!({ "bufferView": nv, "componentType": 5126, "count": vcount, "type": "VEC3" }));
        accessors.push(serde_json::json!({ "bufferView": uv, "componentType": 5126, "count": vcount, "type": "VEC2" }));
        accessors.push(serde_json::json!({ "bufferView": tv, "componentType": 5126, "count": vcount, "type": "VEC4" }));
        accessors.push(serde_json::json!({ "bufferView": cv, "componentType": 5126, "count": vcount, "type": "VEC4" }));

        // The complete PBR set, glTF-canonical: base color, the ORM texture
        // read twice (metallicRoughness takes G/B, occlusion takes R — the
        // standard packing every engine expects), a tangent-space normal map,
        // and emissive. Nothing here needs an artist to finish it.
        let emissive_rgb = if sm.emissive > 0.0 {
            let e = sm.emissive.min(1.0);
            [
                sm.base_color[0] * e,
                sm.base_color[1] * e,
                sm.base_color[2] * e,
            ]
        } else {
            [0.0, 0.0, 0.0]
        };
        let material = if let Some(b) = sm.baked {
            let mut tex_indices = Vec::new();
            for rgba in [&b.albedo, &b.orm, &b.normal] {
                let img = image::RgbaImage::from_raw(b.size, b.size, rgba.to_vec())
                    .ok_or("bad baked buffer size")?;
                let mut out = Cursor::new(Vec::new());
                img.write_to(&mut out, image::ImageFormat::Png)
                    .map_err(|e| e.to_string())?;
                let view = push_view(&mut bin, &out.into_inner());
                images.push(serde_json::json!({
                    "bufferView": view, "mimeType": "image/png", "name": format!("{}-{}", sm.name, images.len()),
                }));
                textures.push(serde_json::json!({ "sampler": 0, "source": images.len() - 1 }));
                tex_indices.push(textures.len() - 1);
            }
            let mut mat = serde_json::json!({
                "name": sm.name,
                "pbrMetallicRoughness": {
                    "baseColorFactor": sm.base_color,
                    "baseColorTexture": { "index": tex_indices[0] },
                    "metallicRoughnessTexture": { "index": tex_indices[1] },
                    "metallicFactor": 1.0, "roughnessFactor": 1.0,
                },
                "occlusionTexture": { "index": tex_indices[1], "strength": 1.0 },
                "normalTexture": { "index": tex_indices[2], "scale": 1.0 },
                "emissiveFactor": emissive_rgb,
                "doubleSided": false,
            });
            if sm.emissive > 1.0 {
                mat["extensions"] = serde_json::json!({
                    "KHR_materials_emissive_strength": { "emissiveStrength": sm.emissive },
                });
            }
            mat
        } else {
            serde_json::json!({
                "name": sm.name,
                "pbrMetallicRoughness": {
                    "baseColorFactor": sm.base_color,
                    "metallicFactor": 0.0, "roughnessFactor": 1.0,
                },
                "emissiveFactor": emissive_rgb,
                "doubleSided": false,
            })
        };
        materials.push(material);
        gltf_meshes.push(serde_json::json!({
            "name": sm.name,
            "primitives": [{
                "attributes": { "POSITION": a0 + 1, "NORMAL": a0 + 2, "TEXCOORD_0": a0 + 3,
                                 "TANGENT": a0 + 4, "COLOR_0": a0 + 5 },
                "indices": a0,
                "material": materials.len() - 1,
            }],
        }));
    }

    let gltf_nodes: Vec<serde_json::Value> = nodes
        .iter()
        .map(|n| {
            serde_json::json!({
                "name": n.name, "mesh": n.mesh,
                "translation": n.translation, "rotation": n.rotation, "scale": n.scale,
            })
        })
        .collect();
    let uses_emissive_strength = meshes.iter().any(|m| m.emissive > 1.0);
    let json = serde_json::json!({
        "asset": { "version": "2.0", "generator": "chisel (the Thread)" },
        "extensionsUsed": if uses_emissive_strength {
            vec!["KHR_materials_emissive_strength"]
        } else {
            Vec::new()
        },
        "scene": 0,
        "scenes": [{ "nodes": (0..nodes.len()).collect::<Vec<_>>() }],
        "nodes": gltf_nodes,
        "meshes": gltf_meshes,
        "materials": materials,
        "samplers": [{ "wrapS": 10497, "wrapT": 10497 }],
        "images": images,
        "textures": textures,
        "accessors": accessors,
        "bufferViews": views,
        "buffers": [{ "byteLength": bin.len() }],
    });

    // GLB container: header + JSON chunk (space-padded) + BIN chunk (zero-padded).
    let mut json_bytes = serde_json::to_vec(&json).map_err(|e| e.to_string())?;
    while json_bytes.len() % 4 != 0 {
        json_bytes.push(b' ');
    }
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let total = 12 + 8 + json_bytes.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json_bytes);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
    Ok(out)
}
