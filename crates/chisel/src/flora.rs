//! Flora — what grows where, and why.
//!
//! Placement is the whole game here. The old scatter drew a uniform random
//! position per blade, and **nothing in nature is distributed that way**:
//! white noise clumps and gaps at every scale at once, which the eye reads
//! instantly as static rather than as a meadow. Two changes fix almost all of
//! it, and neither is a rendering change:
//!
//! 1. **Blue noise.** Candidates are rejected if they land within a species'
//!    spacing of an accepted neighbour, so plants never interpenetrate and
//!    never grid up. This is what a lawn actually looks like from above.
//! 2. **Clustering.** A low-frequency mask over the density field, so trees
//!    grow in stands with clearings between them instead of an even sprinkle.
//!    Forests have edges; the mask is where edges come from.
//!
//! Everything else — which species, how big, how dense — is read from the
//! [`Field`](crate::terrain::Field) the terrain simulation already produced,
//! so a spruce line stops at altitude because the *temperature* stopped it,
//! not because someone drew a line.

use crate::terrain::{value_noise_pub as noise, Field, BIOMES, B_WATER};

/// What kind of thing this is, which decides how a renderer draws it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Form {
    /// Instanced blades, dense and near the camera only.
    Grass,
    /// Small scattered colour — reads at close range, invisible beyond it.
    Flower,
    /// Waist-high mass. Cheap, and it hides the join between grass and tree.
    Shrub,
    /// The silhouette layer: what a landscape reads as from a distance.
    Tree,
}

/// One kind of plant, and the conditions it tolerates.
///
/// The tolerances are the interesting part: they are read against the *same*
/// climate fields the biome weights came from, so a species cannot appear
/// somewhere its own climate forbids even if an author asks for it.
#[derive(Debug, Clone)]
pub struct Species {
    pub name: &'static str,
    pub form: Form,
    /// Metres between neighbours of this species — the blue-noise radius.
    pub spacing: f32,
    /// Metres, min..max. Individuals draw uniformly between them.
    pub height: (f32, f32),
    /// Per-biome affinity, indexed like [`BIOMES`]. Multiplied into density.
    pub affinity: [f32; 8],
    /// Steepest ground it will hold on to, 0..1.
    pub max_slope: f32,
    /// Wettest and driest it tolerates, 0..1.
    pub moisture: (f32, f32),
    /// Coldest and warmest, in the field's temperature units.
    pub temperature: (f32, f32),
    /// How tightly this species gathers, 0..1. At 0 it spreads evenly over
    /// everything it tolerates; at 1 it grows in tight stands with real
    /// clearings between them. Forests have edges -- this is where the edge
    /// comes from, and it is per-species because a spruce wood and a meadow
    /// are not the same shape of thing.
    pub clumping: f32,
    /// Base albedo. Individuals vary around it.
    pub color: [f32; 3],
}

/// A placed individual. Renderer-agnostic on purpose: both backends build
/// their own instance structs from this, and neither owns the placement math.
#[derive(Debug, Clone, Copy)]
pub struct Plant {
    /// Tile-local metres. Y is the ground the mesh draws, so nothing floats.
    pub position: [f32; 3],
    pub yaw: f32,
    /// Metres tall.
    pub height: f32,
    /// Lean, radians. Plants on a slope do not stand plumb.
    pub tilt: f32,
    pub tilt_dir: f32,
    pub color: [f32; 3],
    /// Index into the species table this was scattered from.
    pub species: u16,
}

/// The default table. Deliberately small — a handful of well-chosen species
/// covers a planet, and each one earns its place by being climatically
/// distinct rather than visually different.
pub fn default_species() -> Vec<Species> {
    //           water beach desert grass shrub forest taiga alpine
    vec![
        Species {
            name: "meadow grass",
            form: Form::Grass,
            spacing: 0.17,
            height: (0.22, 0.55),
            affinity: [0.0, 0.15, 0.05, 1.0, 0.7, 0.5, 0.35, 0.05],
            max_slope: 0.55,
            moisture: (0.18, 1.0),
            temperature: (-6.0, 40.0),
            clumping: 0.30,
            color: [0.34, 0.46, 0.16],
        },
        Species {
            name: "dune grass",
            form: Form::Grass,
            spacing: 0.34,
            height: (0.25, 0.6),
            affinity: [0.0, 1.0, 0.55, 0.1, 0.25, 0.0, 0.0, 0.0],
            max_slope: 0.5,
            moisture: (0.0, 0.45),
            temperature: (2.0, 45.0),
            clumping: 0.55,
            color: [0.62, 0.58, 0.34],
        },
        Species {
            name: "wildflower",
            form: Form::Flower,
            spacing: 0.9,
            height: (0.15, 0.4),
            affinity: [0.0, 0.05, 0.0, 1.0, 0.5, 0.3, 0.15, 0.05],
            max_slope: 0.45,
            moisture: (0.3, 0.95),
            temperature: (0.0, 32.0),
            clumping: 0.88,
            color: [0.85, 0.78, 0.35],
        },
        Species {
            name: "heather",
            form: Form::Shrub,
            spacing: 1.6,
            height: (0.4, 0.9),
            affinity: [0.0, 0.1, 0.25, 0.35, 1.0, 0.4, 0.5, 0.25],
            max_slope: 0.62,
            moisture: (0.1, 0.8),
            temperature: (-10.0, 28.0),
            clumping: 0.62,
            color: [0.38, 0.34, 0.24],
        },
        Species {
            name: "broadleaf",
            form: Form::Tree,
            spacing: 7.0,
            height: (9.0, 19.0),
            affinity: [0.0, 0.0, 0.0, 0.25, 0.3, 1.0, 0.15, 0.0],
            max_slope: 0.5,
            moisture: (0.45, 1.0),
            temperature: (4.0, 32.0),
            clumping: 0.72,
            color: [0.20, 0.33, 0.14],
        },
        Species {
            name: "conifer",
            form: Form::Tree,
            spacing: 5.5,
            height: (11.0, 26.0),
            affinity: [0.0, 0.0, 0.0, 0.1, 0.25, 0.55, 1.0, 0.2],
            max_slope: 0.62,
            moisture: (0.3, 1.0),
            // The tree line: conifers simply stop being tolerated up there,
            // and the mountain gets its bare shoulder without anyone drawing
            // a contour on it.
            temperature: (-9.0, 16.0),
            clumping: 0.86,
            color: [0.15, 0.26, 0.17],
        },
        Species {
            name: "desert scrub",
            form: Form::Shrub,
            spacing: 3.2,
            height: (0.5, 1.4),
            affinity: [0.0, 0.2, 1.0, 0.15, 0.4, 0.0, 0.0, 0.05],
            max_slope: 0.55,
            moisture: (0.0, 0.32),
            temperature: (6.0, 48.0),
            clumping: 0.70,
            color: [0.44, 0.42, 0.26],
        },
    ]
}

#[inline]
fn smoothstep(lo: f32, hi: f32, x: f32) -> f32 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[inline]
fn band(x: f32, lo: f32, hi: f32, feather: f32) -> f32 {
    let a = ((x - lo) / feather).clamp(0.0, 1.0);
    let b = ((hi - x) / feather).clamp(0.0, 1.0);
    a * b
}

/// How much of `sp` this cell will support, 0..1.
fn density_at(f: &Field, sp: &Species, x: usize, y: usize) -> f32 {
    let i = f.idx(x, y);
    let base = i * BIOMES.len();
    let mut aff = 0.0;
    for b in 0..BIOMES.len() {
        aff += f.biome[base + b] * sp.affinity[b];
    }
    if aff <= 0.001 {
        return 0.0;
    }
    // Nothing grows in open water, and the shoulder of a cliff is not soil.
    let wet = f.biome[base + B_WATER];
    let slope = 1.0 - ((f.slope[i] - sp.max_slope) / 0.15).clamp(0.0, 1.0);
    let moist = band(f.moisture[i], sp.moisture.0, sp.moisture.1, 0.12);
    let temp = band(f.temperature[i], sp.temperature.0, sp.temperature.1, 4.0);
    aff * slope * moist * temp * (1.0 - wet)
}

/// Scatter one species across a tile.
///
/// `cover` scales the whole result (the manifest's `cover.trees` and friends),
/// and `seed` keeps two species from landing on the same points.
/// The budget caps the instance count. Ground cover at its natural spacing
/// runs to millions of blades over a whole tile, so callers scatter grass
/// over a small radius near the traveler and pass a budget they can draw.
pub fn scatter(
    f: &Field,
    sp: &Species,
    species_index: u16,
    cover: f32,
    seed: u32,
    // Tile-local metres [x0, z0, x1, z1] to fill, or None for the whole tile.
    // Ground cover is not a tile-sized thing: a blade of grass is invisible at
    // 200 m, so grass is scattered over a small region around the traveler
    // while trees, which read from anywhere, cover the whole tile.
    area: Option<[f32; 4]>,
    budget: usize,
) -> Vec<Plant> {
    if cover <= 0.0 {
        return Vec::new();
    }
    let tile = (f.size - 1) as f32 * f.cell_size;
    let a = area.unwrap_or([0.0, 0.0, tile, tile]);
    let (x0, z0) = (a[0].max(0.0), a[1].max(0.0));
    let (x1, z1) = (a[2].min(tile), a[3].min(tile));
    if x1 <= x0 || z1 <= z0 {
        return Vec::new();
    }
    let extent = (x1 - x0).max(z1 - z0);
    // One candidate per spacing-sized cell: a jittered grid is already close
    // to blue noise, and the rejection pass below makes it honest.
    let cells = ((extent / sp.spacing).ceil() as usize).clamp(1, 1024);
    let cell = extent / cells as f32;
    // The rejection search must reach at least one spacing in every
    // direction. Cells are normally spacing-wide, so that is the 3x3
    // neighbourhood -- but the clamp above can make them narrower on a
    // large tile, and then a fixed 3x3 search would silently miss
    // conflicts and the blue-noise guarantee would quietly stop holding.
    // Inside a stand, plants crowd; toward its edge they thin out. Spacing
    // therefore stretches with the cluster mask, and the search has to reach
    // the WIDEST spacing any candidate might ask for.
    let stretch = 1.0 + 1.9 * sp.clumping;
    let reach = ((sp.spacing * stretch / cell).ceil() as usize).max(1);

    let mut accepted: Vec<Plant> = Vec::new();
    // Spatial hash over the same grid, so the rejection test only ever looks
    // at the nine cells that could possibly hold a conflict.
    let mut grid: Vec<i32> = vec![-1; cells * cells];

    let mut h = seed.wrapping_mul(0x9E37_79B9).wrapping_add(1);
    let mut rand = move || {
        h ^= h << 13;
        h ^= h >> 17;
        h ^= h << 5;
        h as f32 / u32::MAX as f32
    };

    for cy in 0..cells {
        if accepted.len() >= budget {
            break;
        }
        for cx in 0..cells {
            // Jitter inside the cell.
            let px = x0 + (cx as f32 + rand()) * cell;
            let pz = z0 + (cy as f32 + rand()) * cell;
            if px > x1 || pz > z1 {
                continue;
            }
            let fx = px / f.cell_size;
            let fz = pz / f.cell_size;
            let gx = (fx.round() as usize).min(f.size - 1);
            let gz = (fz.round() as usize).min(f.size - 1);

            let mut d = density_at(f, sp, gx, gz) * cover;
            if d <= 0.0 {
                continue;
            }
            // Clustering: stands and clearings. Without this a forest is an
            // even sprinkle of trees, which reads as an orchard.
            // Two octaves: the low one decides where the wood is, the high
            // one breaks its interior up so it is not a solid disc.
            // The seed goes to the hash, never to the coordinate: it already
            // selects a different lattice, and adding a large seed to x would
            // push the sample past the far end of the lattice entirely.
            let c1 = noise(px * 0.0075, pz * 0.0075, seed ^ 0x51ed) * 0.5 + 0.5;
            let c2 = noise(px * 0.026, pz * 0.026, seed ^ 0x7a31) * 0.5 + 0.5;
            // Two octaves of value noise land in a narrow band around the
            // middle -- averaging them narrows it further -- so a threshold at
            // 0.5 passed almost everything and the clearings never appeared.
            // Stretching the contrast about the midpoint is what gives the
            // mask something to bite on.
            let cluster = (0.5 + (c1 * 0.72 + c2 * 0.28 - 0.5) * 1.9).clamp(0.0, 1.0);
            // A soft THRESHOLD, not a multiply. The width is what makes an
            // edge an edge: a tightly-clumping species transitions from
            // full cover to none over a few metres, a loose one fades.
            let w = 0.30 - 0.23 * sp.clumping;
            // Stragglers still grow between stands -- a hard zero outside
            // reads as a stencil, and real clearings are never quite empty.
            let floor = 0.10 * (1.0 - sp.clumping);
            let mask = floor + (1.0 - floor) * smoothstep(0.5 - w, 0.5 + w, cluster);
            d *= mask;
            // Thinning by probability alone barely shows, because the spacing
            // rule already caps how many plants a dense patch can hold: making
            // a full patch fuller changes nothing. Stretching the spacing in
            // the thin places is what actually opens a clearing.
            let spacing = sp.spacing * (1.0 + (1.0 - mask) * 1.9 * sp.clumping);
            if rand() > d {
                continue;
            }

            // Blue noise: reject anything too close to an accepted neighbour.
            let mut clash = false;
            'search: for ny in cy.saturating_sub(reach)..=(cy + reach).min(cells - 1) {
                for nx in cx.saturating_sub(reach)..=(cx + reach).min(cells - 1) {
                    let k = grid[ny * cells + nx];
                    if k < 0 {
                        continue;
                    }
                    let p = accepted[k as usize].position;
                    if (p[0] - px).hypot(p[2] - pz) < spacing {
                        clash = true;
                        break 'search;
                    }
                }
            }
            if clash {
                continue;
            }

            let ground = f.height_at(fx, fz);
            let i = f.idx(gx, gz);
            let height = sp.height.0 + (sp.height.1 - sp.height.0) * rand();
            // Individuals vary, and they lean off a slope rather than standing
            // plumb on a hillside like fence posts.
            let v = 0.86 + rand() * 0.28;
            let tint = 0.94 + rand() * 0.12;
            let idx = accepted.len() as i32;
            grid[cy * cells + cx] = idx;
            accepted.push(Plant {
                position: [px, ground, pz],
                yaw: rand() * std::f32::consts::TAU,
                height: height * v,
                // Plants lean off a slope, but only so far -- an uncapped
                // lean on steep ground lays them out flat on their sides.
                tilt: (f.slope[i] * 0.35).min(0.30) + rand() * 0.05,
                tilt_dir: rand() * std::f32::consts::TAU,
                color: [
                    sp.color[0] * tint,
                    sp.color[1] * tint,
                    sp.color[2] * (2.0 - tint),
                ],
                species: species_index,
            });
        }
    }
    accepted
}

/// Scatter every species of a given form over a tile.
pub fn scatter_form(
    f: &Field,
    table: &[Species],
    form: Form,
    cover: f32,
    seed: u32,
    area: Option<[f32; 4]>,
    budget: usize,
) -> Vec<Plant> {
    let mut out = Vec::new();
    let of_form = table.iter().filter(|s| s.form == form).count().max(1);
    for (i, sp) in table.iter().enumerate() {
        if sp.form != form {
            continue;
        }
        // Split the budget so one species cannot starve the rest.
        out.extend(scatter(
            f,
            sp,
            i as u16,
            cover,
            seed.wrapping_add(i as u32 * 7919),
            area,
            budget / of_form,
        ));
    }
    out
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------
//
// Plants are built as ordinary meshes and merged into a few big batches. That
// is deliberate: it needs no instancing path, no new pipeline and no new vertex
// attribute, so flora arrives through the same door as every other mesh in the
// Thread and works in both renderers on day one.
//
// Wind rides in the vertex colour's ALPHA, which the PBR G-buffer does not
// otherwise read. The convention is inverted on purpose -- **alpha 1.0 means
// rigid** -- so every mesh that already exists, all of which are opaque white
// alpha, keeps standing perfectly still. A plant writes `1.0 - sway`, near 1 at
// the root and lower toward the tip, and only things that opt in ever move.

/// Accumulates merged plant geometry.
#[derive(Default)]
struct Builder {
    m: crate::MeshData,
}

impl Builder {
    /// Appends one vertex in plant-local space and returns nothing; callers
    /// index by the base they captured before pushing.
    fn v(&mut self, p: [f32; 3], n_in: [f32; 3], uv: [f32; 2], c: [f32; 3], sway: f32) {
        // Normalize here, at the one place every vertex passes through, rather
        // than trusting each generator to hand in a unit normal. A blade's
        // light-facing normal is authored by eye and was not unit length, and
        // the Gram-Schmidt below only cancels the parallel component when it
        // is -- so the tangent came out very slightly non-perpendicular.
        let nl = (n_in[0] * n_in[0] + n_in[1] * n_in[1] + n_in[2] * n_in[2])
            .sqrt()
            .max(1e-6);
        let n = [n_in[0] / nl, n_in[1] / nl, n_in[2] / nl];
        self.m.positions.push(p);
        self.m.normals.push(n);
        self.m.uvs.push(uv);
        // A tangent must be perpendicular to its own normal. A constant
        // [1,0,0] is fine for ground, whose normals point up, but a plant has
        // normals in every direction and the ones along X would leave the
        // renderer orthogonalizing a zero vector.
        let alt = if n[0].abs() > 0.9 {
            [0.0, 0.0, 1.0]
        } else {
            [1.0, 0.0, 0.0]
        };
        let d = alt[0] * n[0] + alt[1] * n[1] + alt[2] * n[2];
        let t = [alt[0] - n[0] * d, alt[1] - n[1] * d, alt[2] - n[2] * d];
        let l = (t[0] * t[0] + t[1] * t[1] + t[2] * t[2]).sqrt().max(1e-6);
        self.m.tangents.push([t[0] / l, t[1] / l, t[2] / l, 1.0]);
        self.m.colors.push([c[0], c[1], c[2], 1.0 - sway.clamp(0.0, 1.0)]);
    }
    fn tri(&mut self, a: u32, b: u32, c: u32) {
        self.m.indices.extend_from_slice(&[a, b, c]);
    }
    /// A quad, emitted on both faces. Grass with a missing back face goes black
    /// from half the compass, which is far more obvious than the extra tris.
    fn quad2(&mut self, p: [[f32; 3]; 4], n: [f32; 3], c: [f32; 3], sway: [f32; 4]) {
        let b = self.m.positions.len() as u32;
        let uv = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        for i in 0..4 {
            self.v(p[i], n, uv[i], c, sway[i]);
        }
        self.tri(b, b + 1, b + 2);
        self.tri(b, b + 2, b + 3);
        let b2 = self.m.positions.len() as u32;
        let back = [-n[0], -n[1], -n[2]];
        for i in 0..4 {
            self.v(p[i], back, uv[i], c, sway[i]);
        }
        self.tri(b2, b2 + 2, b2 + 1);
        self.tri(b2, b2 + 3, b2 + 2);
    }
}

#[inline]
fn rot(p: [f32; 3], yaw: f32) -> [f32; 3] {
    let (s, c) = yaw.sin_cos();
    [p[0] * c - p[2] * s, p[1], p[0] * s + p[2] * c]
}

/// A tapered, bent blade: two segments, so it curves rather than shearing.
fn blade(b: &mut Builder, pl: &Plant, width: f32, bend: f32, sway_scale: f32) {
    let h = pl.height;
    let (yaw, tip) = (pl.yaw, bend * h);
    let (bs, bc) = pl.tilt_dir.sin_cos();
    // Segment tops, leaning progressively into the bend direction.
    let mids = [
        [bc * tip * 0.22, h * 0.55, bs * tip * 0.22],
        [bc * tip, h, bs * tip],
    ];
    let widths = [width, width * 0.55, 0.0];
    let mut prev = [0.0f32, 0.0, 0.0];
    let mut prev_w = widths[0];
    let mut prev_s = 0.0f32;
    for (i, top) in mids.iter().enumerate() {
        let w = widths[i + 1];
        // Sway grows with the SQUARE of height: a blade pivots at its root, so
        // the tip travels far and the base barely moves.
        let s = ((top[1] / h).powi(2)).clamp(0.0, 1.0) * sway_scale;
        let q = [
            rot([-prev_w * 0.5, prev[1], prev[2]], yaw),
            rot([prev_w * 0.5, prev[1], prev[2]], yaw),
            rot([w * 0.5 + top[0], top[1], top[2]], yaw),
            rot([-w * 0.5 + top[0], top[1], top[2]], yaw),
        ];
        let q = [
            [q[0][0] + prev[0], q[0][1], q[0][2]],
            [q[1][0] + prev[0], q[1][1], q[1][2]],
            q[2],
            q[3],
        ];
        // Blades face the light more than they face the camera: an upward-biased
        // normal is what keeps a lawn from reading as a field of dark slivers.
        let n = rot([0.35, 0.75, 0.0], yaw);
        b.quad2(q, n, pl.color, [prev_s, prev_s, s, s]);
        prev = *top;
        prev_w = w;
        prev_s = s;
    }
}

/// A closed cone, used for conifer skirts and flower heads.
fn cone(b: &mut Builder, c: [f32; 3], r: f32, h: f32, sides: usize, col: [f32; 3], sway: f32) {
    let base = b.m.positions.len() as u32;
    let apex = [c[0], c[1] + h, c[2]];
    b.v(apex, [0.0, 1.0, 0.0], [0.5, 1.0], col, sway);
    for i in 0..sides {
        let a = i as f32 / sides as f32 * std::f32::consts::TAU;
        let (s, co) = a.sin_cos();
        let p = [c[0] + co * r, c[1], c[2] + s * r];
        let n = {
            let l = (h * h + r * r).sqrt().max(1e-5);
            [co * h / l, r / l, s * h / l]
        };
        b.v(p, n, [i as f32 / sides as f32, 0.0], col, sway * 0.55);
    }
    for i in 0..sides {
        let a = base + 1 + i as u32;
        let c2 = base + 1 + ((i + 1) % sides) as u32;
        b.tri(base, c2, a);
    }
}

// A mesh generator takes the parameters the shape has; splitting them into a
// struct to satisfy a lint would only move the same nine numbers somewhere else.
#[allow(clippy::too_many_arguments)]
/// A squashed low-poly blob — the broadleaf canopy and the body of a shrub.
fn blob(b: &mut Builder, c: [f32; 3], r: [f32; 3], seg: usize, rings: usize, col: [f32; 3], sway: f32, seed: u32) {
    let base = b.m.positions.len() as u32;
    let mut h = seed | 1;
    let mut rnd = move || {
        h ^= h << 13;
        h ^= h >> 17;
        h ^= h << 5;
        h as f32 / u32::MAX as f32
    };
    for ring in 0..=rings {
        let v = ring as f32 / rings as f32;
        let phi = v * std::f32::consts::PI;
        let (sp, cp) = phi.sin_cos();
        for s in 0..seg {
            let u = s as f32 / seg as f32;
            let th = u * std::f32::consts::TAU;
            let (st, ct) = th.sin_cos();
            // Per-vertex wobble: a canopy is not an ellipsoid, and the silhouette
            // is the only part of a distant tree anyone actually sees.
            let k = 0.82 + rnd() * 0.36;
            let d = [sp * ct * k, cp * k, sp * st * k];
            b.v(
                [c[0] + d[0] * r[0], c[1] + d[1] * r[1], c[2] + d[2] * r[2]],
                {
                    let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-5);
                    [d[0] / l, d[1] / l, d[2] / l]
                },
                [u, v],
                col,
                sway,
            );
        }
    }
    for ring in 0..rings {
        for s in 0..seg {
            let a = base + (ring * seg + s) as u32;
            let bq = base + (ring * seg + (s + 1) % seg) as u32;
            let c2 = base + ((ring + 1) * seg + s) as u32;
            let d = base + ((ring + 1) * seg + (s + 1) % seg) as u32;
            b.tri(a, bq, c2);
            b.tri(bq, d, c2);
        }
    }
}

/// A tapered trunk. Never sways: a tree bends in its canopy, not its bole.
fn trunk(b: &mut Builder, h: f32, r0: f32, r1: f32, sides: usize, col: [f32; 3]) {
    let base = b.m.positions.len() as u32;
    for ring in 0..2 {
        let (y, r) = if ring == 0 { (0.0, r0) } else { (h, r1) };
        for i in 0..sides {
            let a = i as f32 / sides as f32 * std::f32::consts::TAU;
            let (s, c) = a.sin_cos();
            b.v([c * r, y, s * r], [c, 0.0, s], [i as f32 / sides as f32, y / h], col, 0.0);
        }
    }
    for i in 0..sides {
        let a = base + i as u32;
        let bq = base + ((i + 1) % sides) as u32;
        let c = base + (sides + i) as u32;
        let d = base + (sides + (i + 1) % sides) as u32;
        b.tri(a, c, bq);
        b.tri(bq, c, d);
    }
}

/// Geometric detail, chosen by distance. The ladder is the whole reason a
/// forest can be drawn at all: the near ring gets every blade, the far ring
/// gets silhouettes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Detail {
    /// Everything, full geometry. Only ever the tile you are standing on.
    Near,
    /// No ground cover, simplified canopies.
    Mid,
    /// Trees only, coarsest geometry — the silhouette layer.
    Far,
}

/// Builds one merged mesh for a scattered set. Positions are tile-local, so the
/// caller places the batch with the tile's own transform.
pub fn build(plants: &[Plant], table: &[Species], detail: Detail) -> crate::MeshData {
    let mut b = Builder::default();
    for (i, pl) in plants.iter().enumerate() {
        let sp = match table.get(pl.species as usize) {
            Some(s) => s,
            None => continue,
        };
        let o = pl.position;
        let before = b.m.positions.len();
        // A 0.5 m blade cannot travel half a metre in the wind; a 15 m
        // canopy can. Scale each plant's sway by its own size so one wind
        // strength in the scene is right for everything growing in it.
        let scale = (pl.height / 3.0).clamp(0.12, 1.0);
        match sp.form {
            Form::Grass => {
                if detail != Detail::Near {
                    continue;
                }
                blade(&mut b, pl, pl.height * 0.055, 0.30, scale);
            }
            Form::Flower => {
                if detail != Detail::Near {
                    continue;
                }
                blade(&mut b, pl, pl.height * 0.045, 0.20, scale);
                // The bloom sits at the tip and is the only part with the
                // species colour; the stem stays green.
                cone(&mut b, [0.0, pl.height * 0.92, 0.0], pl.height * 0.18, pl.height * 0.16, 5, sp.color, 0.9 * scale);
            }
            Form::Shrub => {
                if detail == Detail::Far {
                    continue;
                }
                let seg = if detail == Detail::Near { 7 } else { 5 };
                blob(&mut b, [0.0, pl.height * 0.55, 0.0], [pl.height * 0.62, pl.height * 0.55, pl.height * 0.62],
                     seg, 3, pl.color, 0.45 * scale, 0x9e37 ^ i as u32);
            }
            Form::Tree => {
                let h = pl.height;
                let sides = match detail { Detail::Near => 6, Detail::Mid => 5, Detail::Far => 4 };
                let bark = [0.20, 0.15, 0.11];
                if sp.name.contains("conifer") {
                    trunk(&mut b, h, h * 0.030, h * 0.010, sides, bark);
                    // Stacked skirts, widest at the bottom — a spruce silhouette
                    // is the stack, not the individual cone.
                    let tiers = match detail { Detail::Near => 4, Detail::Mid => 3, Detail::Far => 2 };
                    for t in 0..tiers {
                        let f = t as f32 / tiers as f32;
                        let y = h * (0.24 + f * 0.56);
                        let r = h * 0.30 * (1.0 - f * 0.62);
                        cone(&mut b, [0.0, y, 0.0], r, h * 0.42, sides + 2, pl.color, (0.22 + f * 0.3) * scale);
                    }
                } else {
                    trunk(&mut b, h * 0.55, h * 0.035, h * 0.022, sides, bark);
                    let (seg, rings) = match detail {
                        Detail::Near => (8, 5),
                        Detail::Mid => (6, 4),
                        Detail::Far => (5, 3),
                    };
                    let blobs = if detail == Detail::Far { 1 } else { 2 };
                    for k in 0..blobs {
                        let f = k as f32;
                        blob(&mut b,
                             [h * 0.06 * f, h * (0.68 + f * 0.16), h * -0.05 * f],
                             [h * (0.34 - f * 0.07), h * (0.28 - f * 0.05), h * (0.34 - f * 0.07)],
                             seg, rings, pl.color, (0.30 + f * 0.12) * scale, 0x51ed ^ (i as u32 * 7 + k as u32));
                    }
                }
            }
        }
        // Everything above was built at the origin; move it onto the plant.
        // Tilt leans the whole individual off the slope so it is not a fence
        // post driven into a hillside.
        let (ts, tc) = pl.tilt_dir.sin_cos();
        let (lean_s, lean_c) = pl.tilt.sin_cos();
        for k in before..b.m.positions.len() {
            let p = b.m.positions[k];
            let leaned = [
                p[0] + tc * p[1] * lean_s,
                p[1] * lean_c,
                p[2] + ts * p[1] * lean_s,
            ];
            b.m.positions[k] = [leaned[0] + o[0], leaned[1] + o[1], leaned[2] + o[2]];
        }
    }
    b.m
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain::{generate, Relief, TerrainRecipe};

    fn tile() -> (Field, TerrainRecipe) {
        let r = TerrainRecipe {
            relief: Relief::Hills,
            cell_size: 4.0,
            humidity: 0.7,
            latitude: 0.4,
            ..Default::default()
        };
        (generate(&r, 97, 0.0, 0.0), r)
    }

    /// The blue-noise guarantee, and the whole reason this module exists: no
    /// two plants of a species may be closer than its spacing. White noise
    /// fails this immediately.
    #[test]
    fn no_two_plants_of_a_species_overlap() {
        let (f, _) = tile();
        let table = default_species();
        for (i, sp) in table.iter().enumerate() {
            let plants = scatter(&f, sp, i as u16, 1.0, 4242, None, 40_000);
            // Bin by spacing, then compare only within the 3x3 neighbourhood:
            // anything closer than the spacing must share or touch a bin, so
            // this proves the same claim without going quadratic.
            let extent = (f.size - 1) as f32 * f.cell_size;
            let n = ((extent / sp.spacing).ceil() as usize).max(1);
            let mut bins: Vec<Vec<usize>> = vec![Vec::new(); n * n];
            for (k, p) in plants.iter().enumerate() {
                let bx = ((p.position[0] / sp.spacing) as usize).min(n - 1);
                let bz = ((p.position[2] / sp.spacing) as usize).min(n - 1);
                bins[bz * n + bx].push(k);
            }
            for bz in 0..n {
                for bx in 0..n {
                    for &a in &bins[bz * n + bx] {
                        for oz in bz.saturating_sub(1)..=(bz + 1).min(n - 1) {
                            for ox in bx.saturating_sub(1)..=(bx + 1).min(n - 1) {
                                for &b in &bins[oz * n + ox] {
                                    if a >= b {
                                        continue;
                                    }
                                    let (p, q) = (plants[a].position, plants[b].position);
                                    let d = (p[0] - q[0]).hypot(p[2] - q[2]);
                                    assert!(
                                        d >= sp.spacing - 1e-3,
                                        "{} pair {d:.3} m apart, spacing is {}",
                                        sp.name,
                                        sp.spacing
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Every plant stands exactly on the ground the mesh draws.
    #[test]
    fn plants_sit_on_the_terrain() {
        let (f, _) = tile();
        let table = default_species();
        let plants = scatter_form(&f, &table, Form::Tree, 1.0, 7, None, 20_000);
        assert!(!plants.is_empty(), "no trees grew at all");
        for p in &plants {
            let expect = f.height_at(p.position[0] / f.cell_size, p.position[2] / f.cell_size);
            assert!((p.position[1] - expect).abs() < 1e-3, "a tree floated");
        }
    }

    /// Same seed, same forest — twice, and on every machine.
    #[test]
    fn scatter_is_deterministic() {
        let (f, _) = tile();
        let table = default_species();
        let a = scatter_form(&f, &table, Form::Tree, 1.0, 11, None, 20_000);
        let b = scatter_form(&f, &table, Form::Tree, 1.0, 11, None, 20_000);
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(&b) {
            assert_eq!(x.position, y.position);
            assert_eq!(x.species, y.species);
        }
    }

    /// Nothing grows in open water, and nothing grows on a wall.
    #[test]
    fn climate_and_slope_are_obeyed() {
        let r = TerrainRecipe {
            relief: Relief::Archipelago,
            cell_size: 8.0,
            humidity: 0.8,
            ..Default::default()
        };
        let f = generate(&r, 97, 0.0, 0.0);
        let table = default_species();
        for p in scatter_form(&f, &table, Form::Tree, 1.0, 3, None, 20_000) {
            let gx = ((p.position[0] / f.cell_size).round() as usize).min(f.size - 1);
            let gz = ((p.position[2] / f.cell_size).round() as usize).min(f.size - 1);
            let i = f.idx(gx, gz);
            assert!(f.height[i] > r.sea_level - 3.0, "a tree grew in the sea");
            assert!(f.slope[i] < 0.85, "a tree grew on a cliff face");
        }
    }

    /// Clustering: plants must arrive in stands with clearings between, not
    /// as an even sprinkle.
    ///
    /// The baseline is the same species with its clumping turned off, NOT a
    /// Poisson process -- blue noise deliberately suppresses variance below
    /// Poisson, so measuring against the mean would credit the scatter for
    /// exactly the property it is supposed to have. Measured over an area
    /// several stands wide, because one tile can honestly be all forest.
    #[test]
    fn growth_clusters_into_stands() {
        let r = TerrainRecipe {
            relief: Relief::Hills,
            cell_size: 8.0,
            humidity: 0.7,
            latitude: 0.4,
            ..Default::default()
        };
        let f = generate(&r, 129, 0.0, 0.0);
        let table = default_species();
        let i = table.iter().position(|s| s.name == "conifer").unwrap();
        let mut even = table[i].clone();
        even.clumping = 0.0;

        // Bins must be smaller than a stand, or the measurement averages the
        // stands away -- 128 m bins over 133 m stands reported almost nothing.
        let measure = |plants: &[Plant]| -> (f32, f32) {
            let extent = (f.size - 1) as f32 * f.cell_size;
            const N: usize = 16;
            let mut bins = vec![0usize; N * N];
            for p in plants {
                let bx = ((p.position[0] / extent * N as f32) as usize).min(N - 1);
                let bz = ((p.position[2] / extent * N as f32) as usize).min(N - 1);
                bins[bz * N + bx] += 1;
            }
            let mean = plants.len() as f32 / bins.len() as f32;
            let var = bins.iter().map(|c| (*c as f32 - mean).powi(2)).sum::<f32>()
                / bins.len() as f32;
            // Coefficient of variation, and the share of ground that is open.
            let empty = bins.iter().filter(|c| **c * 4 < mean as usize).count() as f32
                / bins.len() as f32;
            (var.sqrt() / mean.max(1e-3), empty)
        };

        let clumped = scatter(&f, &table[i], i as u16, 1.0, 5, None, 20_000);
        let sprinkled = scatter(&f, &even, i as u16, 1.0, 5, None, 20_000);
        assert!(clumped.len() > 100, "too few trees to judge ({})", clumped.len());
        let (cv_a, open_a) = measure(&clumped);
        let (cv_b, open_b) = measure(&sprinkled);
        assert!(
            cv_a > cv_b * 1.5,
            "clumping barely changed the distribution (cv {cv_a:.2} vs {cv_b:.2})"
        );
        // The point of clustering is the clearing, so measure the clearing.
        assert!(
            open_a > 0.15 && open_a > open_b * 2.0,
            "no real clearings: {:.0}% open clumped vs {:.0}% even",
            open_a * 100.0,
            open_b * 100.0
        );
    }

    /// Merged geometry must be structurally sound: every index in range, and
    /// no vertex adrift from the plant it belongs to.
    #[test]
    fn built_geometry_is_sound() {
        let (f, _) = tile();
        let table = default_species();
        let plants = scatter_form(&f, &table, Form::Tree, 1.0, 21, None, 400);
        let m = build(&plants, &table, Detail::Near);
        assert!(!m.indices.is_empty(), "trees built no geometry");
        let n = m.positions.len() as u32;
        assert!(m.indices.iter().all(|i| *i < n), "index out of range");
        assert_eq!(m.normals.len(), m.positions.len());
        assert_eq!(m.colors.len(), m.positions.len());
        assert_eq!(m.uvs.len(), m.positions.len());
        assert_eq!(m.tangents.len(), m.positions.len());
        assert_eq!(m.indices.len() % 3, 0);
        for v in &m.positions {
            assert!(v.iter().all(|c| c.is_finite()), "non-finite vertex");
        }
    }

    /// The LOD ladder must actually shed work, and it must shed it in the
    /// right order: ground cover first, silhouettes last.
    #[test]
    fn detail_sheds_geometry_in_order() {
        let (f, _) = tile();
        let table = default_species();
        let mut all = scatter_form(&f, &table, Form::Grass, 1.0, 3, None, 4_000);
        all.extend(scatter_form(&f, &table, Form::Shrub, 1.0, 4, None, 2_000));
        all.extend(scatter_form(&f, &table, Form::Tree, 1.0, 5, None, 400));
        let near = build(&all, &table, Detail::Near).positions.len();
        let mid = build(&all, &table, Detail::Mid).positions.len();
        let far = build(&all, &table, Detail::Far).positions.len();
        assert!(mid < near, "mid ({mid}) did not drop ground cover from near ({near})");
        assert!(far < mid, "far ({far}) did not simplify past mid ({mid})");
        assert!(far > 0, "far dropped the silhouettes too");
    }

    /// The wind convention, which every other mesh in the engine depends on:
    /// alpha 1.0 is rigid, so nothing that predates flora ever moves. Within a
    /// plant, the root must be stiffer than the tip.
    #[test]
    fn sway_rides_in_alpha_and_roots_are_rigid() {
        let (f, _) = tile();
        let table = default_species();
        let plants = scatter_form(&f, &table, Form::Grass, 1.0, 8, None, 2_000);
        assert!(!plants.is_empty());
        let m = build(&plants, &table, Detail::Near);
        let mut lowest = (f32::MAX, 1.0f32);
        let mut highest = (f32::MIN, 1.0f32);
        for (v, c) in m.positions.iter().zip(&m.colors) {
            assert!((0.0..=1.0).contains(&c[3]), "alpha out of range: {}", c[3]);
            if v[1] < lowest.0 { lowest = (v[1], c[3]); }
            if v[1] > highest.0 { highest = (v[1], c[3]); }
        }
        assert!(lowest.1 > highest.1, "root ({}) was not stiffer than tip ({})", lowest.1, highest.1);
        assert!(lowest.1 > 0.9, "blade roots should be near-rigid, got {}", lowest.1);
    }

    /// A trunk does not sway; a tree bends in its canopy. If trunks moved, a
    /// forest would slide along the ground in the wind.
    #[test]
    fn trunks_are_rigid() {
        let table = default_species();
        let conifer = table.iter().position(|s| s.name == "conifer").unwrap();
        let pl = Plant {
            position: [0.0, 0.0, 0.0], yaw: 0.0, height: 14.0, tilt: 0.0,
            tilt_dir: 0.0, color: [0.2, 0.3, 0.2], species: conifer as u16,
        };
        let m = build(&[pl], &table, Detail::Near);
        for (v, c) in m.positions.iter().zip(&m.colors) {
            if v[1] < 0.2 {
                assert!(c[3] > 0.999, "the base of the trunk swayed (alpha {})", c[3]);
            }
        }
    }

    /// Detail must never change WHERE things are, only how much geometry they
    /// get -- otherwise a tree would jump sideways as you walked toward it.
    #[test]
    fn detail_does_not_move_anything() {
        let (f, _) = tile();
        let table = default_species();
        let plants = scatter_form(&f, &table, Form::Tree, 1.0, 13, None, 200);
        for d in [Detail::Near, Detail::Mid, Detail::Far] {
            let m = build(&plants, &table, d);
            for pl in &plants {
                let near_any = m.positions.iter().any(|v| {
                    (v[0] - pl.position[0]).hypot(v[2] - pl.position[2]) < pl.height
                });
                assert!(near_any, "a tree vanished or moved at {d:?}");
            }
        }
    }

    /// Winding. The engine's convention is CCW-outside, and a plant built
    /// inside-out is lit from behind -- it goes black, which is exactly what
    /// a distant conifer wood did the first time this ran. Checked by
    /// comparing each triangle's geometric normal against the shading normals
    /// its own vertices carry.
    #[test]
    fn geometry_is_wound_outward() {
        let table = default_species();
        for (i, sp) in table.iter().enumerate() {
            let pl = Plant {
                position: [0.0, 0.0, 0.0], yaw: 0.0, height: 6.0, tilt: 0.0,
                tilt_dir: 0.0, color: [0.3, 0.4, 0.2], species: i as u16,
            };
            let m = build(&[pl], &table, Detail::Near);
            let mut wrong = 0;
            let mut total = 0;
            for tri in m.indices.chunks(3) {
                let (a, b, c) = (
                    m.positions[tri[0] as usize],
                    m.positions[tri[1] as usize],
                    m.positions[tri[2] as usize],
                );
                let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
                let fnorm = [
                    u[1] * v[2] - u[2] * v[1],
                    u[2] * v[0] - u[0] * v[2],
                    u[0] * v[1] - u[1] * v[0],
                ];
                let len = (fnorm[0] * fnorm[0] + fnorm[1] * fnorm[1] + fnorm[2] * fnorm[2]).sqrt();
                if len < 1e-9 {
                    continue; // degenerate (the tapered blade tip)
                }
                let sh = m.normals[tri[0] as usize];
                let d = (fnorm[0] * sh[0] + fnorm[1] * sh[1] + fnorm[2] * sh[2]) / len;
                total += 1;
                if d < -0.1 {
                    wrong += 1;
                }
            }
            assert!(
                wrong * 20 <= total,
                "{}: {wrong} of {total} triangles wound inside-out",
                sp.name
            );
        }
    }

    /// A real per-tile seed is a full-width u32, not the small number a test
    /// reaches for. Scattering with one must not panic -- this is the exact
    /// case that took the browser down: a large seed pushed the clustering
    /// noise past the end of its lattice.
    #[test]
    fn survives_a_full_width_seed() {
        let (f, _) = tile();
        let table = default_species();
        for seed in [0u32, 1, 0x9E37_79B9, u32::MAX, u32::MAX - 7] {
            let p = scatter_form(&f, &table, Form::Tree, 1.0, seed, None, 300);
            assert!(p.iter().all(|q| q.position.iter().all(|c| c.is_finite())));
        }
    }


    /// Every tangent must be perpendicular to its own normal and of unit
    /// length. A tangent parallel to the normal makes the renderer normalize a
    /// zero vector, and the NaN comes out the other end as a pixel of pure
    /// black -- which is exactly how a forest first rendered.
    #[test]
    fn tangents_are_perpendicular_to_normals() {
        let (f, _) = tile();
        let table = default_species();
        let mut plants = scatter_form(&f, &table, Form::Tree, 1.0, 31, None, 200);
        plants.extend(scatter_form(&f, &table, Form::Grass, 1.0, 32, None, 500));
        plants.extend(scatter_form(&f, &table, Form::Shrub, 1.0, 33, None, 200));
        let m = build(&plants, &table, Detail::Near);
        assert!(!m.positions.is_empty());
        for (n, t) in m.normals.iter().zip(&m.tangents) {
            let d = n[0] * t[0] + n[1] * t[1] + n[2] * t[2];
            assert!(d.abs() < 1e-3, "tangent not perpendicular (dot {d})");
            let l = (t[0] * t[0] + t[1] * t[1] + t[2] * t[2]).sqrt();
            assert!((l - 1.0).abs() < 1e-3, "tangent not unit length ({l})");
            assert!(t.iter().all(|c| c.is_finite()), "non-finite tangent");
        }
    }


    /// Shading normals must be unit length -- lighting and the tangent frame
    /// both assume it.
    #[test]
    fn normals_are_unit_length() {
        let (f, _) = tile();
        let table = default_species();
        let mut plants = scatter_form(&f, &table, Form::Grass, 1.0, 41, None, 400);
        plants.extend(scatter_form(&f, &table, Form::Tree, 1.0, 42, None, 100));
        let m = build(&plants, &table, Detail::Near);
        for n in &m.normals {
            let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            assert!((l - 1.0).abs() < 1e-3, "normal not unit length ({l})");
        }
    }

    /// The cover knob actually thins growth.
    #[test]
    fn cover_thins_the_scatter() {
        let (f, _) = tile();
        let table = default_species();
        let full = scatter_form(&f, &table, Form::Tree, 1.0, 9, None, 20_000).len();
        let sparse = scatter_form(&f, &table, Form::Tree, 0.25, 9, None, 20_000).len();
        let none = scatter_form(&f, &table, Form::Tree, 0.0, 9, None, 20_000).len();
        assert!(sparse < full, "cover 0.25 grew as much as 1.0 ({sparse} vs {full})");
        assert_eq!(none, 0, "cover 0 still grew {none}");
    }
}
