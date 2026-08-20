//! Preview — a model, rendered to a PNG, with no GPU and no browser.
//!
//! An agent that cannot *see* its model is working blind, and the usual fix
//! (spin up a renderer, take a screenshot) costs seconds and a display. This
//! is a small software rasterizer that shades the model with the very maps
//! the exporter writes — base color, normal, roughness, metallic, occlusion —
//! so the preview answers the question that matters: *does the material
//! read?* It draws a turntable contact sheet by default, because one angle
//! hides more than it shows.
//!
//! It is deliberately not the engine: no shadows, no global illumination, a
//! studio light rig. It is a proof sheet, and it renders in milliseconds.

use crate::model::Built;

/// Preview settings. `Default` is a 3-view turntable at 512×512 per view.
#[derive(Debug, Clone, Copy)]
pub struct PreviewOptions {
    pub width: u32,
    pub height: u32,
    /// Turntable views laid out left to right (1 = a single three-quarter view).
    pub views: u32,
    /// Camera pitch in degrees above the horizon.
    pub pitch: f32,
    /// Supersampling factor (2 = 4× samples per pixel).
    pub ss: u32,
}

impl Default for PreviewOptions {
    fn default() -> Self {
        PreviewOptions {
            width: 512,
            height: 512,
            views: 3,
            pitch: 18.0,
            ss: 2,
        }
    }
}

/// Render the model to an RGBA8 buffer `(pixels, width, height)`.
pub fn render(built: &Built, opts: PreviewOptions) -> (Vec<u8>, u32, u32) {
    let views = opts.views.clamp(1, 6);
    let vw = opts.width.max(64);
    let vh = opts.height.max(64);
    let ss = opts.ss.clamp(1, 3);
    let (sw, sh) = (vw * ss, vh * ss);
    let out_w = vw * views;
    let mut out = vec![0u8; (out_w * vh * 4) as usize];

    let (bmin, bmax) = built.bounds();
    let center = [
        (bmin[0] + bmax[0]) / 2.0,
        (bmin[1] + bmax[1]) / 2.0,
        (bmin[2] + bmax[2]) / 2.0,
    ];
    let radius = {
        let d = [bmax[0] - bmin[0], bmax[1] - bmin[1], bmax[2] - bmin[2]];
        (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(0.05) / 2.0
    };

    for v in 0..views {
        // Turntable: start three-quarter, sweep the rest of the way round.
        let yaw = (35.0 + v as f32 * 360.0 / views as f32).to_radians();
        let pitch = opts.pitch.to_radians();
        let dist = radius * 3.1;
        let eye = [
            center[0] + dist * yaw.cos() * pitch.cos(),
            center[1] + dist * pitch.sin(),
            center[2] + dist * yaw.sin() * pitch.cos(),
        ];
        let tile = render_view(built, eye, center, radius, sw, sh);
        // Box-downsample the supersampled tile into the sheet.
        for y in 0..vh {
            for x in 0..vw {
                let mut acc = [0f32; 4];
                for sy in 0..ss {
                    for sx in 0..ss {
                        let i = (((y * ss + sy) * sw + (x * ss + sx)) * 4) as usize;
                        for c in 0..4 {
                            acc[c] += tile[i + c] as f32;
                        }
                    }
                }
                let n = (ss * ss) as f32;
                let o = (((y * out_w) + v * vw + x) * 4) as usize;
                for c in 0..4 {
                    out[o + c] = (acc[c] / n).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
    }
    (out, out_w, vh)
}

/// Render the model to a PNG file.
pub fn write_png(built: &Built, opts: PreviewOptions, path: &str) -> Result<(), String> {
    let (pixels, w, h) = render(built, opts);
    let img = image::RgbaImage::from_raw(w, h, pixels).ok_or("preview buffer size")?;
    img.save(path).map_err(|e| e.to_string())
}

fn render_view(
    built: &Built,
    eye: [f32; 3],
    target: [f32; 3],
    radius: f32,
    w: u32,
    h: u32,
) -> Vec<u8> {
    let mut color = vec![0f32; (w * h * 3) as usize];
    let mut depth = vec![f32::INFINITY; (w * h) as usize];

    // Studio backdrop: a soft vertical gradient, darker at the edges.
    for y in 0..h {
        let t = y as f32 / h as f32;
        let g = 0.30 - 0.16 * t;
        for x in 0..w {
            let i = ((y * w + x) * 3) as usize;
            let vign = {
                let (dx, dy) = (x as f32 / w as f32 - 0.5, y as f32 / h as f32 - 0.5);
                1.0 - 0.55 * (dx * dx + dy * dy)
            };
            color[i] = g * 0.95 * vign;
            color[i + 1] = g * 1.0 * vign;
            color[i + 2] = g * 1.12 * vign;
        }
    }

    // Camera basis (right-handed, looking at the target).
    let fwd = norm(sub(target, eye));
    let right = norm(cross(fwd, [0.0, 1.0, 0.0]));
    let up = cross(right, fwd);
    let aspect = w as f32 / h as f32;
    let fov = 42f32.to_radians();
    let tan_half = (fov / 2.0).tan();
    let (near, far) = (radius * 0.05, radius * 12.0);

    // The light rig: a warm key over the shoulder, a cool fill, a rim.
    let key_dir = norm([0.55, 0.75, 0.38]);
    let fill_dir = norm([-0.6, 0.25, -0.35]);
    let rim_dir = norm([-0.25, 0.35, -0.9]);

    for part in &built.parts {
        let m = &part.mesh;
        for tri in m.indices.chunks(3) {
            let idx = [tri[0] as usize, tri[1] as usize, tri[2] as usize];
            // World → view → clip.
            let mut screen = [[0f32; 3]; 3];
            let mut invw = [0f32; 3];
            let mut ok = true;
            for k in 0..3 {
                let p = m.positions[idx[k]];
                let rel = sub(p, eye);
                let vz = dot(rel, fwd);
                if vz <= near {
                    ok = false;
                    break;
                }
                let vx = dot(rel, right);
                let vy = dot(rel, up);
                let ndc_x = vx / (vz * tan_half * aspect);
                let ndc_y = vy / (vz * tan_half);
                screen[k] = [
                    (ndc_x * 0.5 + 0.5) * w as f32,
                    (1.0 - (ndc_y * 0.5 + 0.5)) * h as f32,
                    vz,
                ];
                invw[k] = 1.0 / vz;
            }
            if !ok {
                continue;
            }
            // Backface cull (screen-space winding; CCW-outside → negative area).
            let area = (screen[1][0] - screen[0][0]) * (screen[2][1] - screen[0][1])
                - (screen[2][0] - screen[0][0]) * (screen[1][1] - screen[0][1]);
            if area >= 0.0 {
                continue;
            }
            let minx = screen
                .iter()
                .map(|s| s[0])
                .fold(f32::MAX, f32::min)
                .floor()
                .max(0.0) as u32;
            let maxx =
                (screen.iter().map(|s| s[0]).fold(f32::MIN, f32::max).ceil()).min(w as f32 - 1.0);
            let miny = screen
                .iter()
                .map(|s| s[1])
                .fold(f32::MAX, f32::min)
                .floor()
                .max(0.0) as u32;
            let maxy =
                (screen.iter().map(|s| s[1]).fold(f32::MIN, f32::max).ceil()).min(h as f32 - 1.0);
            if maxx < 0.0 || maxy < 0.0 {
                continue;
            }
            for py in miny..=(maxy as u32) {
                for px in minx..=(maxx as u32) {
                    let (fx, fy) = (px as f32 + 0.5, py as f32 + 0.5);
                    // Barycentrics.
                    let w0 = edge(screen[1], screen[2], fx, fy);
                    let w1 = edge(screen[2], screen[0], fx, fy);
                    let w2 = edge(screen[0], screen[1], fx, fy);
                    if !((w0 <= 0.0 && w1 <= 0.0 && w2 <= 0.0)
                        || (w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0))
                    {
                        continue;
                    }
                    let sum = w0 + w1 + w2;
                    if sum.abs() < 1e-12 {
                        continue;
                    }
                    let (b0, b1, b2) = (w0 / sum, w1 / sum, w2 / sum);
                    // Perspective-correct interpolation weights.
                    let iw = b0 * invw[0] + b1 * invw[1] + b2 * invw[2];
                    if iw <= 0.0 {
                        continue;
                    }
                    let z = 1.0 / iw;
                    if z > far {
                        continue;
                    }
                    let di = (py * w + px) as usize;
                    if z >= depth[di] {
                        continue;
                    }
                    let pw = [b0 * invw[0] / iw, b1 * invw[1] / iw, b2 * invw[2] / iw];

                    let interp2 = |f: &dyn Fn(usize) -> [f32; 2]| -> [f32; 2] {
                        let (a, b, c) = (f(idx[0]), f(idx[1]), f(idx[2]));
                        [
                            a[0] * pw[0] + b[0] * pw[1] + c[0] * pw[2],
                            a[1] * pw[0] + b[1] * pw[1] + c[1] * pw[2],
                        ]
                    };
                    let interp3 = |f: &dyn Fn(usize) -> [f32; 3]| -> [f32; 3] {
                        let (a, b, c) = (f(idx[0]), f(idx[1]), f(idx[2]));
                        [
                            a[0] * pw[0] + b[0] * pw[1] + c[0] * pw[2],
                            a[1] * pw[0] + b[1] * pw[1] + c[1] * pw[2],
                            a[2] * pw[0] + b[2] * pw[1] + c[2] * pw[2],
                        ]
                    };

                    let n_geo = norm(interp3(&|i| m.normals[i]));
                    let uv = interp2(&|i| m.uvs.get(i).copied().unwrap_or([0.0; 2]));
                    let vcol = interp3(&|i| {
                        let c = m.colors.get(i).copied().unwrap_or([1.0; 4]);
                        [c[0], c[1], c[2]]
                    });

                    // Material sample (the exact maps the GLB carries).
                    let (mut albedo, mut rough, mut metal, mut occl, mut normal) = (
                        [part.color[0], part.color[1], part.color[2]],
                        0.85f32,
                        0.0f32,
                        1.0f32,
                        n_geo,
                    );
                    if let Some(b) = &part.baked {
                        let a = sample(&b.albedo, b.size, uv);
                        albedo = [
                            a[0] * part.color[0] * vcol[0],
                            a[1] * part.color[1] * vcol[1],
                            a[2] * part.color[2] * vcol[2],
                        ];
                        let orm = sample(&b.orm, b.size, uv);
                        occl = orm[0];
                        rough = orm[1].clamp(0.04, 1.0);
                        metal = orm[2];
                        // Tangent-space normal → world, via the interpolated frame.
                        let tn = sample(&b.normal, b.size, uv);
                        let tsn = [tn[0] * 2.0 - 1.0, tn[1] * 2.0 - 1.0, tn[2] * 2.0 - 1.0];
                        let t4 = m
                            .tangents
                            .get(idx[0])
                            .copied()
                            .unwrap_or([1.0, 0.0, 0.0, 1.0]);
                        let t = norm(sub(
                            [t4[0], t4[1], t4[2]],
                            scale(n_geo, dot(n_geo, [t4[0], t4[1], t4[2]])),
                        ));
                        let bt = scale(cross(n_geo, t), t4[3]);
                        normal = norm([
                            t[0] * tsn[0] + bt[0] * tsn[1] + n_geo[0] * tsn[2],
                            t[1] * tsn[0] + bt[1] * tsn[1] + n_geo[1] * tsn[2],
                            t[2] * tsn[0] + bt[2] * tsn[1] + n_geo[2] * tsn[2],
                        ]);
                    } else {
                        albedo = [
                            albedo[0] * vcol[0],
                            albedo[1] * vcol[1],
                            albedo[2] * vcol[2],
                        ];
                    }

                    let vdir = norm(sub(eye, world_point(m, idx, pw)));
                    let mut lit = [0f32; 3];
                    for (dir, tint, energy) in [
                        (key_dir, [1.0, 0.96, 0.90], 0.95f32),
                        (fill_dir, [0.72, 0.80, 1.0], 0.28),
                        (rim_dir, [0.9, 0.92, 1.0], 0.30),
                    ] {
                        let ndl = dot(normal, dir).max(0.0);
                        if ndl <= 0.0 {
                            continue;
                        }
                        // Lambert diffuse (metals don't diffuse) + Blinn-Phong
                        // specular standing in for GGX at preview quality.
                        let half = norm([dir[0] + vdir[0], dir[1] + vdir[1], dir[2] + vdir[2]]);
                        let ndh = dot(normal, half).max(0.0);
                        let shine = (2.0 / (rough * rough).max(1e-3) - 2.0).clamp(2.0, 2048.0);
                        let spec = ndh.powf(shine) * (1.0 - rough) * energy;
                        let kd = 1.0 - metal;
                        for c in 0..3 {
                            let spec_tint = metal * albedo[c] + (1.0 - metal) * 1.0;
                            lit[c] += ndl * energy * tint[c] * (albedo[c] * kd * 0.85)
                                + spec * spec_tint * tint[c] * 0.6;
                        }
                    }
                    // Ambient: sky above, bounce below, modulated by occlusion.
                    let sky = (normal[1] * 0.5 + 0.5).clamp(0.0, 1.0);
                    for c in 0..3 {
                        let amb = [0.13, 0.15, 0.20][c] * sky + [0.07, 0.06, 0.05][c] * (1.0 - sky);
                        lit[c] += albedo[c] * amb * occl * (1.0 - metal * 0.7);
                        lit[c] *= 0.35 + 0.65 * occl;
                        lit[c] += albedo[c] * part.emissive;
                    }
                    depth[di] = z;
                    let ci = di * 3;
                    for c in 0..3 {
                        // Reinhard + gamma, matching the engine's tonemap feel.
                        let v = lit[c].max(0.0);
                        color[ci + c] = (v / (1.0 + v)).powf(1.0 / 2.2);
                    }
                }
            }
        }
    }

    let mut rgba = vec![255u8; (w * h * 4) as usize];
    for i in 0..(w * h) as usize {
        for c in 0..3 {
            rgba[i * 4 + c] = (color[i * 3 + c].clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        }
    }
    rgba
}

fn world_point(m: &crate::MeshData, idx: [usize; 3], pw: [f32; 3]) -> [f32; 3] {
    let (a, b, c) = (
        m.positions[idx[0]],
        m.positions[idx[1]],
        m.positions[idx[2]],
    );
    [
        a[0] * pw[0] + b[0] * pw[1] + c[0] * pw[2],
        a[1] * pw[0] + b[1] * pw[1] + c[1] * pw[2],
        a[2] * pw[0] + b[2] * pw[1] + c[2] * pw[2],
    ]
}

/// Bilinear, wrapping sample of an RGBA8 map → 0..1 RGB.
fn sample(buf: &[u8], size: u32, uv: [f32; 2]) -> [f32; 3] {
    let s = size as f32;
    let (u, v) = (uv[0] * s, uv[1] * s);
    let (x0, y0) = (u.floor(), v.floor());
    let (fx, fy) = (u - x0, v - y0);
    let at = |xi: f32, yi: f32| -> [f32; 3] {
        let x = ((xi as i64).rem_euclid(size as i64)) as usize;
        let y = ((yi as i64).rem_euclid(size as i64)) as usize;
        let i = (y * size as usize + x) * 4;
        [
            buf[i] as f32 / 255.0,
            buf[i + 1] as f32 / 255.0,
            buf[i + 2] as f32 / 255.0,
        ]
    };
    let (c00, c10, c01, c11) = (
        at(x0, y0),
        at(x0 + 1.0, y0),
        at(x0, y0 + 1.0),
        at(x0 + 1.0, y0 + 1.0),
    );
    let mut out = [0f32; 3];
    for c in 0..3 {
        let a = c00[c] + (c10[c] - c00[c]) * fx;
        let b = c01[c] + (c11[c] - c01[c]) * fx;
        out[c] = a + (b - a) * fy;
    }
    out
}

fn edge(a: [f32; 3], b: [f32; 3], px: f32, py: f32) -> f32 {
    (b[0] - a[0]) * (py - a[1]) - (b[1] - a[1]) * (px - a[0])
}
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn scale(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn norm(a: [f32; 3]) -> [f32; 3] {
    let l = dot(a, a).sqrt().max(1e-9);
    [a[0] / l, a[1] / l, a[2] / l]
}

#[cfg(test)]
mod tests {
    use super::*;
    use infinite_manifest::model::Model;

    #[test]
    fn a_preview_draws_the_model_over_the_backdrop() {
        let model: Model = serde_json::from_str(
            r#"{ "name": "ball",
                 "nodes": [ { "prim": "sphere", "r": 1.0 } ],
                 "materials": [ { "texture": { "kind": "voronoi", "colors": [[0.8,0.3,0.2],[0.9,0.8,0.6]],
                                                "height": 0.6, "ao": 0.5, "size": 64 } } ] }"#,
        )
        .unwrap();
        let built = crate::model::build(&model).unwrap();
        let opts = PreviewOptions {
            width: 96,
            height: 96,
            views: 2,
            ss: 1,
            ..Default::default()
        };
        let (px, w, h) = render(&built, opts);
        assert_eq!((w, h), (192, 96), "two views side by side");
        assert_eq!(px.len(), (w * h * 4) as usize);

        // The subject is drawn: the centre of each view is brighter and
        // warmer than the backdrop corner (the sphere is lit and red-ish).
        for v in 0..2u32 {
            let cx = v * 96 + 48;
            let centre = ((48 * w + cx) * 4) as usize;
            let corner = ((4 * w + v * 96 + 4) * 4) as usize;
            assert!(
                px[centre] > px[corner] + 10,
                "view {v}: subject brighter than backdrop ({} vs {})",
                px[centre],
                px[corner]
            );
            assert!(
                px[centre] > px[centre + 2],
                "view {v}: the red material reads red"
            );
        }
    }

    #[test]
    fn previews_are_deterministic() {
        let model: Model =
            serde_json::from_str(r#"{ "nodes": [ { "prim": "box", "w": 1, "h": 1, "d": 1 } ] }"#)
                .unwrap();
        let built = crate::model::build(&model).unwrap();
        let o = PreviewOptions {
            width: 64,
            height: 64,
            views: 1,
            ss: 1,
            ..Default::default()
        };
        assert_eq!(render(&built, o).0, render(&built, o).0);
    }
}
