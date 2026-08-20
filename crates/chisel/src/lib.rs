//! # Chisel — the Thread's mesher
//!
//! Turns a [`Shape`] recipe (a signed-distance tree: primitives, smooth
//! blends, carves, lathed profiles) into a renderable triangle mesh. This is
//! the "Blender for agents" bet made concrete: an agent *describes* the
//! volume it wants — blend two spheres, carve a doorway, lathe a vase profile
//! — and Chisel gives it geometry with correct outward winding and smooth
//! gradient normals. No vertices cross any wire; the recipe is the model.
//!
//! Meshing is **naive surface nets** over a regular grid: one vertex per
//! sign-crossing cell (at the mean of its edge crossings), one quad per
//! sign-crossing grid edge. It handles every boolean/blend an SDF can
//! express, at a resolution the manifest chooses (load-time cost only —
//! never in the frame loop; a 40³ grid meshes in well under a millisecond).

pub mod builtin;
pub mod gltf;
pub mod model;
pub mod preview;
pub mod texture;
pub mod weft_model;

use infinite_manifest::shape::{Group, Lathe, Prim, Shape};

/// Default sampling resolution (grid cells along the longest axis).
pub const DEFAULT_RESOLUTION: u32 = 40;
/// The mesher's hard cap — a manifest cannot make a browser sample a 500³ grid.
pub const MAX_RESOLUTION: u32 = 96;

/// A meshed shape: positions, smooth normals, box-projected UVs + tangents,
/// CCW-outside triangle indices.
#[derive(Debug, Clone, Default)]
pub struct MeshData {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub tangents: Vec<[f32; 4]>,
    /// Weathering baked per vertex from the SDF's curvature: convex edges
    /// lighten (wear), concave crevices darken (grime) — multiplied into
    /// albedo by every PBR renderer that honours vertex color.
    pub colors: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
}

/// Signed distance from `p` to the shape's surface (negative inside).
pub fn eval(shape: &Shape, p: [f32; 3]) -> f32 {
    match shape {
        Shape::Prim(prim) => eval_prim(prim, local(p, prim.at, prim.rot)),
        Shape::Group(g) => eval_group(g, local(p, g.at, g.rot)),
        Shape::Lathe(l) => eval_lathe(l, local(p, l.at, 0.0)),
    }
}

fn local(p: [f32; 3], at: [f32; 3], rot_deg: f32) -> [f32; 3] {
    let q = [p[0] - at[0], p[1] - at[1], p[2] - at[2]];
    if rot_deg == 0.0 {
        return q;
    }
    let a = (-rot_deg).to_radians();
    let (s, c) = a.sin_cos();
    [q[0] * c + q[2] * s, q[1], -q[0] * s + q[2] * c]
}

fn eval_prim(pr: &Prim, p: [f32; 3]) -> f32 {
    // Rotational prims run Y-long internally; `axis` swizzles the sample
    // point so the same math serves lying-down cylinders (arch cuts).
    let p = match pr.axis.as_str() {
        "x" => [p[1], p[0], p[2]],
        "z" => [p[0], p[2], p[1]],
        _ => p,
    };
    let len2 = |x: f32, y: f32| (x * x + y * y).sqrt();
    let len3 = |v: [f32; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    match pr.prim.as_str() {
        "sphere" => len3(p) - pr.r,
        "box" => {
            let b = pr.size.unwrap_or([1.0; 3]);
            let q = [
                p[0].abs() - b[0] / 2.0 + pr.rounded,
                p[1].abs() - b[1] / 2.0 + pr.rounded,
                p[2].abs() - b[2] / 2.0 + pr.rounded,
            ];
            let outside = len3([q[0].max(0.0), q[1].max(0.0), q[2].max(0.0)]);
            outside + q[0].max(q[1]).max(q[2]).min(0.0) - pr.rounded
        }
        "cylinder" => {
            let d = [len2(p[0], p[2]) - pr.r, p[1].abs() - pr.h / 2.0];
            len2(d[0].max(0.0), d[1].max(0.0)) + d[0].max(d[1]).min(0.0)
        }
        "capsule" => {
            let half = (pr.h / 2.0 - pr.r).max(0.0);
            let y = (p[1].abs() - half).max(0.0);
            len3([p[0], y, p[2]]) - pr.r
        }
        "cone" => {
            // A capped cone as a lathe of its trapezoid — exact in the radial plane.
            let profile = [
                [0.0, -pr.h / 2.0],
                [pr.r, -pr.h / 2.0],
                [pr.r2.max(1e-4), pr.h / 2.0],
                [0.0, pr.h / 2.0],
            ];
            polygon2(&profile, [len2(p[0], p[2]), p[1]])
        }
        "torus" => {
            let q = [len2(p[0], p[2]) - pr.r, p[1]];
            len2(q[0], q[1]) - pr.r2
        }
        _ => f32::INFINITY, // unknown prim: contributes nothing (validated upstream)
    }
}

fn eval_group(g: &Group, p: [f32; 3]) -> f32 {
    let mut it = g.parts.iter();
    let first = it.next().expect("validated: parts non-empty");
    let mut d = eval(first, p);
    for part in it {
        let e = eval(part, p);
        d = match g.op.as_str() {
            "blend" => smooth_min(d, e, g.k.max(1e-4)),
            "cut" => d.max(-e),
            "intersect" => d.max(e),
            _ => d.min(e),
        };
    }
    d
}

fn eval_lathe(l: &Lathe, p: [f32; 3]) -> f32 {
    let s = (p[0] * p[0] + p[2] * p[2]).sqrt();
    polygon2(&l.lathe, [s, p[1]])
}

/// Polynomial smooth minimum (the melt in `blend`).
fn smooth_min(a: f32, b: f32, k: f32) -> f32 {
    let h = (0.5 + 0.5 * (b - a) / k).clamp(0.0, 1.0);
    b * (1.0 - h) + a * h - k * h * (1.0 - h)
}

/// Signed distance to a closed 2D polygon (negative inside). Standard
/// edge-distance + winding-parity construction.
fn polygon2(pts: &[[f32; 2]], p: [f32; 2]) -> f32 {
    let n = pts.len();
    let mut d = f32::INFINITY;
    let mut sign = 1.0f32;
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (pts[j], pts[i]);
        let e = [b[0] - a[0], b[1] - a[1]];
        let w = [p[0] - a[0], p[1] - a[1]];
        let t =
            ((w[0] * e[0] + w[1] * e[1]) / (e[0] * e[0] + e[1] * e[1]).max(1e-12)).clamp(0.0, 1.0);
        let dv = [w[0] - e[0] * t, w[1] - e[1] * t];
        d = d.min((dv[0] * dv[0] + dv[1] * dv[1]).sqrt());
        // Winding parity (crossing test).
        let cond = [(p[1] >= a[1]), (p[1] < b[1]), (e[0] * w[1] > e[1] * w[0])];
        if cond.iter().all(|c| *c) || cond.iter().all(|c| !*c) {
            sign = -sign;
        }
        j = i;
    }
    sign * d
}

/// How surface points become texture coordinates. The right projection is
/// most of what separates "textured" from "PBR-ready": a box projection on a
/// turned vase seams down its side, a cylindrical one wraps it like a label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UvMode {
    /// Pick from the shape: cylindrical for lathes and rotational prims,
    /// spherical for spheres, box otherwise.
    Auto,
    /// Per-triangle projection along its dominant axis.
    Box,
    /// Angle around Y → u, height → v (turned things, columns, vases).
    Cylindrical,
    /// Longitude/latitude (balls, domes).
    Spherical,
}

impl UvMode {
    /// Parse the manifest spelling (`auto` · `box` · `cylindrical` · `spherical`).
    pub fn parse(s: &str) -> UvMode {
        match s {
            "box" => UvMode::Box,
            "cylindrical" | "cylinder" => UvMode::Cylindrical,
            "spherical" | "sphere" => UvMode::Spherical,
            _ => UvMode::Auto,
        }
    }

    /// Resolve `Auto` against the shape being meshed.
    fn resolve(self, shape: &Shape) -> UvMode {
        if self != UvMode::Auto {
            return self;
        }
        match shape {
            Shape::Lathe(_) => UvMode::Cylindrical,
            Shape::Prim(p) => match p.prim.as_str() {
                "sphere" => UvMode::Spherical,
                "cylinder" | "capsule" | "cone" | "torus" => UvMode::Cylindrical,
                _ => UvMode::Box,
            },
            // A carve of many parts: whatever the seed part wants.
            Shape::Group(g) => g
                .parts
                .first()
                .map(|p| UvMode::Auto.resolve(p))
                .unwrap_or(UvMode::Box),
        }
    }
}

/// Meshing options. `Default` is the good default: 40³ sampling, automatic
/// projection, half a texture repeat per metre.
#[derive(Debug, Clone, Copy)]
pub struct MeshOptions {
    pub resolution: Option<u32>,
    pub uv: UvMode,
    /// Texture repeats per metre.
    pub uv_scale: f32,
}

impl Default for MeshOptions {
    fn default() -> Self {
        MeshOptions {
            resolution: None,
            uv: UvMode::Auto,
            uv_scale: 0.5,
        }
    }
}

/// Mesh a shape at `resolution` grid cells along its longest axis, with
/// automatic UVs. See [`mesh_with`] for projection control.
pub fn mesh(shape: &Shape, resolution: Option<u32>) -> MeshData {
    mesh_with(
        shape,
        MeshOptions {
            resolution,
            ..Default::default()
        },
    )
}

/// Mesh a shape with explicit options.
pub fn mesh_with(shape: &Shape, opts: MeshOptions) -> MeshData {
    // Exact path for the shape architecture is mostly made of. Marching a
    // plain box wastes thousands of triangles on flat faces AND wobbles
    // them (surface-net vertices sit at edge-crossing averages, so a wall
    // ends up faintly quilted). A box is twelve triangles and six perfect
    // normals; take them.
    if let Shape::Prim(p) = shape {
        if p.prim == "box" && p.rounded == 0.0 {
            return exact_box(p, opts.uv_scale);
        }
    }
    let resolution = opts.resolution;
    let res = resolution
        .unwrap_or(DEFAULT_RESOLUTION)
        .clamp(8, MAX_RESOLUTION) as usize;
    let (bmin, bmax) = shape.bounds();
    // Pad by one cell so the surface never touches the sampling boundary.
    let extent = [bmax[0] - bmin[0], bmax[1] - bmin[1], bmax[2] - bmin[2]];
    let longest = extent[0].max(extent[1]).max(extent[2]).max(1e-3);
    let cell = longest / res as f32;
    let dims = [
        ((extent[0] / cell).ceil() as usize + 3).max(4),
        ((extent[1] / cell).ceil() as usize + 3).max(4),
        ((extent[2] / cell).ceil() as usize + 3).max(4),
    ];
    let origin = [
        bmin[0] - 1.5 * cell,
        bmin[1] - 1.5 * cell,
        bmin[2] - 1.5 * cell,
    ];
    let corner = |i: usize, j: usize, k: usize| {
        [
            origin[0] + i as f32 * cell,
            origin[1] + j as f32 * cell,
            origin[2] + k as f32 * cell,
        ]
    };

    // Sample the field at every lattice corner.
    let (nx, ny, nz) = (dims[0] + 1, dims[1] + 1, dims[2] + 1);
    let idx = |i: usize, j: usize, k: usize| (i * ny + j) * nz + k;
    let mut field = vec![0.0f32; nx * ny * nz];
    for i in 0..nx {
        for j in 0..ny {
            for k in 0..nz {
                field[idx(i, j, k)] = eval(shape, corner(i, j, k));
            }
        }
    }

    // One vertex per sign-crossing cell, at the mean of its edge crossings.
    let mut cell_vertex = vec![u32::MAX; dims[0] * dims[1] * dims[2]];
    let cidx = |i: usize, j: usize, k: usize| (i * dims[1] + j) * dims[2] + k;
    let mut out = MeshData::default();
    const CELL_EDGES: [([usize; 3], [usize; 3]); 12] = [
        ([0, 0, 0], [1, 0, 0]),
        ([0, 1, 0], [1, 1, 0]),
        ([0, 0, 1], [1, 0, 1]),
        ([0, 1, 1], [1, 1, 1]),
        ([0, 0, 0], [0, 1, 0]),
        ([1, 0, 0], [1, 1, 0]),
        ([0, 0, 1], [0, 1, 1]),
        ([1, 0, 1], [1, 1, 1]),
        ([0, 0, 0], [0, 0, 1]),
        ([1, 0, 0], [1, 0, 1]),
        ([0, 1, 0], [0, 1, 1]),
        ([1, 1, 0], [1, 1, 1]),
    ];
    for i in 0..dims[0] {
        for j in 0..dims[1] {
            for k in 0..dims[2] {
                let mut sum = [0.0f32; 3];
                let mut count = 0u32;
                for (a, b) in CELL_EDGES {
                    let fa = field[idx(i + a[0], j + a[1], k + a[2])];
                    let fb = field[idx(i + b[0], j + b[1], k + b[2])];
                    if (fa < 0.0) != (fb < 0.0) {
                        let t = fa / (fa - fb);
                        let pa = corner(i + a[0], j + a[1], k + a[2]);
                        let pb = corner(i + b[0], j + b[1], k + b[2]);
                        for c in 0..3 {
                            sum[c] += pa[c] + (pb[c] - pa[c]) * t;
                        }
                        count += 1;
                    }
                }
                if count > 0 {
                    let v = [
                        sum[0] / count as f32,
                        sum[1] / count as f32,
                        sum[2] / count as f32,
                    ];
                    cell_vertex[cidx(i, j, k)] = out.positions.len() as u32;
                    out.positions.push(v);
                    // Smooth normal: normalized SDF gradient (central differences).
                    let e = cell * 0.5;
                    let g = [
                        eval(shape, [v[0] + e, v[1], v[2]]) - eval(shape, [v[0] - e, v[1], v[2]]),
                        eval(shape, [v[0], v[1] + e, v[2]]) - eval(shape, [v[0], v[1] - e, v[2]]),
                        eval(shape, [v[0], v[1], v[2] + e]) - eval(shape, [v[0], v[1], v[2] - e]),
                    ];
                    let len = (g[0] * g[0] + g[1] * g[1] + g[2] * g[2]).sqrt().max(1e-9);
                    out.normals.push([g[0] / len, g[1] / len, g[2] / len]);
                    // Curvature ≈ the SDF's laplacian: positive = convex edge
                    // (worn bright), negative = concave crevice (grimed dark).
                    let d0 = eval(shape, v);
                    let lap = (eval(shape, [v[0] + e, v[1], v[2]])
                        + eval(shape, [v[0] - e, v[1], v[2]])
                        + eval(shape, [v[0], v[1] + e, v[2]])
                        + eval(shape, [v[0], v[1] - e, v[2]])
                        + eval(shape, [v[0], v[1], v[2] + e])
                        + eval(shape, [v[0], v[1], v[2] - e])
                        - 6.0 * d0)
                        / (e * e);
                    let wear = (lap * cell * 0.8).tanh();
                    let shade = (1.0 + 0.28 * wear).clamp(0.65, 1.25);
                    out.colors.push([shade, shade, shade, 1.0]);
                }
            }
        }
    }

    // One quad per sign-crossing lattice edge, wound CCW seen from outside
    // (the repo convention — guarded by the winding test below).
    let mut quad = |v: [u32; 4], flip: bool| {
        if v.iter().any(|x| *x == u32::MAX) {
            return;
        }
        let [a, b, c, d] = if flip { [v[3], v[2], v[1], v[0]] } else { v };
        out.indices.extend_from_slice(&[a, b, c, a, c, d]);
    };
    for i in 0..dims[0] {
        for j in 0..dims[1] {
            for k in 0..dims[2] {
                // X-edge from corner (i+1, j+1, k+1) — shared by 4 cells in y/z.
                if j + 1 < dims[1] && k + 1 < dims[2] {
                    let fa = field[idx(i, j + 1, k + 1)];
                    let fb = field[idx(i + 1, j + 1, k + 1)];
                    if (fa < 0.0) != (fb < 0.0) {
                        quad(
                            [
                                cell_vertex[cidx(i, j, k)],
                                cell_vertex[cidx(i, j + 1, k)],
                                cell_vertex[cidx(i, j + 1, k + 1)],
                                cell_vertex[cidx(i, j, k + 1)],
                            ],
                            fb < 0.0,
                        );
                    }
                }
                // Y-edge.
                if i + 1 < dims[0] && k + 1 < dims[2] {
                    let fa = field[idx(i + 1, j, k + 1)];
                    let fb = field[idx(i + 1, j + 1, k + 1)];
                    if (fa < 0.0) != (fb < 0.0) {
                        quad(
                            [
                                cell_vertex[cidx(i, j, k)],
                                cell_vertex[cidx(i, j, k + 1)],
                                cell_vertex[cidx(i + 1, j, k + 1)],
                                cell_vertex[cidx(i + 1, j, k)],
                            ],
                            fb < 0.0,
                        );
                    }
                }
                // Z-edge.
                if i + 1 < dims[0] && j + 1 < dims[1] {
                    let fa = field[idx(i + 1, j + 1, k)];
                    let fb = field[idx(i + 1, j + 1, k + 1)];
                    if (fa < 0.0) != (fb < 0.0) {
                        quad(
                            [
                                cell_vertex[cidx(i, j, k)],
                                cell_vertex[cidx(i + 1, j, k)],
                                cell_vertex[cidx(i + 1, j + 1, k)],
                                cell_vertex[cidx(i, j + 1, k)],
                            ],
                            fb < 0.0,
                        );
                    }
                }
            }
        }
    }
    project_uvs(
        &mut out,
        opts.uv.resolve(shape),
        opts.uv_scale,
        center_of(shape),
    );
    weld(&mut out);
    out
}

/// Merge vertices that agree in every attribute. Projection splits every
/// triangle's corners so each face can own exact UVs; welding puts back the
/// ones that never differed — typically a two-thirds cut in vertex count and
/// file size, with the seams (which differ in UV or normal) left split, as
/// they must be.
fn weld(m: &mut MeshData) {
    use std::collections::HashMap;
    let key = |i: usize| -> [u32; 12] {
        let q = |v: f32, scale: f32| (v * scale).round() as i32 as u32;
        let p = m.positions[i];
        let n = m.normals[i];
        let uv = m.uvs.get(i).copied().unwrap_or([0.0; 2]);
        let t = m.tangents.get(i).copied().unwrap_or([1.0, 0.0, 0.0, 1.0]);
        let c = m.colors.get(i).copied().unwrap_or([1.0; 4]);
        [
            q(p[0], 1e5),
            q(p[1], 1e5),
            q(p[2], 1e5),
            q(n[0], 1e4),
            q(n[1], 1e4),
            q(n[2], 1e4),
            q(uv[0], 1e4),
            q(uv[1], 1e4),
            q(t[0], 1e3),
            q(t[3], 1.0),
            q(c[0], 1e3),
            q(c[1], 1e3),
        ]
    };
    let mut seen: HashMap<[u32; 12], u32> = HashMap::with_capacity(m.positions.len());
    let mut remap = vec![0u32; m.positions.len()];
    let (mut positions, mut normals, mut uvs, mut tangents, mut colors) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for i in 0..m.positions.len() {
        let k = key(i);
        let idx = *seen.entry(k).or_insert_with(|| {
            positions.push(m.positions[i]);
            normals.push(m.normals[i]);
            uvs.push(m.uvs.get(i).copied().unwrap_or([0.0; 2]));
            tangents.push(m.tangents.get(i).copied().unwrap_or([1.0, 0.0, 0.0, 1.0]));
            colors.push(m.colors.get(i).copied().unwrap_or([1.0; 4]));
            (positions.len() - 1) as u32
        });
        remap[i] = idx;
    }
    for i in m.indices.iter_mut() {
        *i = remap[*i as usize];
    }
    m.positions = positions;
    m.normals = normals;
    m.uvs = uvs;
    m.tangents = tangents;
    m.colors = colors;
}

/// The XZ centre + Y range of a shape (the axis cylindrical/spherical UVs
/// wrap around).
fn center_of(shape: &Shape) -> [f32; 3] {
    let (min, max) = shape.bounds();
    [
        (min[0] + max[0]) / 2.0,
        (min[1] + max[1]) / 2.0,
        (min[2] + max[2]) / 2.0,
    ]
}

/// Project UVs and derive tangents. Vertices are split per triangle
/// (normals are copied, so shading stays smooth) — that lets each face own
/// exact texture coordinates, and lets cylindrical wrapping resolve its
/// seam per triangle instead of smearing a whole band.
///
/// Tangents come from the UV derivatives (Lengyel's construction), not from
/// a guessed axis: normal maps only line up when the tangent frame is the
/// one the UVs actually imply.
fn project_uvs(m: &mut MeshData, mode: UvMode, uv_scale: f32, center: [f32; 3]) {
    let n_idx = m.indices.len();
    let mut positions = Vec::with_capacity(n_idx);
    let mut normals = Vec::with_capacity(n_idx);
    let mut uvs = Vec::with_capacity(n_idx);
    let mut tangents = Vec::with_capacity(n_idx);
    let mut colors = Vec::with_capacity(n_idx);
    let mut indices = Vec::with_capacity(n_idx);

    for tri in m.indices.chunks(3) {
        let p: [[f32; 3]; 3] = [
            m.positions[tri[0] as usize],
            m.positions[tri[1] as usize],
            m.positions[tri[2] as usize],
        ];
        let e1 = sub(p[1], p[0]);
        let e2 = sub(p[2], p[0]);
        let face_n = cross(e1, e2);

        // --- texture coordinates for this triangle's three corners ---
        let mut uv3: [[f32; 2]; 3] = [[0.0; 2]; 3];
        match mode {
            UvMode::Cylindrical => {
                // u = angle about Y (0..1 around), v = height. The wrap seam
                // is fixed per triangle: if the corners straddle it, lift the
                // small angles by a full turn so the face stays continuous.
                let circumference = {
                    let r = p
                        .iter()
                        .map(|q| ((q[0] - center[0]).powi(2) + (q[2] - center[2]).powi(2)).sqrt())
                        .fold(0.0f32, f32::max);
                    (r * std::f32::consts::TAU).max(0.001)
                };
                let mut u: [f32; 3] = [0.0; 3];
                let mut radial: [f32; 3] = [0.0; 3];
                for i in 0..3 {
                    let (dx, dz) = (p[i][0] - center[0], p[i][2] - center[2]);
                    radial[i] = (dx * dx + dz * dz).sqrt();
                    u[i] = dz.atan2(dx) / std::f32::consts::TAU + 0.5; // 0..1
                }
                unwrap_angles(&mut u, radial);
                for i in 0..3 {
                    uv3[i] = [u[i] * circumference * uv_scale, p[i][1] * uv_scale];
                }
            }
            UvMode::Spherical => {
                let radius = p
                    .iter()
                    .map(|q| length(sub(*q, center)))
                    .fold(0.0f32, f32::max)
                    .max(0.001);
                let mut u: [f32; 3] = [0.0; 3];
                let mut v: [f32; 3] = [0.0; 3];
                let mut radial: [f32; 3] = [0.0; 3];
                for i in 0..3 {
                    let d = sub(p[i], center);
                    radial[i] = (d[0] * d[0] + d[2] * d[2]).sqrt();
                    u[i] = d[2].atan2(d[0]) / std::f32::consts::TAU + 0.5;
                    v[i] = (d[1] / length(d).max(1e-6)).clamp(-1.0, 1.0).asin()
                        / std::f32::consts::PI
                        + 0.5;
                }
                unwrap_angles(&mut u, radial);
                let circumference = radius * std::f32::consts::TAU;
                for i in 0..3 {
                    uv3[i] = [
                        u[i] * circumference * uv_scale,
                        v[i] * circumference * 0.5 * uv_scale,
                    ];
                }
            }
            _ => {
                // Box: project along the face's dominant axis.
                let ax = if face_n[0].abs() >= face_n[1].abs() && face_n[0].abs() >= face_n[2].abs()
                {
                    0
                } else if face_n[1].abs() >= face_n[2].abs() {
                    1
                } else {
                    2
                };
                let (ua, va) = match ax {
                    0 => (2, 1),
                    1 => (0, 2),
                    _ => (0, 1),
                };
                for i in 0..3 {
                    uv3[i] = [p[i][ua] * uv_scale, p[i][va] * uv_scale];
                }
            }
        }

        // --- tangent from the UV derivatives (per triangle, exact) ---
        let duv1 = [uv3[1][0] - uv3[0][0], uv3[1][1] - uv3[0][1]];
        let duv2 = [uv3[2][0] - uv3[0][0], uv3[2][1] - uv3[0][1]];
        let det = duv1[0] * duv2[1] - duv2[0] * duv1[1];
        let tangent3 = if det.abs() < 1e-12 {
            // Degenerate UVs: any perpendicular works better than a NaN.
            let n = normalize(face_n);
            let axis = if n[1].abs() < 0.9 {
                [0.0, 1.0, 0.0]
            } else {
                [1.0, 0.0, 0.0]
            };
            normalize(cross(axis, n))
        } else {
            let r = 1.0 / det;
            normalize([
                (e1[0] * duv2[1] - e2[0] * duv1[1]) * r,
                (e1[1] * duv2[1] - e2[1] * duv1[1]) * r,
                (e1[2] * duv2[1] - e2[2] * duv1[1]) * r,
            ])
        };
        // Bitangent handedness (glTF's tangent.w).
        let bitangent = if det.abs() < 1e-12 {
            cross(normalize(face_n), tangent3)
        } else {
            let r = 1.0 / det;
            [
                (e2[0] * duv1[0] - e1[0] * duv2[0]) * r,
                (e2[1] * duv1[0] - e1[1] * duv2[0]) * r,
                (e2[2] * duv1[0] - e1[2] * duv2[0]) * r,
            ]
        };

        for (i, &vi) in tri.iter().enumerate() {
            let vn = m.normals[vi as usize];
            // Gram-Schmidt the tangent against this vertex's smooth normal.
            let t = normalize(sub(tangent3, scale3(vn, dot(vn, tangent3))));
            let w = if dot(cross(vn, t), bitangent) < 0.0 {
                -1.0
            } else {
                1.0
            };
            indices.push(positions.len() as u32);
            positions.push(p[i]);
            normals.push(vn);
            uvs.push(uv3[i]);
            tangents.push([t[0], t[1], t[2], w]);
            colors.push(m.colors.get(vi as usize).copied().unwrap_or([1.0; 4]));
        }
    }
    m.positions = positions;
    m.normals = normals;
    m.uvs = uvs;
    m.tangents = tangents;
    m.colors = colors;
    m.indices = indices;
}

/// A box, exactly: 24 vertices (each face its own, for hard edges), 12
/// triangles, per-face UVs at the same scale the projections use.
fn exact_box(p: &infinite_manifest::shape::Prim, uv_scale: f32) -> MeshData {
    let s = p.size.unwrap_or([1.0; 3]);
    let (hx, hy, hz) = (s[0] / 2.0, s[1] / 2.0, s[2] / 2.0);
    // (normal, tangent, the two extents the face spans)
    let faces: [([f32; 3], [f32; 3], [[f32; 3]; 4]); 6] = [
        // +X
        (
            [1.0, 0.0, 0.0],
            [0.0, 0.0, -1.0],
            [[hx, -hy, hz], [hx, -hy, -hz], [hx, hy, -hz], [hx, hy, hz]],
        ),
        // −X
        (
            [-1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0],
            [
                [-hx, -hy, -hz],
                [-hx, -hy, hz],
                [-hx, hy, hz],
                [-hx, hy, -hz],
            ],
        ),
        // +Y
        (
            [0.0, 1.0, 0.0],
            [1.0, 0.0, 0.0],
            [[-hx, hy, hz], [hx, hy, hz], [hx, hy, -hz], [-hx, hy, -hz]],
        ),
        // −Y
        (
            [0.0, -1.0, 0.0],
            [1.0, 0.0, 0.0],
            [
                [-hx, -hy, -hz],
                [hx, -hy, -hz],
                [hx, -hy, hz],
                [-hx, -hy, hz],
            ],
        ),
        // +Z
        (
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 0.0],
            [[-hx, -hy, hz], [hx, -hy, hz], [hx, hy, hz], [-hx, hy, hz]],
        ),
        // −Z
        (
            [0.0, 0.0, -1.0],
            [-1.0, 0.0, 0.0],
            [
                [hx, -hy, -hz],
                [-hx, -hy, -hz],
                [-hx, hy, -hz],
                [hx, hy, -hz],
            ],
        ),
    ];
    let yaw = (-p.rot).to_radians();
    let (sin, cos) = (yaw.sin(), yaw.cos());
    let turn = |v: [f32; 3]| {
        if p.rot == 0.0 {
            v
        } else {
            [v[0] * cos + v[2] * sin, v[1], -v[0] * sin + v[2] * cos]
        }
    };
    let mut m = MeshData::default();
    for (n, t, corners) in faces {
        let base = m.positions.len() as u32;
        let (n, t) = (turn(n), turn(t));
        let bt = cross(n, t);
        for c in corners {
            let world = turn(c);
            m.positions
                .push([world[0] + p.at[0], world[1] + p.at[1], world[2] + p.at[2]]);
            m.normals.push(n);
            // Face-local UVs: project the corner onto the face's own axes.
            m.uvs.push([dot(c, t) * uv_scale, dot(c, bt) * uv_scale]);
            m.tangents.push([t[0], t[1], t[2], 1.0]);
            m.colors.push([1.0, 1.0, 1.0, 1.0]);
        }
        m.indices
            .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    m
}

/// Fix a triangle's angular coordinates: unwrap across the 0/1 seam, and —
/// when the corners still span most of the circle — collapse them onto the
/// corner furthest from the axis. That second case is a pole/axis triangle,
/// where longitude is meaningless; smearing the whole texture across it is
/// the classic sphere-seam artefact, so we simply don't.
fn unwrap_angles(u: &mut [f32; 3], radial: [f32; 3]) {
    let (umin, umax) = (
        u.iter().copied().fold(f32::MAX, f32::min),
        u.iter().copied().fold(f32::MIN, f32::max),
    );
    if umax - umin > 0.5 {
        for x in u.iter_mut() {
            if *x < 0.5 {
                *x += 1.0;
            }
        }
    }
    let (umin, umax) = (
        u.iter().copied().fold(f32::MAX, f32::min),
        u.iter().copied().fold(f32::MIN, f32::max),
    );
    if umax - umin > 0.5 {
        let best = (0..3)
            .max_by(|a, b| radial[*a].total_cmp(&radial[*b]))
            .unwrap_or(0);
        *u = [u[best]; 3];
    }
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
fn length(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}
fn scale3(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn normalize(a: [f32; 3]) -> [f32; 3] {
    let l = length(a).max(1e-9);
    [a[0] / l, a[1] / l, a[2] / l]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sphere(r: f32) -> Shape {
        serde_json::from_str(&format!(r#"{{ "prim": "sphere", "r": {r} }}"#)).unwrap()
    }

    #[test]
    fn a_sphere_meshes_at_its_radius_with_outward_ccw_winding() {
        let m = mesh(&sphere(1.0), Some(32));
        assert!(m.positions.len() > 500, "vertices: {}", m.positions.len());
        assert_eq!(m.indices.len() % 3, 0);
        // Every vertex sits on the surface (within a cell).
        for p in &m.positions {
            let r = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
            assert!((r - 1.0).abs() < 0.12, "vertex off the sphere: r={r}");
        }
        // Winding: each triangle's geometric normal points away from the centre
        // (CCW seen from outside — the repo's mesh convention).
        let mut outward = 0usize;
        for t in m.indices.chunks(3) {
            let (a, b, c) = (
                m.positions[t[0] as usize],
                m.positions[t[1] as usize],
                m.positions[t[2] as usize],
            );
            let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let n = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            let centroid = [
                (a[0] + b[0] + c[0]) / 3.0,
                (a[1] + b[1] + c[1]) / 3.0,
                (a[2] + b[2] + c[2]) / 3.0,
            ];
            if n[0] * centroid[0] + n[1] * centroid[1] + n[2] * centroid[2] > 0.0 {
                outward += 1;
            }
        }
        let frac = outward as f32 / (m.indices.len() / 3) as f32;
        assert!(frac > 0.99, "outward-wound fraction: {frac}");
        // Normals agree with positions on a sphere.
        for (p, n) in m.positions.iter().zip(&m.normals) {
            let dot = p[0] * n[0] + p[1] * n[1] + p[2] * n[2];
            assert!(dot > 0.9, "normal ≈ radial on a sphere (dot {dot})");
        }
    }

    #[test]
    fn cut_carves_and_blend_melds() {
        // A box with a sphere carved out of its top face.
        let carved: Shape = serde_json::from_str(
            r#"{ "op": "cut", "parts": [
                 { "prim": "box", "size": [2, 1, 2] },
                 { "prim": "sphere", "r": 0.7, "at": [0, 0.5, 0] }
               ] }"#,
        )
        .unwrap();
        assert!(eval(&carved, [0.0, 0.45, 0.0]) > 0.0, "the bowl is hollow");
        assert!(eval(&carved, [0.8, 0.0, 0.8]) < 0.0, "the corners remain");
        let m = mesh(&carved, Some(32));
        assert!(!m.indices.is_empty());

        // Blend: midpoint between two spheres is INSIDE the meld but OUTSIDE
        // a plain union.
        let mk = |op: &str| -> Shape {
            serde_json::from_str(&format!(
                r#"{{ "op": "{op}", "k": 0.4, "parts": [
                     {{ "prim": "sphere", "r": 0.5, "at": [-0.6, 0, 0] }},
                     {{ "prim": "sphere", "r": 0.5, "at": [0.6, 0, 0] }}
                   ] }}"#
            ))
            .unwrap()
        };
        assert!(eval(&mk("union"), [0.0, 0.0, 0.0]) > 0.0);
        assert!(eval(&mk("blend"), [0.0, 0.0, 0.0]) < eval(&mk("union"), [0.0, 0.0, 0.0]));
    }

    #[test]
    fn lathe_revolves_a_profile() {
        // A goblet profile: solid at the stem radius, hollow beyond the rim.
        let vase: Shape = serde_json::from_str(
            r#"{ "lathe": [[0, 0], [0.5, 0], [0.62, 0.2], [0.4, 0.9], [0.55, 1.3], [0, 1.3]] }"#,
        )
        .unwrap();
        assert!(eval(&vase, [0.2, 0.1, 0.0]) < 0.0, "inside the base");
        assert!(eval(&vase, [1.0, 0.5, 0.0]) > 0.0, "outside the wall");
        // Revolution symmetry: same distance at same radius, any angle.
        let d1 = eval(&vase, [0.6, 0.5, 0.0]);
        let d2 = eval(&vase, [0.0, 0.5, 0.6]);
        assert!((d1 - d2).abs() < 1e-4, "{d1} vs {d2}");
        let m = mesh(&vase, Some(40));
        assert!(m.positions.len() > 300);
    }

    #[test]
    fn uv_frames_are_pbr_correct_in_every_projection() {
        // Normal mapping is only as good as the tangent frame: every tangent
        // must be unit, perpendicular to its vertex normal, and consistent
        // with the UVs it was derived from. Checked in all three modes.
        let vase: Shape = serde_json::from_str(
            r#"{ "lathe": [[0,0],[0.5,0],[0.62,0.4],[0.35,1.0],[0.5,1.4],[0,1.4]] }"#,
        )
        .unwrap();
        for (shape, mode) in [
            (sphere(1.0), UvMode::Spherical),
            (vase.clone(), UvMode::Cylindrical),
            (sphere(1.0), UvMode::Box),
        ] {
            let m = mesh_with(
                &shape,
                MeshOptions {
                    resolution: Some(28),
                    uv: mode,
                    uv_scale: 0.5,
                },
            );
            assert_eq!(m.uvs.len(), m.positions.len());
            assert_eq!(m.tangents.len(), m.positions.len());
            for i in 0..m.positions.len() {
                let t = m.tangents[i];
                let n = m.normals[i];
                let len = (t[0] * t[0] + t[1] * t[1] + t[2] * t[2]).sqrt();
                assert!(
                    (len - 1.0).abs() < 1e-3,
                    "{mode:?}: unit tangent (len {len})"
                );
                let d = t[0] * n[0] + t[1] * n[1] + t[2] * n[2];
                assert!(d.abs() < 1e-3, "{mode:?}: tangent ⊥ normal (dot {d})");
                assert!(t[3] == 1.0 || t[3] == -1.0, "{mode:?}: handedness is ±1");
                assert!(
                    m.uvs[i].iter().all(|c| c.is_finite()),
                    "{mode:?}: finite UVs"
                );
            }
            // No triangle may straddle the wrap seam (the smear artefact):
            // after unwrapping, a face covers at most half a wrap — anything
            // wider would be running the texture backwards across the seam.
            let wrap = m.uvs.iter().map(|uv| uv[0]).fold(f32::MIN, f32::max)
                - m.uvs.iter().map(|uv| uv[0]).fold(f32::MAX, f32::min);
            for tri in m.indices.chunks(3) {
                let us: Vec<f32> = tri.iter().map(|&i| m.uvs[i as usize][0]).collect();
                let span = us.iter().copied().fold(f32::MIN, f32::max)
                    - us.iter().copied().fold(f32::MAX, f32::min);
                assert!(
                    span <= wrap * 0.5 + 0.05,
                    "{mode:?}: no seam-straddling triangle (span {span} of wrap {wrap})"
                );
            }
        }
        // Auto picks the right projection without being told.
        assert_eq!(UvMode::Auto.resolve(&vase), UvMode::Cylindrical);
        assert_eq!(UvMode::Auto.resolve(&sphere(1.0)), UvMode::Spherical);
    }

    #[test]
    fn a_plain_box_meshes_exactly() {
        let b: Shape =
            serde_json::from_str(r#"{ "prim": "box", "size": [2, 1, 3], "at": [0, 0.5, 0] }"#)
                .unwrap();
        let m = mesh(&b, Some(40));
        assert_eq!(
            m.indices.len() / 3,
            12,
            "twelve triangles, not two thousand"
        );
        assert_eq!(
            m.positions.len(),
            24,
            "hard edges: a vertex per face corner"
        );
        // Exactly the declared size, exactly where it was put.
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for p in &m.positions {
            for c in 0..3 {
                lo[c] = lo[c].min(p[c]);
                hi[c] = hi[c].max(p[c]);
            }
        }
        assert!((hi[0] - lo[0] - 2.0).abs() < 1e-5 && (hi[1] - lo[1] - 1.0).abs() < 1e-5);
        assert!(
            (lo[1] - 0.0).abs() < 1e-5,
            "sits where it was placed: {lo:?}"
        );
        // Outward winding, and normals agreeing with the faces.
        for tri in m.indices.chunks(3) {
            let (a, b2, c) = (
                m.positions[tri[0] as usize],
                m.positions[tri[1] as usize],
                m.positions[tri[2] as usize],
            );
            let u = sub(b2, a);
            let v = sub(c, a);
            let n = cross(u, v);
            let vn = m.normals[tri[0] as usize];
            assert!(dot(n, vn) > 0.0, "face winding matches its normal");
        }
        // A rounded box still marches (rounding is a curve, not a box).
        let r: Shape =
            serde_json::from_str(r#"{ "prim": "box", "size": [2, 1, 3], "rounded": 0.2 }"#)
                .unwrap();
        assert!(mesh(&r, Some(24)).indices.len() / 3 > 12);
    }

    #[test]
    fn welding_shrinks_the_mesh_without_changing_the_surface() {
        let m = mesh(&sphere(1.0), Some(32));
        // Triangles are untouched; vertices are far fewer than 3-per-face.
        assert_eq!(m.indices.len() % 3, 0);
        // Corners that agree merge; the ones that differ (per-face UVs and
        // tangents on a smooth surface) must stay split, so the win is real
        // but partial — measure it honestly rather than wish for 3×.
        assert!(
            m.positions.len() < (m.indices.len() as f32 * 0.85) as usize,
            "welded: {} verts for {} indices",
            m.positions.len(),
            m.indices.len()
        );
        // Every index still addresses a vertex, and every parallel array agrees.
        assert!(m.indices.iter().all(|i| (*i as usize) < m.positions.len()));
        for arr_len in [
            m.normals.len(),
            m.uvs.len(),
            m.tangents.len(),
            m.colors.len(),
        ] {
            assert_eq!(arr_len, m.positions.len());
        }
        // The surface is unchanged: every vertex still sits on the sphere.
        for p in &m.positions {
            let r = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
            assert!((r - 1.0).abs() < 0.12, "still a sphere: r={r}");
        }
    }

    #[test]
    fn glb_export_roundtrips_through_the_engine_loader() {
        // Carve → bake → export → reload with the SAME reader the browser
        // uses for any glTF asset. The whole "creations leave the editor"
        // promise, in one test.
        let carved: Shape = serde_json::from_str(
            r#"{ "op": "blend", "k": 0.3, "parts": [
                 { "prim": "sphere", "r": 0.6, "at": [0, 0.5, 0] },
                 { "prim": "box", "size": [1.4, 0.5, 1.4] }
               ] }"#,
        )
        .unwrap();
        let m = mesh(&carved, Some(24));
        let recipe: infinite_manifest::texture::TextureRecipe = serde_json::from_str(
            r#"{ "kind": "voronoi", "colors": [[0.4,0.4,0.42],[0.6,0.58,0.55]],
                 "height": 0.5, "ao": 0.4, "size": 64 }"#,
        )
        .unwrap();
        let baked = crate::texture::bake(&recipe);
        let glb = crate::gltf::write_glb(&m, Some(&baked), "test-carving").unwrap();
        assert_eq!(&glb[0..4], b"glTF");

        let dir = std::env::temp_dir().join(format!("chisel-glb-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("carving.glb");
        std::fs::write(&path, &glb).unwrap();
        // Read it back with a **third-party** glTF implementation rather than
        // our own loader. A round trip through the same code that wrote it
        // proves only that we are self-consistent; the claim worth testing is
        // that anyone's reader can open what Chisel exports.
        // `::gltf` — the crate, not our own `gltf` module of the same name,
        // which `use super::*` would otherwise shadow it with.
        let (doc, buffers, _) =
            ::gltf::import(&path).expect("a standard glTF reader opens our GLB");
        let prim = doc
            .meshes()
            .flat_map(|me| me.primitives())
            .next()
            .expect("a primitive");
        let reader = prim.reader(|b| Some(&buffers[b.index()]));
        let positions: Vec<[f32; 3]> = reader.read_positions().expect("positions").collect();
        assert_eq!(positions.len(), m.positions.len(), "vertices survive");
        let indices: Vec<u32> = reader.read_indices().expect("indices").into_u32().collect();
        assert_eq!(indices.len(), m.indices.len(), "indices survive");
        assert!(reader.read_tex_coords(0).is_some(), "UVs survive");
        assert!(reader.read_tangents().is_some(), "tangents survive");
        // The PBR set arrives as embedded images, not as a promise in the JSON.
        assert!(doc.images().len() >= 3, "albedo, normal and ORM are all in the file");
        let mat = prim.material().pbr_metallic_roughness();
        assert!(mat.base_color_texture().is_some(), "albedo is bound to the material");
        assert!(
            mat.metallic_roughness_texture().is_some(),
            "metallic-roughness is bound to the material"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolution_is_clamped() {
        let m = mesh(&sphere(0.5), Some(100_000));
        assert!(
            m.positions.len() < 400_000,
            "cap held: {}",
            m.positions.len()
        );
    }
}
