//! `thread view <file.glb> [-o sheet.png] [--views n]` — a turntable proof
//! sheet for ANY glb, not only the ones chisel carved.
//!
//! The Quarry and `thread model` prove their own output with a 3-view
//! turntable; a mesh that arrived from anywhere else (an image-to-3D model,
//! an artist's export, a Unity asset) had no equivalent, so it was judged by
//! its triangle count and a description — which is how five bland desert
//! props were called "right" (2026-09-25). Same renderer, same lights, same
//! honesty: what the sheet shows is what the file contains.
//!
//! What it reads: positions, normals, uvs, indices, and the base-colour
//! texture or factor of every primitive. Roughness/metallic come from the
//! material factors (their textures are folded to a flat ORM), the normal
//! map is flat. Emissive factor is carried as the part's emissive strength.
//! Skins and animations are ignored: this is a still.
use std::process::ExitCode;

use chisel::model::{Built, BuiltPart};
use chisel::texture::Baked;
use chisel::MeshData;

pub fn cmd_view(args: &[String]) -> ExitCode {
    let mut file: Option<&String> = None;
    let mut out: Option<&String> = None;
    let mut views: u32 = 3;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-o" | "--out" => out = it.next(),
            "--views" => views = it.next().and_then(|v| v.parse().ok()).unwrap_or(3),
            s if s.starts_with("--") => {}
            _ => file = Some(a),
        }
    }
    let Some(file) = file else {
        eprintln!("usage: thread view <file.glb> [-o sheet.png] [--views n]");
        return ExitCode::from(2);
    };
    let built = match load(file) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("✗ {file}: {e}");
            return ExitCode::from(1);
        }
    };
    let (min, max) = built.bounds();
    let size = [max[0] - min[0], max[1] - min[1], max[2] - min[2]];
    let out_path = out
        .cloned()
        .unwrap_or_else(|| format!("{}.png", file.trim_end_matches(".glb").trim_end_matches(".gltf")));
    let opts = chisel::preview::PreviewOptions { views, ..Default::default() };
    match chisel::preview::write_png(&built, opts, &out_path) {
        Ok(()) => {
            println!(
                "✓ {} — {} part(s), {} tris, {:.2} × {:.2} × {:.2} m → {out_path} ({views}-view turntable)",
                built.name,
                built.parts.len(),
                built.triangles(),
                size[0],
                size[1],
                size[2]
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("✗ preview: {e}");
            ExitCode::from(1)
        }
    }
}

pub(crate) fn load(path: &str) -> Result<Built, String> {
    let (doc, buffers, images) = gltf::import(path).map_err(|e| format!("cannot read glTF: {e}"))?;
    let name = std::path::Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "model".into());
    let mut parts: Vec<BuiltPart> = Vec::new();

    // Walk the scene so node transforms apply (a Hunyuan/Trellis glb is one
    // node, an artist's may be many); fall back to bare meshes.
    let mut stack: Vec<(gltf::Node, [[f32; 4]; 4])> = Vec::new();
    let scene = doc.default_scene().or_else(|| doc.scenes().next());
    if let Some(scene) = scene {
        for n in scene.nodes() {
            stack.push((n, identity()));
        }
    }
    let mut visited_any = false;
    while let Some((node, parent)) = stack.pop() {
        let local = node.transform().matrix();
        let world = mul(&parent, &local);
        if let Some(mesh) = node.mesh() {
            visited_any = true;
            for prim in mesh.primitives() {
                if let Some(part) = primitive_part(&prim, &buffers, &images, &world, mesh.name()) {
                    parts.push(part);
                }
            }
        }
        for c in node.children() {
            stack.push((c, world));
        }
    }
    if !visited_any {
        for mesh in doc.meshes() {
            for prim in mesh.primitives() {
                if let Some(part) = primitive_part(&prim, &buffers, &images, &identity(), mesh.name()) {
                    parts.push(part);
                }
            }
        }
    }
    if parts.is_empty() {
        return Err("no triangle primitives".into());
    }
    Ok(Built { name, parts })
}

fn primitive_part(
    prim: &gltf::Primitive,
    buffers: &[gltf::buffer::Data],
    images: &[gltf::image::Data],
    world: &[[f32; 4]; 4],
    mesh_name: Option<&str>,
) -> Option<BuiltPart> {
    if prim.mode() != gltf::mesh::Mode::Triangles {
        return None;
    }
    let reader = prim.reader(|b| Some(&buffers[b.index()]));
    let positions: Vec<[f32; 3]> = reader.read_positions()?.map(|p| xform(world, p)).collect();
    let normals: Vec<[f32; 3]> = match reader.read_normals() {
        Some(n) => n.map(|v| xform_dir(world, v)).collect(),
        None => vec![[0.0, 1.0, 0.0]; positions.len()],
    };
    let uvs: Vec<[f32; 2]> = match reader.read_tex_coords(0) {
        Some(t) => t.into_f32().collect(),
        None => vec![[0.0, 0.0]; positions.len()],
    };
    let indices: Vec<u32> = match reader.read_indices() {
        Some(i) => i.into_u32().collect(),
        None => (0..positions.len() as u32).collect(),
    };
    let mat = prim.material();
    let pbr = mat.pbr_metallic_roughness();
    let color = pbr.base_color_factor();
    let emissive_f = mat.emissive_factor();
    let emissive = emissive_f.iter().cloned().fold(0.0_f32, f32::max);

    // Base-colour texture → square albedo; roughness/metallic factors → flat ORM.
    let baked = pbr.base_color_texture().and_then(|info| {
        let img = images.get(info.texture().source().index())?;
        let rgba = to_rgba(img)?;
        let size = 512u32;
        let resized = image::imageops::resize(&rgba, size, size, image::imageops::FilterType::Triangle);
        let albedo = resized.into_raw();
        let n = (size * size) as usize;
        let normal = [128u8, 128, 255, 255].repeat(n);
        let orm = [255u8, (pbr.roughness_factor() * 255.0) as u8, (pbr.metallic_factor() * 255.0) as u8, 255].repeat(n);
        Some(Baked { size, albedo, normal, orm })
    });

    let mesh = MeshData {
        positions,
        normals,
        uvs,
        tangents: Vec::new(),
        colors: Vec::new(),
        indices,
    };
    Some(BuiltPart {
        name: mat.name().or(mesh_name).unwrap_or("part").to_string(),
        mesh,
        baked,
        color,
        emissive,
    })
}

fn to_rgba(img: &gltf::image::Data) -> Option<image::RgbaImage> {
    use gltf::image::Format;
    let (w, h) = (img.width, img.height);
    let px = &img.pixels;
    let mut out = image::RgbaImage::new(w, h);
    let n = (w * h) as usize;
    match img.format {
        Format::R8G8B8A8 => {
            for i in 0..n {
                out.put_pixel((i as u32) % w, (i as u32) / w, image::Rgba([px[i * 4], px[i * 4 + 1], px[i * 4 + 2], px[i * 4 + 3]]));
            }
        }
        Format::R8G8B8 => {
            for i in 0..n {
                out.put_pixel((i as u32) % w, (i as u32) / w, image::Rgba([px[i * 3], px[i * 3 + 1], px[i * 3 + 2], 255]));
            }
        }
        Format::R8 => {
            for i in 0..n {
                out.put_pixel((i as u32) % w, (i as u32) / w, image::Rgba([px[i], px[i], px[i], 255]));
            }
        }
        _ => return None,
    }
    Some(out)
}

fn identity() -> [[f32; 4]; 4] {
    [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]]
}
/// Column-major 4×4 multiply (glTF convention), `a * b`.
fn mul(a: &[[f32; 4]; 4], b: &[[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut m = [[0.0; 4]; 4];
    for c in 0..4 {
        for r in 0..4 {
            m[c][r] = (0..4).map(|k| a[k][r] * b[c][k]).sum();
        }
    }
    m
}
fn xform(m: &[[f32; 4]; 4], p: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * p[0] + m[1][0] * p[1] + m[2][0] * p[2] + m[3][0],
        m[0][1] * p[0] + m[1][1] * p[1] + m[2][1] * p[2] + m[3][1],
        m[0][2] * p[0] + m[1][2] * p[1] + m[2][2] * p[2] + m[3][2],
    ]
}
fn xform_dir(m: &[[f32; 4]; 4], v: [f32; 3]) -> [f32; 3] {
    let d = [
        m[0][0] * v[0] + m[1][0] * v[1] + m[2][0] * v[2],
        m[0][1] * v[0] + m[1][1] * v[1] + m[2][1] * v[2],
        m[0][2] * v[0] + m[1][2] * v[1] + m[2][2] * v[2],
    ];
    let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-6);
    [d[0] / l, d[1] / l, d[2] / l]
}
