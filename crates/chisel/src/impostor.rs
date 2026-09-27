//! Impostor — eight views become a cross-quad billboard.
//!
//! A forest never draws a tree per tree at distance. The last LOD is not a
//! coarser mesh but a picture of the model: the previewer already renders
//! turntables, so eight views of it, 45° apart, go into one atlas, and eight
//! single-sided quads — four vertical planes, a face on each side — stand
//! where the model stood, each showing the view rendered from its own side.
//! Whatever angle the camera looks from, the quads facing it carry the views
//! taken from there, and the ones facing away are culled.
//!
//! It is the same pipeline as everything else: a [`Built`] in, a [`Built`]
//! out — one part, one baked material, a base colour map with alpha, so the
//! exporter marks it a cutout and glTFast, Infinite and the previewer all
//! draw it without a special case. The studio lighting bakes in, which is what
//! a billboard at that distance wants.
use crate::model::{Built, BuiltPart};
use crate::preview::{framing, render_tile, PreviewOptions, FOV_DEG};
use crate::texture::Baked;
use crate::MeshData;

/// Views around the model, 45° apart.
pub const VIEWS: u32 = 8;
/// The atlas is a 3 × 3 grid of tiles, eight used: square, as every baked
/// map is, and one tile spare.
const GRID: u32 = 3;

/// The eight-view impostor of a model, as a model: one part, `<name>-impostor`.
///
/// `tile` is the side of one view in the atlas (the atlas is 3 × that). The
/// quads are sized to the previewer's framing, so a pixel of the atlas lands
/// where the model's surface was to within the perspective of the shot.
pub fn impostor(built: &Built, tile: u32) -> Result<Built, String> {
    if built.parts.iter().all(|p| p.mesh.positions.is_empty()) {
        return Err("nothing to make an impostor of".into());
    }
    let tile = tile.clamp(64, 1024);
    let opts = PreviewOptions { width: tile, height: tile, views: 1, pitch: 0.0, ss: 2, fill: 1.0, transparent: true };
    let (center, _radius, dist) = framing(built, opts);
    // What one tile shows, in metres, at the model's centre plane: the
    // previewer's vertical field of view at the distance it framed from.
    let half = dist * (FOV_DEG.to_radians() / 2.0).tan();

    let size = tile * GRID;
    let mut albedo = vec![0u8; (size * size * 4) as usize];
    let mut mesh = MeshData::default();
    for k in 0..VIEWS {
        let yaw_deg = k as f32 * 360.0 / VIEWS as f32;
        let (px, w, h) = render_tile(built, yaw_deg, 0.0, opts);
        let (col, row) = (k % GRID, k / GRID);
        for y in 0..h {
            let src = ((y * w) * 4) as usize;
            let dst = (((row * tile + y) * size + col * tile) * 4) as usize;
            albedo[dst..dst + (w * 4) as usize].copy_from_slice(&px[src..src + (w * 4) as usize]);
        }
        quad(&mut mesh, center, half, yaw_deg, col, row);
    }
    // Flat normal, matte and unlit-by-metal: the light is in the picture.
    let n = (size * size) as usize;
    let normal: Vec<u8> = [128u8, 128, 255, 255].repeat(n);
    let orm: Vec<u8> = [255u8, 235, 0, 255].repeat(n);
    let baked = Baked { size, albedo, normal, orm };
    Ok(Built {
        name: format!("{}-impostor", built.name),
        parts: vec![BuiltPart {
            name: "impostor".into(),
            mesh,
            baked: Some(baked),
            color: [1.0, 1.0, 1.0, 1.0],
            emissive: 0.0,
            double_sided: false,
        }],
    })
}

/// The atlas alone, as PNG bytes — for looking at.
pub fn atlas_png(impostor: &Built) -> Result<Vec<u8>, String> {
    let b = impostor.parts.first().and_then(|p| p.baked.as_ref()).ok_or("no atlas")?;
    let img = image::RgbaImage::from_raw(b.size, b.size, b.albedo.clone()).ok_or("atlas buffer size")?;
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok(out.into_inner())
}

/// One quad facing the camera that took view `k`: the previewer's eye sits at
/// `yaw` around the centre, its screen-right is `(sin yaw, 0, -cos yaw)` and
/// its up is +Y at pitch 0, so the quad spans those two at `half` metres and
/// maps to the tile the view landed in. Wound counter-clockwise as seen from
/// the camera, so back-face culling hides it from behind.
fn quad(m: &mut MeshData, center: [f32; 3], half: f32, yaw_deg: f32, col: u32, row: u32) {
    let (sy, cy) = yaw_deg.to_radians().sin_cos();
    let normal = [cy, 0.0, sy];
    let right = [sy, 0.0, -cy];
    let up = [0.0, 1.0, 0.0];
    let at = |r: f32, u: f32| -> [f32; 3] {
        [
            center[0] + right[0] * r * half + up[0] * u * half,
            center[1] + right[1] * r * half + up[1] * u * half,
            center[2] + right[2] * r * half + up[2] * u * half,
        ]
    };
    let g = GRID as f32;
    let uv = |u: f32, v: f32| -> [f32; 2] { [(col as f32 + u) / g, (row as f32 + v) / g] };
    let base = m.positions.len() as u32;
    // Bottom-left, bottom-right, top-right, top-left — image v runs top-down.
    for (r, u, tu, tv) in [(-1.0, -1.0, 0.0, 1.0), (1.0, -1.0, 1.0, 1.0), (1.0, 1.0, 1.0, 0.0), (-1.0, 1.0, 0.0, 0.0)] {
        m.positions.push(at(r, u));
        m.normals.push(normal);
        m.uvs.push(uv(tu, tv));
        m.tangents.push([right[0], right[1], right[2], 1.0]);
        m.colors.push([1.0, 1.0, 1.0, 1.0]);
    }
    m.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eight_views_become_eight_quads_and_a_cutout_atlas() {
        let cube = Built {
            name: "cube".into(),
            parts: vec![BuiltPart { name: "c".into(), mesh: crate::builtin::cube(), baked: None, color: [0.8, 0.3, 0.2, 1.0], emissive: 0.0, double_sided: false }],
        };
        let imp = impostor(&cube, 64).unwrap();
        assert_eq!(imp.parts.len(), 1);
        let m = &imp.parts[0].mesh;
        assert_eq!(m.positions.len(), 8 * 4);
        assert_eq!(m.indices.len(), 8 * 6);
        let b = imp.parts[0].baked.as_ref().unwrap();
        assert_eq!(b.size, 64 * 3);
        // The cube is in the picture, and so is the air around it.
        let alpha: Vec<u8> = b.albedo.chunks_exact(4).map(|p| p[3]).collect();
        assert!(alpha.iter().any(|a| *a == 255), "something was drawn");
        assert!(alpha.iter().any(|a| *a == 0), "and the rest is transparent");
        // The unused ninth tile is empty.
        let tile = 64usize;
        let (col, row) = (2usize, 2usize);
        let i = ((row * tile + tile / 2) * b.size as usize + col * tile + tile / 2) * 4;
        assert_eq!(b.albedo[i + 3], 0);
        // It exports as a cutout, with the atlas inside.
        let glb = crate::model::export_glb(&imp).unwrap();
        let text = String::from_utf8_lossy(&glb).to_string();
        assert!(text.contains(r#""alphaMode":"MASK""#));
        // Every quad faces outward from the centre and is a unit normal.
        for k in 0..8 {
            let n = m.normals[k * 4];
            let c = m.positions[k * 4..k * 4 + 4].iter().fold([0.0f32; 3], |a, p| [a[0] + p[0] / 4.0, a[1] + p[1] / 4.0, a[2] + p[2] / 4.0]);
            assert!((n[0] * n[0] + n[1] * n[1] + n[2] * n[2] - 1.0).abs() < 1e-4);
            assert!(c[0] * n[0] + c[2] * n[2] > -1e-4, "quad {k} faces its camera");
        }
        // Same model, same impostor.
        let again = impostor(&cube, 64).unwrap();
        assert_eq!(again.parts[0].baked.as_ref().unwrap().albedo, b.albedo);
    }
}
