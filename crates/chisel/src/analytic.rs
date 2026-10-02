//! Exact meshes for single primitives.
//!
//! The surface-nets carve is the right tool where volumes meet — a blend, a
//! cut, a doorway — but for a lone rounded box or cylinder it is the wrong
//! one: it spends tens of thousands of triangles on a flat face and still
//! only approximates a 3 cm rounding. A primitive has a formula, so we build
//! its surface from the formula, the way any modelling tool does: a cube grid
//! bent onto its rounding radius, a revolved profile for anything turned, a
//! parametric sphere, ellipsoid and torus. Exact positions, exact normals, a
//! few hundred to a couple of thousand triangles. The placement (axis, tilt,
//! yaw, offset) is the same transform the SDF evaluates, so both paths agree.

use infinite_manifest::shape::Prim;

use crate::MeshData;

type V3 = [f32; 3];

fn norm(v: V3) -> V3 {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-12);
    [v[0] / l, v[1] / l, v[2] / l]
}

/// Mesh a primitive exactly, or `None` for shapes this module does not draw.
/// `detail` is the carve's resolution (cells along the longest axis); it sets
/// how finely curves are divided, so a part keeps the density it asked for.
pub fn mesh_prim(p: &Prim, detail: u32) -> Option<MeshData> {
    let detail = detail.clamp(8, 160) as f32;
    let mut m = match p.prim.as_str() {
        "box" => rounded_box(p.size.unwrap_or([1.0; 3]), p.rounded, detail),
        "cylinder" => {
            let rd = p.rounded.clamp(0.0, p.r.min(p.h / 2.0));
            revolve(&rounded_column(p.r, p.h, rd, rd, detail), around(p.r, detail))
        }
        "cone" => revolve(&cone_profile(p.r, p.r2, p.h), around(p.r.max(p.r2), detail)),
        "capsule" => revolve(&capsule_profile(p.r, p.h, detail), around(p.r, detail)),
        "sphere" => ellipsoid([p.r; 3], detail),
        "ellipsoid" => {
            let s = p.size.unwrap_or([1.0; 3]);
            ellipsoid([s[0] / 2.0, s[1] / 2.0, s[2] / 2.0], detail)
        }
        "torus" => torus(p.r, p.r2, detail),
        _ => return None,
    };
    place(&mut m, p);
    orient(&mut m);
    Some(m)
}

/// Target edge length along a curve, in metres, at the default detail (40):
/// a 3 cm chord reads round at room distance. Higher detail divides finer.
fn edge(detail: f32) -> f32 {
    0.045 * 40.0 / detail
}

/// Segments around a circle of radius `r`: by its real size, so a leaf gets
/// a few dozen and a table top as many as it needs.
fn around(r: f32, detail: f32) -> usize {
    ((std::f32::consts::TAU * r / edge(detail)).ceil() as usize).clamp(14, 48)
}

/// Divisions of a quarter arc of radius `r`.
fn quarter_arc(r: f32, detail: f32) -> usize {
    ((std::f32::consts::FRAC_PI_2 * r / edge(detail)).ceil() as usize).clamp(2, 6)
}

/// Axis swizzle, tilt (Z, then X), yaw, offset: the inverse of what `eval`
/// undoes, applied to positions and normals alike.
fn place(m: &mut MeshData, p: &Prim) {
    let swz = |v: V3| -> V3 {
        match p.axis.as_str() {
            "x" => [v[1], v[0], v[2]],
            "z" => [v[0], v[2], v[1]],
            _ => v,
        }
    };
    let rz = p.rz.to_radians();
    let rx = p.rx.to_radians();
    let ry = p.rot.to_radians();
    let rot = |v: V3| -> V3 {
        let mut v = swz(v);
        if rz != 0.0 {
            let (s, c) = rz.sin_cos();
            v = [v[0] * c - v[1] * s, v[0] * s + v[1] * c, v[2]];
        }
        if rx != 0.0 {
            let (s, c) = rx.sin_cos();
            v = [v[0], v[1] * c - v[2] * s, v[1] * s + v[2] * c];
        }
        if ry != 0.0 {
            // the same yaw sense as `local` undoes
            let (s, c) = ry.sin_cos();
            v = [v[0] * c + v[2] * s, v[1], -v[0] * s + v[2] * c];
        }
        v
    };
    for v in &mut m.positions {
        let r = rot(*v);
        *v = [r[0] + p.at[0], r[1] + p.at[1], r[2] + p.at[2]];
    }
    for n in &mut m.normals {
        *n = norm(rot(*n));
    }
}

/// Wind every triangle counter-clockwise seen from outside: compare its face
/// normal with its vertices' normals and flip the ones that disagree. Robust
/// to the mirror an axis swizzle makes.
fn orient(m: &mut MeshData) {
    for t in m.indices.chunks_exact_mut(3) {
        let (a, b, c) = (m.positions[t[0] as usize], m.positions[t[1] as usize], m.positions[t[2] as usize]);
        let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let f = [e1[1] * e2[2] - e1[2] * e2[1], e1[2] * e2[0] - e1[0] * e2[2], e1[0] * e2[1] - e1[1] * e2[0]];
        let n = [0usize, 1, 2].iter().fold([0.0f32; 3], |s, k| {
            let v = m.normals[t[*k] as usize];
            [s[0] + v[0], s[1] + v[1], s[2] + v[2]]
        });
        if f[0] * n[0] + f[1] * n[1] + f[2] * n[2] < 0.0 {
            t.swap(1, 2);
        }
    }
}

fn push(m: &mut MeshData, p: V3, n: V3) -> u32 {
    m.positions.push(p);
    m.normals.push(n);
    m.colors.push([1.0; 4]);
    (m.positions.len() - 1) as u32
}

/// A box of full size `s` with corners rounded by `r` (inside the size): each
/// face is a grid whose outer bands are bent onto the rounding, so flats stay
/// two triangles wide and curves get the divisions.
fn rounded_box(s: V3, r: f32, detail: f32) -> MeshData {
    let h = [s[0] / 2.0, s[1] / 2.0, s[2] / 2.0];
    let r = r.clamp(0.0, h[0].min(h[1]).min(h[2]));
    let bands = if r <= 0.0 { 0 } else { quarter_arc(r, detail) };
    // coordinates along one axis: the rounding band, the flat, the band again
    let coords = |hh: f32| -> Vec<f32> {
        let inner = hh - r;
        let mut c = vec![-hh];
        for k in 1..bands {
            let a = (k as f32 / bands as f32) * std::f32::consts::FRAC_PI_2;
            c.push(-inner - r * a.cos());
        }
        c.push(-inner);
        if inner > 1e-6 {
            c.push(inner);
        }
        for k in (1..bands).rev() {
            let a = (k as f32 / bands as f32) * std::f32::consts::FRAC_PI_2;
            c.push(inner + r * a.cos());
        }
        c.push(hh);
        c.dedup_by(|a, b| (*a - *b).abs() < 1e-7);
        c
    };
    let cs = [coords(h[0]), coords(h[1]), coords(h[2])];
    let mut m = MeshData::default();
    // (fixed axis, sign, u axis, v axis)
    for (ax, sign) in [(0usize, 1.0f32), (0, -1.0), (1, 1.0), (1, -1.0), (2, 1.0), (2, -1.0)] {
        let (u, v) = match ax {
            0 => (1usize, 2usize),
            1 => (0, 2),
            _ => (0, 1),
        };
        let (cu, cv) = (&cs[u], &cs[v]);
        let base = m.positions.len() as u32;
        for &a in cu {
            for &b in cv {
                let mut q = [0.0f32; 3];
                q[ax] = sign * h[ax];
                q[u] = a;
                q[v] = b;
                let inner = [q[0].clamp(-(h[0] - r), h[0] - r), q[1].clamp(-(h[1] - r), h[1] - r), q[2].clamp(-(h[2] - r), h[2] - r)];
                let d = [q[0] - inner[0], q[1] - inner[1], q[2] - inner[2]];
                let dl = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                let (pos, n) = if r > 0.0 && dl > 1e-7 {
                    let n = [d[0] / dl, d[1] / dl, d[2] / dl];
                    ([inner[0] + n[0] * r, inner[1] + n[1] * r, inner[2] + n[2] * r], n)
                } else {
                    let mut n = [0.0; 3];
                    n[ax] = sign;
                    (q, n)
                };
                push(&mut m, pos, n);
            }
        }
        let nv = cv.len() as u32;
        for i in 0..cu.len() as u32 - 1 {
            for j in 0..nv - 1 {
                let a = base + i * nv + j;
                let (b, c, d) = (a + nv, a + nv + 1, a + 1);
                m.indices.extend_from_slice(&[a, b, c, a, c, d]);
            }
        }
    }
    m
}

/// A profile point in the (radius, height) plane with its outward normal.
type Prof = Vec<([f32; 2], [f32; 2])>;

/// A cylinder side with its two rims rounded by `rb` (bottom) and `rt` (top);
/// zero is a sharp rim (the normal splits there).
fn rounded_column(r: f32, h: f32, rb: f32, rt: f32, detail: f32) -> Prof {
    let (y0, y1) = (-h / 2.0, h / 2.0);
    let arc = |rd: f32| -> usize { if rd <= 0.0 { 0 } else { quarter_arc(rd, detail).max(3) } };
    let mut p: Prof = vec![([0.0, y0], [0.0, -1.0])];
    if rb > 0.0 {
        let n = arc(rb);
        for k in 0..=n {
            let a = -std::f32::consts::FRAC_PI_2 + (k as f32 / n as f32) * std::f32::consts::FRAC_PI_2;
            p.push(([r - rb + rb * a.cos(), y0 + rb + rb * a.sin()], [a.cos(), a.sin()]));
        }
    } else {
        p.push(([r, y0], [0.0, -1.0]));
        p.push(([r, y0], [1.0, 0.0]));
    }
    if rt > 0.0 {
        let n = arc(rt);
        for k in 0..=n {
            let a = (k as f32 / n as f32) * std::f32::consts::FRAC_PI_2;
            p.push(([r - rt + rt * a.cos(), y1 - rt + rt * a.sin()], [a.cos(), a.sin()]));
        }
    } else {
        p.push(([r, y1], [1.0, 0.0]));
        p.push(([r, y1], [0.0, 1.0]));
    }
    p.push(([0.0, y1], [0.0, 1.0]));
    p
}

/// A capped cone, radius `r` at the bottom to `r2` at the top.
fn cone_profile(r: f32, r2: f32, h: f32) -> Prof {
    let (y0, y1) = (-h / 2.0, h / 2.0);
    let side = norm([h, 0.0, r - r2]);
    let sn = [side[0], side[2]];
    let mut p: Prof = vec![([0.0, y0], [0.0, -1.0]), ([r, y0], [0.0, -1.0]), ([r, y0], sn), ([r2.max(0.0), y1], sn)];
    if r2 > 1e-5 {
        p.push(([r2, y1], [0.0, 1.0]));
        p.push(([0.0, y1], [0.0, 1.0]));
    }
    p
}

/// Two hemispheres joined by a side; `h` is the full length.
fn capsule_profile(r: f32, h: f32, detail: f32) -> Prof {
    let half = (h / 2.0 - r).max(0.0);
    let n = quarter_arc(r, detail).max(3);
    let mut p: Prof = Vec::new();
    for k in 0..=n {
        let a = -std::f32::consts::FRAC_PI_2 + (k as f32 / n as f32) * std::f32::consts::FRAC_PI_2;
        p.push(([r * a.cos(), -half + r * a.sin()], [a.cos(), a.sin()]));
    }
    for k in 0..=n {
        let a = (k as f32 / n as f32) * std::f32::consts::FRAC_PI_2;
        p.push(([r * a.cos(), half + r * a.sin()], [a.cos(), a.sin()]));
    }
    p
}

/// Turn a profile around the Y axis.
fn revolve(profile: &Prof, segments: usize) -> MeshData {
    let mut m = MeshData::default();
    let rows = profile.len() as u32;
    for s in 0..=segments {
        let a = s as f32 / segments as f32 * std::f32::consts::TAU;
        let (sn, cs) = a.sin_cos();
        for (pt, n) in profile {
            push(&mut m, [pt[0] * cs, pt[1], pt[0] * sn], norm([n[0] * cs, n[1], n[0] * sn]));
        }
    }
    for s in 0..segments as u32 {
        for i in 0..rows - 1 {
            let a = s * rows + i;
            let (b, c, d) = (a + rows, a + rows + 1, a + 1);
            // skip the zero-area quads a split normal makes (same point twice)
            let (pa, pd) = (profile[i as usize].0, profile[i as usize + 1].0);
            if (pa[0] - pd[0]).abs() < 1e-7 && (pa[1] - pd[1]).abs() < 1e-7 {
                continue;
            }
            // on the axis a quad is a triangle: drop the half with no area
            if pa[0] > 1e-6 {
                m.indices.extend_from_slice(&[a, b, c]);
            }
            if pd[0] > 1e-6 {
                m.indices.extend_from_slice(&[a, c, d]);
            }
        }
    }
    m
}

fn ellipsoid(r: V3, detail: f32) -> MeshData {
    // around the waist by the waist's mean radius; a flat leaf needs no more
    let seg = around((r[0] + r[2]) / 2.0, detail);
    let rings = (seg / 2).max(7);
    let mut m = MeshData::default();
    for i in 0..=rings {
        let th = i as f32 / rings as f32 * std::f32::consts::PI;
        let (st, ct) = th.sin_cos();
        for j in 0..=seg {
            let ph = j as f32 / seg as f32 * std::f32::consts::TAU;
            let (sp, cp) = ph.sin_cos();
            let u = [st * cp, ct, st * sp];
            let pos = [u[0] * r[0], u[1] * r[1], u[2] * r[2]];
            let n = norm([u[0] / r[0], u[1] / r[1], u[2] / r[2]]);
            push(&mut m, pos, n);
        }
    }
    let row = seg as u32 + 1;
    for i in 0..rings as u32 {
        for j in 0..seg as u32 {
            let a = i * row + j;
            let (b, c, d) = (a + row, a + row + 1, a + 1);
            if i != 0 {
                m.indices.extend_from_slice(&[a, b, d]);
            }
            if i != rings as u32 - 1 {
                m.indices.extend_from_slice(&[d, b, c]);
            }
        }
    }
    m
}

fn torus(big: f32, small: f32, detail: f32) -> MeshData {
    let seg = around(big + small, detail);
    let tube = around(small, detail).min(24);
    let mut m = MeshData::default();
    for i in 0..=seg {
        let a = i as f32 / seg as f32 * std::f32::consts::TAU;
        let (sa, ca) = a.sin_cos();
        for j in 0..=tube {
            let b = j as f32 / tube as f32 * std::f32::consts::TAU;
            let (sb, cb) = b.sin_cos();
            let n = [cb * ca, sb, cb * sa];
            push(&mut m, [(big + small * cb) * ca, small * sb, (big + small * cb) * sa], n);
        }
    }
    let row = tube as u32 + 1;
    for i in 0..seg as u32 {
        for j in 0..tube as u32 {
            let a = i * row + j;
            m.indices.extend_from_slice(&[a, a + row, a + row + 1, a, a + row + 1, a + 1]);
        }
    }
    m
}

/// A lathe (a polyline profile of `[radius, height]` points, closed back to
/// the axis) turned around Y. Where the profile bends gently the normal is
/// shared, so a cup's belly shades smooth; at a sharp turn (a rim, a base)
/// it splits, so the edge stays crisp.
pub fn mesh_lathe(points: &[[f32; 2]], at: [f32; 3], detail: u32) -> Option<MeshData> {
    if points.len() < 2 {
        return None;
    }
    let detail = detail.clamp(8, 160) as f32;
    // close onto the axis at both ends, as the SDF lathe does
    let mut pts: Vec<[f32; 2]> = Vec::new();
    if points[0][0] > 1e-5 {
        pts.push([0.0, points[0][1]]);
    }
    pts.extend_from_slice(points);
    let last = *points.last().unwrap();
    if last[0] > 1e-5 {
        pts.push([0.0, last[1]]);
    }
    pts.dedup_by(|a, b| (a[0] - b[0]).abs() < 1e-7 && (a[1] - b[1]).abs() < 1e-7);
    // outward normal of each segment: the profile may run either way, so
    // orient by the side the segment's midpoint faces relative to the axis
    let seg_n: Vec<[f32; 2]> = pts
        .windows(2)
        .map(|w| {
            let (d0, d1) = (w[1][0] - w[0][0], w[1][1] - w[0][1]);
            let l = (d0 * d0 + d1 * d1).sqrt().max(1e-9);
            [d1 / l, -d0 / l]
        })
        .collect();
    // decide the profile's winding once (the shoelace sign) so normals point out
    let area: f32 = pts.windows(2).map(|w| w[0][0] * w[1][1] - w[1][0] * w[0][1]).sum::<f32>()
        + (pts[pts.len() - 1][0] * pts[0][1] - pts[0][0] * pts[pts.len() - 1][1]);
    let s = if area > 0.0 { 1.0 } else { -1.0 };
    let seg_n: Vec<[f32; 2]> = seg_n.into_iter().map(|n| [n[0] * s, n[1] * s]).collect();
    let smooth = 0.86f32; // cos 30°
    let mut prof: Prof = Vec::new();
    for (i, p) in pts.iter().enumerate() {
        let before = if i > 0 { Some(seg_n[i - 1]) } else { None };
        let after = seg_n.get(i).copied();
        match (before, after) {
            (Some(a), Some(b)) if a[0] * b[0] + a[1] * b[1] > smooth => {
                let n = norm([a[0] + b[0], a[1] + b[1], 0.0]);
                prof.push((*p, [n[0], n[1]]));
            }
            (Some(a), Some(b)) => {
                prof.push((*p, a));
                prof.push((*p, b));
            }
            (Some(a), None) => prof.push((*p, a)),
            (None, Some(b)) => prof.push((*p, b)),
            _ => {}
        }
    }
    let rmax = pts.iter().map(|p| p[0]).fold(0.0f32, f32::max);
    let mut m = revolve(&prof, around(rmax, detail));
    for v in &mut m.positions {
        *v = [v[0] + at[0], v[1] + at[1], v[2] + at[2]];
    }
    orient(&mut m);
    Some(m)
}
