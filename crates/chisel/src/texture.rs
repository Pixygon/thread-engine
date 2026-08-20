//! The texture baker: [`TextureRecipe`] → the full PBR set.
//!
//! One deterministic, seamlessly-tiling pattern drives everything: its value
//! indexes the color ramp (albedo), lerps roughness/metallic, acts as a
//! height field for the tangent-space normal map, and darkens crevices into
//! the occlusion channel. All integer-hash noise — same recipe, same texels,
//! on every machine (the determinism rule Weft set for code, kept for pixels).

use infinite_manifest::texture::{TextureRecipe, MAX_SIZE};

/// Baked RGBA8 maps, each `size × size`. `orm` is glTF-packed:
/// R = occlusion, G = roughness, B = metallic.
pub struct Baked {
    pub size: u32,
    pub albedo: Vec<u8>,
    pub normal: Vec<u8>,
    pub orm: Vec<u8>,
}

/// Bake a recipe. Height/AO derive from the same pattern field, so the maps
/// agree with each other — crevices are dark AND dented. A layered recipe
/// (`over`) bakes both and blends every map by an fbm mask — moss creeping
/// over granite, rust blooming on iron.
pub fn bake(t: &TextureRecipe) -> Baked {
    let base = bake_single(t);
    let Some(over) = &t.over else { return base };
    if t.mix <= 0.0 {
        return base; // a declared-but-unused layer costs nothing
    }
    let mut top_recipe = (**over).clone();
    top_recipe.size = t.size; // the blend needs matching texel grids
    let top = bake_single(&top_recipe);
    let size = base.size as usize;
    let mut out = base;
    for y in 0..size {
        for x in 0..size {
            let u = x as f32 / size as f32 * t.mask_scale;
            let v = y as f32 / size as f32 * t.mask_scale;
            let mut m = 0.0;
            let mut amp = 0.5;
            let mut freq = 1.0;
            for o in 0..3u32 {
                m += amp
                    * value_noise(
                        u * freq,
                        v * freq,
                        (t.mask_scale.max(1.0).round() as i64) * (1 << o),
                        t.mask_seed.wrapping_add(77 + o),
                    );
                amp *= 0.5;
                freq *= 2.0;
            }
            // mask noise ∈ ~[-0.5, 0.5]; coverage rises with `mix`.
            let blend = smoothstep((0.5 - t.mix) - 0.15, (0.5 - t.mix) + 0.15, m + 0.5);
            let i = (y * size + x) * 4;
            for (dst, src) in [
                (&mut out.albedo, &top.albedo),
                (&mut out.orm, &top.orm),
                (&mut out.normal, &top.normal),
            ] {
                for c in 0..4 {
                    let a = dst[i + c] as f32;
                    let b = src[i + c] as f32;
                    dst[i + c] = (a + (b - a) * blend) as u8;
                }
            }
        }
    }
    out
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn bake_single(t: &TextureRecipe) -> Baked {
    let size = t.size.clamp(16, MAX_SIZE) as usize;
    let n = size * size;
    let mut value = vec![0.0f32; n];
    for y in 0..size {
        for x in 0..size {
            // Sample in tile space [0, scale) — every pattern wraps at `scale`.
            let u = x as f32 / size as f32 * t.scale;
            let v = y as f32 / size as f32 * t.scale;
            value[y * size + x] = pattern(t, u, v);
        }
    }

    let mut albedo = vec![0u8; n * 4];
    let mut orm = vec![0u8; n * 4];
    let mut normal = vec![0u8; n * 4];
    let to8 = |f: f32| (f.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    for i in 0..n {
        let val = value[i];
        let c = ramp(&t.colors, val);
        albedo[i * 4] = to8(c[0]);
        albedo[i * 4 + 1] = to8(c[1]);
        albedo[i * 4 + 2] = to8(c[2]);
        albedo[i * 4 + 3] = 255;
        let occl = 1.0 - t.ao * (1.0 - val);
        orm[i * 4] = to8(occl);
        let rb = t.rough_band();
        orm[i * 4 + 1] = to8(rb[0] + (rb[1] - rb[0]) * val);
        orm[i * 4 + 2] = to8(t.metallic[0] + (t.metallic[1] - t.metallic[0]) * val);
        orm[i * 4 + 3] = 255;
    }
    // Normal from the pattern as height (wrapped central differences).
    for y in 0..size {
        for x in 0..size {
            let at = |dx: isize, dy: isize| {
                let xx = (x as isize + dx).rem_euclid(size as isize) as usize;
                let yy = (y as isize + dy).rem_euclid(size as isize) as usize;
                value[yy * size + xx]
            };
            let dx = (at(1, 0) - at(-1, 0)) * t.height * size as f32 / 64.0;
            let dy = (at(0, 1) - at(0, -1)) * t.height * size as f32 / 64.0;
            let len = (dx * dx + dy * dy + 1.0).sqrt();
            let i = (y * size + x) * 4;
            normal[i] = to8((-dx / len) * 0.5 + 0.5);
            normal[i + 1] = to8((-dy / len) * 0.5 + 0.5);
            normal[i + 2] = to8((1.0 / len) * 0.5 + 0.5);
            normal[i + 3] = 255;
        }
    }
    Baked {
        size: size as u32,
        albedo,
        normal,
        orm,
    }
}

/// The pattern field at tile-space `(u, v)` — 0..1, wrapping at `t.scale`.
fn pattern(t: &TextureRecipe, u: f32, v: f32) -> f32 {
    let period = t.scale.max(1.0).round() as i64;
    match t.kind.as_str() {
        "fbm" => {
            let mut sum = 0.0;
            let mut amp = 0.5;
            let mut freq = 1.0;
            for o in 0..t.octaves.clamp(1, 8) {
                sum += amp
                    * value_noise(
                        u * freq,
                        v * freq,
                        period * (1 << o),
                        t.seed.wrapping_add(o),
                    );
                amp *= 0.5;
                freq *= 2.0;
            }
            (sum + 0.5).clamp(0.0, 1.0)
        }
        "voronoi" => {
            // F2−F1 cell noise: 0 at cell borders (the cracks), 1 mid-cell.
            let (f1, f2) = voronoi_f1f2(u, v, period, t.seed);
            ((f2 - f1) * 2.5).clamp(0.0, 1.0)
        }
        "bricks" => {
            let row = v.floor();
            let uu = u + if (row as i64).rem_euclid(2) == 1 {
                0.5
            } else {
                0.0
            };
            let (bu, bv) = (uu.fract(), v.fract());
            let mortar = 0.06;
            if bu < mortar || bv < mortar * 2.0 {
                0.0
            } else {
                // Per-brick tone so courses read laid, not printed.
                let id = hash2(
                    (uu.floor() as i64).rem_euclid(period),
                    (row as i64).rem_euclid(period),
                    t.seed,
                );
                0.55 + 0.45 * id
            }
        }
        "wood" => {
            // Plank grain: stripes along u, warped by tiling noise, with a
            // fine second harmonic so the rings read as growth, not wallpaper.
            let warp = value_noise(u * 0.5, v * 0.5, period.max(1), t.seed) * 2.2
                + value_noise(u, v * 0.25, period.max(1), t.seed ^ 0x51) * 0.6;
            let rings = ((u + warp) * 2.0).fract().abs();
            let grain = ((u + warp) * 14.0).fract().abs() * 0.12;
            (0.25 + 0.75 * (1.0 - (rings * 2.0 - 1.0).abs()) - grain).clamp(0.0, 1.0)
        }
        "veins" => {
            // Marble: ridged turbulence — |noise| inverted gives thin dark
            // seams through a bright field, which is what veining is.
            let mut turb = 0.0;
            let mut amp = 0.5;
            let mut freq = 1.0;
            for o in 0..t.octaves.clamp(1, 6) {
                turb += amp
                    * value_noise(
                        u * freq,
                        v * freq,
                        period * (1 << o),
                        t.seed.wrapping_add(o),
                    )
                    .abs();
                amp *= 0.5;
                freq *= 2.0;
            }
            let seam = (1.0 - (turb * 6.0).min(1.0)).powf(2.0);
            (1.0 - seam * 0.85).clamp(0.0, 1.0)
        }
        "checker" => {
            let c = (u.floor() as i64 + v.floor() as i64).rem_euclid(2);
            if c == 0 {
                0.25
            } else {
                0.75
            }
        }
        _ => 0.5, // flat
    }
}

/// Evenly-spaced color ramp lookup.
fn ramp(colors: &[[f32; 3]], t: f32) -> [f32; 3] {
    match colors.len() {
        0 => [0.6, 0.6, 0.6],
        1 => colors[0],
        n => {
            let x = t.clamp(0.0, 1.0) * (n - 1) as f32;
            let i = (x.floor() as usize).min(n - 2);
            let f = x - i as f32;
            let (a, b) = (colors[i], colors[i + 1]);
            [
                a[0] + (b[0] - a[0]) * f,
                a[1] + (b[1] - a[1]) * f,
                a[2] + (b[2] - a[2]) * f,
            ]
        }
    }
}

/// Deterministic lattice hash → 0..1.
fn hash2(x: i64, y: i64, seed: u32) -> f32 {
    let mut h = (x as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add((y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F))
        .wrapping_add(seed as u64);
    h ^= h >> 33;
    h = h.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
    h ^= h >> 33;
    (h & 0xFFFFFF) as f32 / 0xFFFFFF as f32
}

/// Tiling value noise: smoothstep-interpolated lattice hashes, period-wrapped.
fn value_noise(u: f32, v: f32, period: i64, seed: u32) -> f32 {
    let (iu, iv) = (u.floor() as i64, v.floor() as i64);
    let (fu, fv) = (u - iu as f32, v - iv as f32);
    let (su, sv) = (fu * fu * (3.0 - 2.0 * fu), fv * fv * (3.0 - 2.0 * fv));
    let g = |dx: i64, dy: i64| {
        hash2(
            (iu + dx).rem_euclid(period),
            (iv + dy).rem_euclid(period),
            seed,
        ) - 0.5
    };
    let a = g(0, 0) + su * (g(1, 0) - g(0, 0));
    let b = g(0, 1) + su * (g(1, 1) - g(0, 1));
    a + sv * (b - a)
}

/// Tiling cellular noise: distances to the nearest and second-nearest
/// jittered cell points.
fn voronoi_f1f2(u: f32, v: f32, period: i64, seed: u32) -> (f32, f32) {
    let (iu, iv) = (u.floor() as i64, v.floor() as i64);
    let (mut f1, mut f2) = (f32::INFINITY, f32::INFINITY);
    for dy in -1..=1 {
        for dx in -1..=1 {
            let (cx, cy) = (iu + dx, iv + dy);
            let (wx, wy) = (cx.rem_euclid(period), cy.rem_euclid(period));
            let jx = hash2(wx, wy, seed);
            let jy = hash2(wx, wy, seed.wrapping_add(101));
            let px = cx as f32 + jx;
            let py = cy as f32 + jy;
            let d = ((px - u).powi(2) + (py - v).powi(2)).sqrt();
            if d < f1 {
                f2 = f1;
                f1 = d;
            } else if d < f2 {
                f2 = d;
            }
        }
    }
    (f1, f2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recipe(kind: &str) -> TextureRecipe {
        serde_json::from_str(&format!(
            r#"{{ "kind": "{kind}", "scale": 4, "seed": 7,
                 "colors": [[0.2,0.2,0.2],[0.8,0.8,0.8]],
                 "roughness": [0.9, 0.6], "height": 0.8, "ao": 0.5, "size": 64 }}"#
        ))
        .unwrap()
    }

    #[test]
    fn bakes_deterministically_and_tiles_seamlessly() {
        for kind in ["fbm", "voronoi", "bricks", "checker"] {
            let t = recipe(kind);
            let a = bake(&t);
            let b = bake(&t);
            assert_eq!(a.albedo, b.albedo, "{kind} deterministic");
            // Seamless: the pattern field at u and u+scale is identical.
            let s = t.scale;
            for probe in [(0.13, 0.77), (0.5, 0.25), (0.9, 0.9)] {
                let p0 = pattern(&t, probe.0, probe.1);
                let p1 = pattern(&t, probe.0 + s, probe.1 + s);
                assert!((p0 - p1).abs() < 1e-4, "{kind} tiles: {p0} vs {p1}");
            }
            // The maps vary (not flat) except checker's two tones.
            let distinct: std::collections::BTreeSet<u8> =
                a.albedo.iter().step_by(4).copied().collect();
            assert!(distinct.len() >= 2, "{kind} has variation");
        }
    }

    #[test]
    fn normal_map_is_unit_ish_and_orm_packs_the_ranges() {
        let t = recipe("voronoi");
        let b = bake(&t);
        // Normals decode to ~unit vectors, z-positive.
        for px in b.normal.chunks(4).step_by(97) {
            let v = [
                px[0] as f32 / 127.5 - 1.0,
                px[1] as f32 / 127.5 - 1.0,
                px[2] as f32 / 127.5 - 1.0,
            ];
            let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            assert!((len - 1.0).abs() < 0.1, "unit normal (len {len})");
            assert!(v[2] > 0.0, "z-up tangent space");
        }
        // Roughness stays within its declared band.
        for px in b.orm.chunks(4).step_by(53) {
            let r = px[1] as f32 / 255.0;
            assert!((0.58..=0.92).contains(&r), "roughness in band: {r}");
        }
        // AO darkens somewhere (cracks exist).
        assert!(
            b.orm.chunks(4).any(|p| p[0] < 200),
            "crevices darken occlusion"
        );
    }
}
