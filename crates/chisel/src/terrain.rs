//! Terra — the Thread's landscape generator.
//!
//! One physically-motivated simulation, read many ways. Almost every engine
//! generates terrain, then biomes, then vegetation, then rocks as four
//! independent systems that never quite agree — forests that ignore the
//! valley, deserts on the wet side of a mountain, boulders with no reason to
//! be there. That disagreement *is* what fake looks like.
//!
//! So this does the opposite: uplift, then erosion, then flow — and the mesh,
//! the rivers, the climate, the plant cover and the stone all fall out of the
//! same run, unable to contradict each other.
//!
//! ```text
//!   uplift ──▶ erode ──▶ flow ──▶ climate ──▶ cover ──▶ scatter
//!   (why the      (what      (rivers,   (rain      (biome    (plants,
//!    land rises)   water      for free)  shadow)    weights)   stone)
//!                  did)
//! ```
//!
//! Pure: no rendering, no I/O, no globals. Both renderers ask this one
//! implementation, so a world cannot look different depending on who opened
//! it — the same rule the builtin primitives live under.

use std::f32::consts::PI;

/// How dramatic the land is. A preset picks the uplift blend and the erosion
/// budget; everything else follows from the simulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relief {
    /// Wide and gentle — river plains, long sightlines.
    Plains,
    /// Rolling country. The default: readable, walkable, never flat.
    Hills,
    /// Sharp ridgelines, deep valleys, scree. Ridged multifractal dominant.
    Alpine,
    /// Heavily eroded soft rock — mesas, gullies, exposed strata.
    Badlands,
    /// Mostly below sea level, with island chains rising out of it.
    Archipelago,
}

impl Relief {
    /// Parse the manifest's spelling. Unknown names fall back to hills rather
    /// than failing a world — an author's typo should cost them drama, not
    /// their landscape.
    pub fn from_str(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "plains" => Relief::Plains,
            "alpine" => Relief::Alpine,
            "badlands" => Relief::Badlands,
            "archipelago" => Relief::Archipelago,
            _ => Relief::Hills,
        }
    }

    /// (ridged share, vertical scale in metres, erosion droplets per cell).
    fn profile(self) -> (f32, f32, f32) {
        match self {
            Relief::Plains => (0.05, 55.0, 0.30),
            Relief::Hills => (0.32, 260.0, 0.55),
            Relief::Alpine => (0.82, 1500.0, 0.90),
            Relief::Badlands => (0.48, 420.0, 1.60), // erosion IS the look
            Relief::Archipelago => (0.45, 520.0, 0.55),
        }
    }
}

/// A landscape, as a recipe. Seven fields describe a world.
#[derive(Debug, Clone)]
pub struct TerrainRecipe {
    pub seed: u32,
    pub relief: Relief,
    /// Metres above the datum that counts as shoreline.
    pub sea_level: f32,
    /// 0 = equator, 1 = pole. Drives the temperature band.
    pub latitude: f32,
    /// Prevailing wind, normalised on use. Rain falls on the windward side.
    pub wind: [f32; 2],
    /// 0 = arid, 1 = drenched. The moisture the wind starts with.
    pub humidity: f32,
    /// Metres per cell of the simulation grid. Bigger = coarser, faster.
    pub cell_size: f32,
}

impl Default for TerrainRecipe {
    fn default() -> Self {
        Self {
            seed: 1337,
            relief: Relief::Hills,
            sea_level: 0.0,
            latitude: 0.5,
            wind: [1.0, 0.0],
            humidity: 0.6,
            cell_size: 4.0,
        }
    }
}

/// The Whittaker set. Weights, never an id — biomes blend at their edges, and
/// a hard boundary in nature is the exception, not the rule.
pub const BIOMES: [&str; 8] = [
    "water", "beach", "desert", "grassland", "shrubland", "forest", "taiga", "alpine",
];
pub const B_WATER: usize = 0;
pub const B_BEACH: usize = 1;
pub const B_DESERT: usize = 2;
pub const B_GRASSLAND: usize = 3;
pub const B_SHRUBLAND: usize = 4;
pub const B_FOREST: usize = 5;
pub const B_TAIGA: usize = 6;
pub const B_ALPINE: usize = 7;

/// One simulated tile: every field the rest of the system reads.
///
/// All the vectors are `size × size`, row-major, and describe the *same*
/// cells — so a lookup at one index is consistent across every field, which
/// is the whole point of running one simulation.
#[derive(Debug, Clone)]
pub struct Field {
    pub size: usize,
    pub cell_size: f32,
    /// Metres. Post-erosion.
    pub height: Vec<f32>,
    /// Metres of loose material sitting on bedrock — scree, sand, alluvium.
    /// Thermal erosion writes it; the rock scatter reads it.
    pub sediment: Vec<f32>,
    /// How much upstream land drains through each cell. The river map.
    pub flow: Vec<f32>,
    /// 0..1, where 1 is vertical.
    pub slope: Vec<f32>,
    /// °C-ish, unitless but monotonic: latitude minus altitude lapse.
    pub temperature: Vec<f32>,
    /// 0..1 after rain shadow and river proximity.
    pub moisture: Vec<f32>,
    /// `size*size*8` — biome weights per cell, summing to 1.
    pub biome: Vec<f32>,
}

impl Field {
    #[inline]
    pub fn idx(&self, x: usize, y: usize) -> usize {
        y * self.size + x
    }

    /// Bilinear height at a fractional cell coordinate — the sampler meshing
    /// and scatter both use, so a tree stands exactly on the ground the mesh
    /// draws.
    pub fn height_at(&self, fx: f32, fy: f32) -> f32 {
        bilinear(&self.height, self.size, fx, fy)
    }

    pub fn flow_at(&self, fx: f32, fy: f32) -> f32 {
        bilinear(&self.flow, self.size, fx, fy)
    }

    pub fn sediment_at(&self, fx: f32, fy: f32) -> f32 {
        bilinear(&self.sediment, self.size, fx, fy)
    }

    /// The dominant biome at a cell, for tests and debug colour.
    pub fn dominant(&self, x: usize, y: usize) -> usize {
        let base = self.idx(x, y) * BIOMES.len();
        let mut best = 0;
        for b in 1..BIOMES.len() {
            if self.biome[base + b] > self.biome[base + best] {
                best = b;
            }
        }
        best
    }
}

fn bilinear(v: &[f32], size: usize, fx: f32, fy: f32) -> f32 {
    let max = size as f32 - 1.0;
    let x = fx.clamp(0.0, max);
    let y = fy.clamp(0.0, max);
    let x0 = x.floor() as usize;
    let y0 = y.floor() as usize;
    let x1 = (x0 + 1).min(size - 1);
    let y1 = (y0 + 1).min(size - 1);
    let tx = x - x0 as f32;
    let ty = y - y0 as f32;
    let a = v[y0 * size + x0] * (1.0 - tx) + v[y0 * size + x1] * tx;
    let b = v[y1 * size + x0] * (1.0 - tx) + v[y1 * size + x1] * tx;
    a * (1.0 - ty) + b * ty
}

// ─── noise ────────────────────────────────────────────────────────────────
// Value noise with a hashed lattice: no tables, no allocation, identical on
// every machine. Terrain quality comes from what we *stack* on top of it —
// the ridges, the warp, and above all the erosion — not from the noise basis.

#[inline]
fn hash2(x: i32, y: i32, seed: u32) -> f32 {
    let mut h = (x as u32)
        .wrapping_mul(374_761_393)
        .wrapping_add((y as u32).wrapping_mul(668_265_263))
        .wrapping_add(seed.wrapping_mul(1_442_695_041));
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    ((h ^ (h >> 16)) as f32 / u32::MAX as f32) * 2.0 - 1.0
}

#[inline]
fn smootherstep(t: f32) -> f32 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

fn value_noise(x: f32, y: f32, seed: u32) -> f32 {
    let xi = x.floor();
    let yi = y.floor();
    let tx = smootherstep(x - xi);
    let ty = smootherstep(y - yi);
    let (xi, yi) = (xi as i32, yi as i32);
    let a = hash2(xi, yi, seed);
    let b = hash2(xi + 1, yi, seed);
    let c = hash2(xi, yi + 1, seed);
    let d = hash2(xi + 1, yi + 1, seed);
    let top = a + (b - a) * tx;
    let bot = c + (d - c) * tx;
    top + (bot - top) * ty
}

/// Fractal Brownian motion — the soft, rolling half of a landscape.
fn fbm(x: f32, y: f32, octaves: u32, seed: u32) -> f32 {
    let (mut f, mut amp, mut sum, mut norm) = (1.0f32, 1.0f32, 0.0f32, 0.0f32);
    for o in 0..octaves {
        sum += value_noise(x * f, y * f, seed.wrapping_add(o * 7919)) * amp;
        norm += amp;
        f *= 2.0;
        amp *= 0.5;
    }
    sum / norm.max(1e-6)
}

/// Ridged multifractal — the sharp half.
///
/// `1 − |n|` folds the noise into creases, and weighting each octave by the
/// last keeps detail on the ridges while leaving the valleys smooth. This is
/// what makes mountains read as *ranges* with connected crests instead of a
/// field of lumps, and plain fbm will never do it however it is tuned.
fn ridged(x: f32, y: f32, octaves: u32, seed: u32) -> f32 {
    let (mut f, mut amp, mut sum, mut norm) = (1.0f32, 1.0f32, 0.0f32, 0.0f32);
    let mut weight = 1.0f32;
    for o in 0..octaves {
        let n = value_noise(x * f, y * f, seed.wrapping_add(o * 6151));
        let mut r = 1.0 - n.abs();
        r *= r;
        r *= weight;
        weight = (r * 2.0).clamp(0.0, 1.0);
        sum += r * amp;
        norm += amp;
        f *= 2.0;
        amp *= 0.5;
    }
    (sum / norm.max(1e-6)) * 2.0 - 1.0
}

/// The uplift field: *why* the land rises, before any water touches it.
///
/// Ridged where the relief wants mountains, fbm underneath for the lowlands,
/// and the whole domain warped so nothing lines up with the sampling grid.
fn uplift(wx: f32, wy: f32, r: &TerrainRecipe) -> f32 {
    let (ridge_share, scale, _) = r.relief.profile();
    // Continental frequency — one "range" per few kilometres.
    let f = 1.0 / 2600.0;
    // Domain warp: sample the field at a position that is itself noisy.
    let wxw = wx + fbm(wx * f * 1.7, wy * f * 1.7, 3, r.seed ^ 0x51ed) * 900.0;
    let wyw = wy + fbm(wx * f * 1.7 + 31.4, wy * f * 1.7 + 17.2, 3, r.seed ^ 0x9e37) * 900.0;

    let soft = fbm(wxw * f, wyw * f, 6, r.seed) * 0.5 + 0.5;
    let sharp = ridged(wxw * f, wyw * f, 6, r.seed ^ 0x2545) * 0.5 + 0.5;
    let mut h = soft * (1.0 - ridge_share) + sharp * ridge_share;

    // A continental mask so the world has coasts instead of an endless plateau.
    let cont = fbm(wxw * f * 0.35, wyw * f * 0.35, 3, r.seed ^ 0x7f4a);
    h += cont * 0.35;

    if r.relief == Relief::Archipelago {
        // Push the datum up so most of the field drowns and only peaks remain.
        h -= 0.26;
    }
    h * scale
}

// ─── the simulation ───────────────────────────────────────────────────────

/// Generate one tile of `size × size` cells whose south-west corner sits at
/// `(origin_x, origin_y)` metres.
///
/// `margin` cells of context are simulated beyond every edge and then trimmed,
/// so a droplet that would have run off the edge still carves the part of its
/// valley that lands inside the tile. That is what makes neighbouring tiles
/// agree without ever having met — the seam problem, solved by overlap rather
/// than by stitching.
pub fn generate(r: &TerrainRecipe, size: usize, origin_x: f32, origin_y: f32) -> Field {
    let margin = (size / 4).clamp(8, 48);
    let big = size + margin * 2;
    let cs = r.cell_size;

    // 1. Uplift.
    let mut height = vec![0.0f32; big * big];
    for y in 0..big {
        for x in 0..big {
            let wx = origin_x + (x as f32 - margin as f32) * cs;
            let wy = origin_y + (y as f32 - margin as f32) * cs;
            height[y * big + x] = uplift(wx, wy, r);
        }
    }

    // 2. Erosion — the step that turns noise into landscape.
    let mut sediment = vec![0.0f32; big * big];
    let (_, _, budget) = r.relief.profile();
    let droplets = ((big * big) as f32 * budget) as usize;
    hydraulic(&mut height, &mut sediment, big, cs, droplets, r.seed);
    thermal(&mut height, &mut sediment, big, cs, 12);

    // Loose material settles: a light smoothing of the deposition map. Raw
    // per-droplet deposits are salt-and-pepper and dithered every shoreline.
    let sediment = blur(&sediment, big, 2);

    // 3. Flow — free, and it is the river map.
    let flow = flow_accumulation(&height, big);

    // 4/5. Climate and cover, per cell.
    let flow_soft = blur(&flow, big, 2);
    let slope_full = slopes(&height, big, cs);
    let moisture_full = blur(&moisture(&height, &flow, big, cs, r), big, 4);

    // Trim the margin away: everything below is tile-local.
    let mut f = Field {
        size,
        cell_size: cs,
        height: vec![0.0; size * size],
        sediment: vec![0.0; size * size],
        flow: vec![0.0; size * size],
        slope: vec![0.0; size * size],
        temperature: vec![0.0; size * size],
        moisture: vec![0.0; size * size],
        biome: vec![0.0; size * size * BIOMES.len()],
    };
    for y in 0..size {
        for x in 0..size {
            let s = (y + margin) * big + (x + margin);
            let d = y * size + x;
            f.height[d] = height[s];
            f.sediment[d] = sediment[s];
            f.flow[d] = flow[s];
            f.slope[d] = slope_full[s];
            f.moisture[d] = moisture_full[s];
            f.temperature[d] = temperature(height[s], r);
            let w = classify(
                height[s],
                slope_full[s],
                temperature(height[s], r),
                moisture_full[s],
                flow_soft[s],
                r,
            );
            f.biome[d * BIOMES.len()..(d + 1) * BIOMES.len()].copy_from_slice(&w);
        }
    }
    f
}

/// Droplet hydraulic erosion.
///
/// Each droplet is a particle with position, velocity, water and a sediment
/// load. It flows downhill along the interpolated gradient; where the slope is
/// steep it takes more than it carries and cuts, where the ground flattens it
/// drops the surplus. Repeated a few hundred thousand times this produces the
/// dendritic drainage every real landscape has and no noise function contains:
/// V-profile valleys upstream, alluvial fans where the grade eases, ridgelines
/// that connect because the water had to go *around* them.
fn hydraulic(h: &mut [f32], sed: &mut [f32], size: usize, cell: f32, droplets: usize, seed: u32) {
    const INERTIA: f32 = 0.05;
    const CAPACITY: f32 = 4.0;
    const EROSION: f32 = 0.3;
    const DEPOSITION: f32 = 0.3;
    const EVAPORATION: f32 = 0.02;
    const GRAVITY: f32 = 10.0;
    const MAX_STEPS: usize = 64;
    const RADIUS: i32 = 2;

    let mut rng = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 17;
        rng ^= rng << 5;
        rng as f32 / u32::MAX as f32
    };

    for _ in 0..droplets {
        let mut px = next() * (size as f32 - 1.0);
        let mut py = next() * (size as f32 - 1.0);
        let (mut dx, mut dy) = (0.0f32, 0.0f32);
        let mut water = 1.0f32;
        let mut carry = 0.0f32;
        let mut speed = 1.0f32;

        for _ in 0..MAX_STEPS {
            let (gx, gy) = gradient(h, size, px, py);
            // Momentum: a droplet does not turn on a sixpence, which is what
            // keeps valleys running straight instead of scribbling.
            dx = dx * INERTIA - gx * (1.0 - INERTIA);
            dy = dy * INERTIA - gy * (1.0 - INERTIA);
            let len = (dx * dx + dy * dy).sqrt();
            if len < 1e-5 {
                break;
            }
            dx /= len;
            dy /= len;
            let (nx, ny) = (px + dx, py + dy);
            if nx < 1.0 || ny < 1.0 || nx >= size as f32 - 2.0 || ny >= size as f32 - 2.0 {
                break;
            }
            let old_h = bilinear(h, size, px, py);
            let new_h = bilinear(h, size, nx, ny);
            let drop = old_h - new_h;

            let capacity = (drop.max(0.0) * speed * water * CAPACITY).max(0.01);
            if carry > capacity || drop < 0.0 {
                // Uphill or over capacity: put material down. Filling a pit
                // with exactly the step height is what lets lakes and flats
                // form instead of the droplet drilling forever.
                let amount = if drop < 0.0 {
                    (-drop).min(carry)
                } else {
                    (carry - capacity) * DEPOSITION
                };
                carry -= amount;
                deposit(h, sed, size, px, py, amount);
            } else {
                let amount = ((capacity - carry) * EROSION).min(drop.max(0.0));
                carry += amount;
                erode(h, sed, size, px, py, amount, RADIUS);
            }

            speed = (speed * speed + drop * GRAVITY).max(0.0).sqrt();
            water *= 1.0 - EVAPORATION;
            if water < 0.01 {
                break;
            }
            px = nx;
            py = ny;
        }
        let _ = cell;
    }
}

fn gradient(h: &[f32], size: usize, px: f32, py: f32) -> (f32, f32) {
    let x = px.floor() as usize;
    let y = py.floor() as usize;
    let x1 = (x + 1).min(size - 1);
    let y1 = (y + 1).min(size - 1);
    let tx = px - x as f32;
    let ty = py - y as f32;
    let h00 = h[y * size + x];
    let h10 = h[y * size + x1];
    let h01 = h[y1 * size + x];
    let h11 = h[y1 * size + x1];
    (
        (h10 - h00) * (1.0 - ty) + (h11 - h01) * ty,
        (h01 - h00) * (1.0 - tx) + (h11 - h10) * tx,
    )
}

fn deposit(h: &mut [f32], sed: &mut [f32], size: usize, px: f32, py: f32, amount: f32) {
    let x = px.floor() as usize;
    let y = py.floor() as usize;
    let x1 = (x + 1).min(size - 1);
    let y1 = (y + 1).min(size - 1);
    let tx = px - x as f32;
    let ty = py - y as f32;
    for (i, w) in [
        (y * size + x, (1.0 - tx) * (1.0 - ty)),
        (y * size + x1, tx * (1.0 - ty)),
        (y1 * size + x, (1.0 - tx) * ty),
        (y1 * size + x1, tx * ty),
    ] {
        h[i] += amount * w;
        sed[i] += amount * w;
    }
}

/// Remove material over a small disc rather than a single cell — a
/// single-cell cut leaves needle artefacts that read as noise, not erosion.
fn erode(h: &mut [f32], sed: &mut [f32], size: usize, px: f32, py: f32, amount: f32, radius: i32) {
    let cx = px.round() as i32;
    let cy = py.round() as i32;
    let mut total = 0.0f32;
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            let d = ((dx * dx + dy * dy) as f32).sqrt();
            if d <= radius as f32 {
                total += 1.0 - d / (radius as f32 + 1e-6);
            }
        }
    }
    if total <= 0.0 {
        return;
    }
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            let (x, y) = (cx + dx, cy + dy);
            if x < 0 || y < 0 || x >= size as i32 || y >= size as i32 {
                continue;
            }
            let d = ((dx * dx + dy * dy) as f32).sqrt();
            if d > radius as f32 {
                continue;
            }
            let w = (1.0 - d / (radius as f32 + 1e-6)) / total;
            let i = y as usize * size + x as usize;
            let take = amount * w;
            h[i] -= take;
            sed[i] = (sed[i] - take).max(0.0);
        }
    }
}

/// Thermal erosion: anything steeper than the talus angle slides.
///
/// This is what puts scree at the foot of a cliff and stops mountains from
/// being infinitely sharp. The sediment it moves is also the map the rock
/// scatter reads later — a boulder field with a reason to exist.
fn thermal(h: &mut [f32], sed: &mut [f32], size: usize, cell: f32, iterations: usize) {
    // ~34°, the repose angle of loose rock.
    let talus = 0.7 * cell;
    for _ in 0..iterations {
        let src = h.to_vec();
        for y in 1..size - 1 {
            for x in 1..size - 1 {
                let i = y * size + x;
                let me = src[i];
                let mut total = 0.0f32;
                let mut diffs = [0.0f32; 4];
                for (k, n) in [i - 1, i + 1, i - size, i + size].iter().enumerate() {
                    let d = me - src[*n];
                    if d > talus {
                        diffs[k] = d - talus;
                        total += d - talus;
                    }
                }
                if total <= 0.0 {
                    continue;
                }
                // Damped by 0.25 to stop the cell oscillating with its
                // neighbours across iterations. It used to also cap at
                // `me * 0.5` — the cell's own ALTITUDE — which is nonsense for
                // anything below the datum: a cell at −40 m got a negative
                // budget, so it pumped material UP toward zero and pushed its
                // neighbours down, tearing every seabed into ±90 m spikes and
                // driving the sediment map negative. How much rock can slide
                // has nothing to do with how high above sea level it sits.
                let move_total = total * 0.25;
                h[i] -= move_total;
                for (k, n) in [i - 1, i + 1, i - size, i + size].iter().enumerate() {
                    if diffs[k] > 0.0 {
                        let share = move_total * (diffs[k] / total);
                        h[*n] += share;
                        sed[*n] += share;
                    }
                }
            }
        }
    }
}

/// Flow accumulation: how much upstream land drains through each cell.
///
/// Cells are visited from high to low, each pushing its accumulated water into
/// its lowest neighbour (D8). One sort and one pass — and the result is the
/// river network, which also feeds moisture and riverbed stone. Free, because
/// the erosion already shaped the surface it runs on.
fn flow_accumulation(h: &[f32], size: usize) -> Vec<f32> {
    let mut order: Vec<u32> = (0..(size * size) as u32).collect();
    order.sort_unstable_by(|a, b| h[*b as usize].total_cmp(&h[*a as usize]));
    let mut flow = vec![1.0f32; size * size];
    for &i in &order {
        let i = i as usize;
        let (x, y) = (i % size, i / size);
        if x == 0 || y == 0 || x + 1 == size || y + 1 == size {
            continue;
        }
        let mut lowest = i;
        for n in [i - 1, i + 1, i - size, i + size, i - size - 1, i - size + 1, i + size - 1, i + size + 1] {
            if h[n] < h[lowest] {
                lowest = n;
            }
        }
        if lowest != i {
            let f = flow[i];
            flow[lowest] += f;
        }
    }
    flow
}

/// Separable box blur, `radius` cells. Cheap, and enough to stop a spiky
/// field from dithering a classification boundary.
fn blur(v: &[f32], size: usize, radius: usize) -> Vec<f32> {
    let mut tmp = vec![0.0f32; v.len()];
    let mut out = vec![0.0f32; v.len()];
    let r = radius as isize;
    for y in 0..size {
        for x in 0..size {
            let (mut sum, mut n) = (0.0f32, 0.0f32);
            for d in -r..=r {
                let xx = x as isize + d;
                if xx >= 0 && (xx as usize) < size {
                    sum += v[y * size + xx as usize];
                    n += 1.0;
                }
            }
            tmp[y * size + x] = sum / n;
        }
    }
    for y in 0..size {
        for x in 0..size {
            let (mut sum, mut n) = (0.0f32, 0.0f32);
            for d in -r..=r {
                let yy = y as isize + d;
                if yy >= 0 && (yy as usize) < size {
                    sum += tmp[yy as usize * size + x];
                    n += 1.0;
                }
            }
            out[y * size + x] = sum / n;
        }
    }
    out
}

fn slopes(h: &[f32], size: usize, cell: f32) -> Vec<f32> {
    let mut s = vec![0.0f32; size * size];
    for y in 0..size {
        for x in 0..size {
            let xm = x.saturating_sub(1);
            let xp = (x + 1).min(size - 1);
            let ym = y.saturating_sub(1);
            let yp = (y + 1).min(size - 1);
            let dx = (h[y * size + xp] - h[y * size + xm]) / (2.0 * cell);
            let dy = (h[yp * size + x] - h[ym * size + x]) / (2.0 * cell);
            // tan → 0..1, where 1 is a wall.
            s[y * size + x] = ((dx * dx + dy * dy).sqrt()).atan() / (PI * 0.5);
        }
    }
    s
}

fn temperature(height: f32, r: &TerrainRecipe) -> f32 {
    // Latitude band, then the lapse rate: ~6.5 °C per kilometre climbed.
    let base = 27.0 - r.latitude.clamp(0.0, 1.0) * 52.0;
    base - (height - r.sea_level).max(0.0) * 0.0065
}

/// Moisture: base humidity, minus what the wind lost climbing, plus what the
/// rivers give back.
///
/// The march is the interesting half. Stepping upwind from each cell and
/// shedding moisture wherever the land *rose* puts deserts behind mountains
/// and rainforest on the windward slope — the orographic rain shadow, which is
/// the single most recognisable climate pattern on Earth and is almost free
/// once the height field exists.
fn moisture(h: &[f32], flow: &[f32], size: usize, cell: f32, r: &TerrainRecipe) -> Vec<f32> {
    let wl = (r.wind[0] * r.wind[0] + r.wind[1] * r.wind[1]).sqrt().max(1e-5);
    let (wx, wy) = (r.wind[0] / wl, r.wind[1] / wl);
    let steps = 24;
    let stride = 3.0;

    let mut m = vec![0.0f32; size * size];
    for y in 0..size {
        for x in 0..size {
            let i = y * size + x;
            let start = r.humidity.clamp(0.0, 1.0);
            let mut climbed = 0.0f32;
            let mut prev = bilinear(h, size, x as f32, y as f32);
            // Walk UPWIND, so we accumulate what the air lost getting here.
            for s in 1..=steps {
                let sx = x as f32 - wx * s as f32 * stride;
                let sy = y as f32 - wy * s as f32 * stride;
                if sx < 0.0 || sy < 0.0 || sx >= size as f32 || sy >= size as f32 {
                    break;
                }
                let hh = bilinear(h, size, sx, sy);
                // Air climbing sheds rain; descending air is dry and stays dry.
                climbed += (prev - hh).max(0.0);
                prev = hh;
            }
            // Saturating: the first ridge takes most of the rain, and the
            // tenth cannot take what is no longer there. A linear subtraction
            // drove whole regions to exactly zero and made the map binary.
            let moisture = start * (-climbed / 900.0).exp();
            // Rivers water their own banks.
            let river = (flow[i] / 900.0).clamp(0.0, 1.0).sqrt() * 0.35;
            // Ocean is wet.
            let sea = if h[i] < r.sea_level { 0.4 } else { 0.0 };
            let _ = cell;
            m[i] = (moisture + river + sea).clamp(0.0, 1.0);
        }
    }
    m
}

/// Whittaker classification → blended weights.
fn classify(
    height: f32,
    slope: f32,
    temp: f32,
    moist: f32,
    flow: f32,
    r: &TerrainRecipe,
) -> [f32; 8] {
    let mut w = [0.0f32; 8];

    // Water: below the datum, or a river channel big enough to be one.
    let depth = r.sea_level - height;
    let sea = smoothstep(-3.0, 5.0, depth);
    let river = smoothstep(2200.0, 9000.0, flow) * (1.0 - smoothstep(0.0, 0.25, slope));
    w[B_WATER] = sea.max(river);

    // Beach: the metre or two above the waterline, and only where it's flat.
    w[B_BEACH] = smoothstep(-9.0, -1.0, depth) * (1.0 - sea) * (1.0 - smoothstep(0.10, 0.30, slope));

    // Bare rock wherever it is too steep to hold soil — this is what makes
    // cliffs read as cliffs rather than as vertical lawn.
    let bare = smoothstep(0.42, 0.68, slope);
    let cold = 1.0 - smoothstep(-8.0, 2.0, temp);
    w[B_ALPINE] = bare.max(cold * smoothstep(0.0, 0.3, slope + 0.15));

    // The Whittaker body: temperature × moisture.
    let warm = smoothstep(-1.0, 15.0, temp);
    let wet = moist;
    let land = (1.0 - w[B_WATER]).max(0.0) * (1.0 - w[B_ALPINE]).max(0.0);
    w[B_DESERT] = land * warm * (1.0 - smoothstep(0.16, 0.42, wet));
    w[B_GRASSLAND] = land * warm * bump(wet, 0.30, 0.60) * (1.0 - smoothstep(0.55, 0.85, wet));
    w[B_SHRUBLAND] = land * bump(wet, 0.22, 0.50) * (1.0 - warm * 0.4);
    w[B_FOREST] = land * warm * smoothstep(0.48, 0.78, wet);
    w[B_TAIGA] = land * (1.0 - warm) * smoothstep(0.30, 0.62, wet);

    // Normalise — weights, always summing to one.
    let sum: f32 = w.iter().sum();
    if sum > 1e-5 {
        for v in w.iter_mut() {
            *v /= sum;
        }
    } else {
        w[B_GRASSLAND] = 1.0;
    }
    w
}

#[inline]
fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    if (b - a).abs() < 1e-6 {
        return if x >= b { 1.0 } else { 0.0 };
    }
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A soft band: 1 in the middle of `[a,b]`, falling off outside it.
#[inline]
fn bump(x: f32, a: f32, b: f32) -> f32 {
    smoothstep(a - 0.15, a + 0.05, x) * (1.0 - smoothstep(b - 0.05, b + 0.20, x))
}

// ─── preview ──────────────────────────────────────────────────────────────

/// Debug colour for a cell: biome tint, shaded by the sun and darkened where
/// water runs. Not the renderer's material — this exists so a landscape can be
/// judged as a PNG in a second, without a browser or a GPU.
pub fn preview_rgb(f: &Field, x: usize, y: usize, r: &TerrainRecipe) -> [u8; 3] {
    let sed_bare = (f.sediment[f.idx(x, y)] / 6.0).clamp(0.0, 0.55);
    const TINT: [[f32; 3]; 8] = [
        [0.13, 0.30, 0.48], // water
        [0.80, 0.74, 0.55], // beach
        [0.78, 0.68, 0.45], // desert
        [0.52, 0.63, 0.30], // grassland
        [0.56, 0.53, 0.31], // shrubland
        [0.17, 0.34, 0.16], // forest
        [0.20, 0.31, 0.26], // taiga
        [0.55, 0.54, 0.52], // alpine rock
    ];
    let base = f.idx(x, y) * BIOMES.len();
    let mut c = [0.0f32; 3];
    for b in 0..BIOMES.len() {
        let w = f.biome[base + b];
        for k in 0..3 {
            c[k] += TINT[b][k] * w;
        }
    }
    // Loose scree and washed sediment read as bare ground, not lawn.
    for k in 0..3 {
        c[k] = c[k] * (1.0 - sed_bare) + [0.62, 0.56, 0.45][k] * sed_bare;
    }

    // Hillshade from the height gradient — a sun in the north-west, which is
    // the convention every map reader already has in their eye.
    let i = f.idx(x, y);
    let xm = x.saturating_sub(1);
    let xp = (x + 1).min(f.size - 1);
    let ym = y.saturating_sub(1);
    let yp = (y + 1).min(f.size - 1);
    let dx = (f.height[y * f.size + xp] - f.height[y * f.size + xm]) / (2.0 * f.cell_size);
    let dy = (f.height[yp * f.size + x] - f.height[ym * f.size + x]) / (2.0 * f.cell_size);
    let n = [-dx, 1.0, -dy];
    let nl = (n[0] * n[0] + 1.0 + n[2] * n[2]).sqrt();
    let l = [-0.5, 0.75, -0.43];
    let shade = ((n[0] * l[0] + n[1] * l[1] + n[2] * l[2]) / nl).clamp(0.15, 1.0);

    // Snow above the freezing line, on anything that isn't a wall.
    let snow = if f.temperature[i] < -2.0 && f.slope[i] < 0.55 { 0.75 } else { 0.0 };
    for k in 0..3 {
        c[k] = c[k] * (1.0 - snow) + 0.92 * snow;
        c[k] *= shade;
    }
    // Deep water reads deeper.
    if f.height[i] < r.sea_level {
        let d = ((r.sea_level - f.height[i]) / 120.0).clamp(0.0, 1.0);
        for k in 0..3 {
            c[k] *= 1.0 - d * 0.35;
        }
    }
    [
        (c[0].clamp(0.0, 1.0) * 255.0) as u8,
        (c[1].clamp(0.0, 1.0) * 255.0) as u8,
        (c[2].clamp(0.0, 1.0) * 255.0) as u8,
    ]
}

/// Render a field to an RGB8 buffer for eyeballing.
pub fn preview(f: &Field, r: &TerrainRecipe) -> Vec<u8> {
    let mut px = Vec::with_capacity(f.size * f.size * 3);
    for y in 0..f.size {
        for x in 0..f.size {
            px.extend_from_slice(&preview_rgb(f, x, y, r));
        }
    }
    px
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(relief: Relief, size: usize) -> (Field, TerrainRecipe) {
        let r = TerrainRecipe { relief, cell_size: 8.0, ..Default::default() };
        (generate(&r, size, 0.0, 0.0), r)
    }

    /// Every field is finite and the biome weights are a partition. A NaN here
    /// becomes a hole in the mesh a hundred lines later.
    #[test]
    fn fields_are_finite_and_biomes_partition() {
        let (f, _) = tile(Relief::Hills, 64);
        assert!(f.height.iter().all(|v| v.is_finite()), "height has NaN/inf");
        assert!(f.flow.iter().all(|v| v.is_finite() && *v >= 0.0));
        assert!(f.moisture.iter().all(|v| (0.0..=1.0).contains(v)));
        for c in 0..f.size * f.size {
            let sum: f32 = f.biome[c * BIOMES.len()..(c + 1) * BIOMES.len()].iter().sum();
            assert!((sum - 1.0).abs() < 1e-3, "biome weights sum to {sum}, not 1");
        }
    }

    /// The same recipe must produce the same world on every machine, twice.
    /// Determinism is what lets a tile be cached, and lets two travelers stand
    /// on the same hill.
    #[test]
    fn generation_is_deterministic() {
        let (a, _) = tile(Relief::Alpine, 48);
        let (b, _) = tile(Relief::Alpine, 48);
        assert_eq!(a.height, b.height);
        assert_eq!(a.biome, b.biome);
    }

    /// Erosion must actually erode: a droplet run has to cut the landscape
    /// somewhere, or the whole realism argument is decoration.
    #[test]
    fn erosion_carves_and_deposits() {
        let r = TerrainRecipe { relief: Relief::Alpine, cell_size: 8.0, ..Default::default() };
        let size = 96usize;
        let mut raw = vec![0.0f32; size * size];
        for y in 0..size {
            for x in 0..size {
                raw[y * size + x] = uplift(x as f32 * 8.0, y as f32 * 8.0, &r);
            }
        }
        let mut eroded = raw.clone();
        let mut sed = vec![0.0f32; size * size];
        hydraulic(&mut eroded, &mut sed, size, 8.0, size * size, r.seed);
        let cut = raw.iter().zip(&eroded).filter(|(a, b)| **a - **b > 0.5).count();
        assert!(cut > size * size / 100, "erosion barely cut anything ({cut} cells)");
        assert!(sed.iter().any(|s| *s > 0.0), "nothing was ever deposited");
    }

    /// Flow must concentrate: most cells drain a little, a few drain a lot.
    /// A flat histogram would mean no river network formed.
    #[test]
    fn flow_forms_channels() {
        let (f, _) = tile(Relief::Hills, 96);
        let max = f.flow.iter().cloned().fold(0.0f32, f32::max);
        let mean = f.flow.iter().sum::<f32>() / f.flow.len() as f32;
        assert!(max > mean * 20.0, "flow never concentrated (max {max}, mean {mean})");
    }

    /// The rain shadow: with a west wind, the eastern (lee) side of a range
    /// must end up drier than the western (windward) side.
    #[test]
    fn wind_casts_a_rain_shadow() {
        let r = TerrainRecipe {
            relief: Relief::Alpine,
            wind: [1.0, 0.0],
            humidity: 0.75,
            cell_size: 10.0,
            ..Default::default()
        };
        let f = generate(&r, 96, 0.0, 0.0);
        // Compare the wettest windward quarter against the same lee quarter,
        // row by row, so a single wet valley cannot carry the result.
        let (mut windward, mut lee, mut rows) = (0.0f32, 0.0f32, 0);
        for y in 0..f.size {
            let mut peak = 0usize;
            for x in 0..f.size {
                if f.height[f.idx(x, y)] > f.height[f.idx(peak, y)] {
                    peak = x;
                }
            }
            if peak < 8 || peak + 8 >= f.size {
                continue;
            }
            windward += f.moisture[f.idx(peak - 6, y)];
            lee += f.moisture[f.idx(peak + 6, y)];
            rows += 1;
        }
        assert!(rows > 10, "not enough usable rows ({rows})");
        assert!(
            windward > lee,
            "no rain shadow: windward {windward:.2} vs lee {lee:.2} over {rows} rows"
        );
    }

    /// Relief presets must actually differ in drama, or the knob is a lie.
    #[test]
    fn relief_presets_change_the_range() {
        let relief_of = |r: Relief| {
            let (f, _) = tile(r, 64);
            let lo = f.height.iter().cloned().fold(f32::MAX, f32::min);
            let hi = f.height.iter().cloned().fold(f32::MIN, f32::max);
            hi - lo
        };
        let plains = relief_of(Relief::Plains);
        let alpine = relief_of(Relief::Alpine);
        assert!(alpine > plains * 3.0, "alpine {alpine:.0} vs plains {plains:.0}");
    }

    #[test]
    fn unknown_relief_falls_back_to_hills() {
        assert_eq!(Relief::from_str("nonsense"), Relief::Hills);
        assert_eq!(Relief::from_str("ALPINE"), Relief::Alpine);
    }
}

// ─── meshing ──────────────────────────────────────────────────────────────

/// Ground albedo at a cell, for the mesh's vertex colours.
///
/// Separate from [`preview_rgb`] on purpose: the preview is a *map*, shaded
/// and tinted to be read from above, while this is the actual colour of the
/// ground you stand on. Every PBR path here multiplies vertex colour into
/// albedo, so a terrain arrives fully dressed without one new shader — the
/// splat maps and triplanar rock refine it later, they do not replace it.
pub fn ground_albedo(f: &Field, x: usize, y: usize, r: &TerrainRecipe) -> [f32; 3] {
    // Ground tints, not map tints: what soil, sand and rock look like underfoot.
    const TINT: [[f32; 3]; 8] = [
        [0.09, 0.16, 0.20], // water — the bed seen through it
        [0.76, 0.70, 0.54], // beach sand
        [0.72, 0.61, 0.42], // desert
        [0.33, 0.42, 0.18], // grassland
        [0.40, 0.39, 0.22], // shrubland
        [0.16, 0.26, 0.13], // forest floor
        [0.19, 0.26, 0.21], // taiga
        [0.44, 0.43, 0.41], // bare rock
    ];
    let i = f.idx(x, y);
    let base = i * BIOMES.len();
    let mut c = [0.0f32; 3];
    for b in 0..BIOMES.len() {
        let w = f.biome[base + b];
        for k in 0..3 {
            c[k] += TINT[b][k] * w;
        }
    }

    // Loose material over the top: scree, silt, river sand.
    let sed = (f.sediment[i] / 5.0).clamp(0.0, 0.5);
    for k in 0..3 {
        c[k] = c[k] * (1.0 - sed) + [0.58, 0.52, 0.42][k] * sed;
    }

    // Snow, faded in over a band rather than switched on — a hard freezing
    // line draws a contour on the mountain that no snowfall ever drew.
    let snow = smoothstep(0.0, -5.0, f.temperature[i]) * (1.0 - smoothstep(0.45, 0.75, f.slope[i]));
    for k in 0..3 {
        c[k] = c[k] * (1.0 - snow) + [0.92, 0.94, 0.97][k] * snow;
    }

    // Per-cell variation so a hillside is never one flat wash. Small, and
    // keyed to position so it is stable across regenerations.
    let v = 1.0 + value_noise(x as f32 * 0.37, y as f32 * 0.37, r.seed ^ 0xa53f) * 0.07;
    [c[0] * v, c[1] * v, c[2] * v]
}

/// Turn a field into a renderable tile.
///
/// `lod` is a power-of-two stride: 0 renders every cell, 1 every other, and so
/// on, which is what lets a distant tile cost a sixteenth of a near one while
/// staying the same shape.
///
/// `skirt` drops a curtain of geometry around the tile's rim. Neighbouring
/// tiles at different LODs disagree about the height along their shared edge
/// by a few centimetres, and that gap is a crack you can see the sky through —
/// a skirt hides it for the cost of one quad strip, which is the oldest trick
/// in terrain rendering and still the right one.
pub fn mesh(f: &Field, r: &TerrainRecipe, lod: u32, skirt: f32) -> crate::MeshData {
    let step = 1usize << lod.min(5);
    let n = ((f.size - 1) / step) + 1; // vertices per side
    let mut m = crate::MeshData::default();
    m.positions.reserve(n * n);

    let at = |i: usize| (i * step).min(f.size - 1);
    // Normals come from the height field's own gradient rather than from
    // averaged face normals: it is cheaper, and it stays smooth across an LOD
    // change instead of visibly re-faceting when the stride doubles.
    let normal_at = |cx: usize, cy: usize| {
        let xm = cx.saturating_sub(step);
        let xp = (cx + step).min(f.size - 1);
        let ym = cy.saturating_sub(step);
        let yp = (cy + step).min(f.size - 1);
        let dx = (f.height[cy * f.size + xp] - f.height[cy * f.size + xm])
            / ((xp - xm).max(1) as f32 * f.cell_size);
        let dz = (f.height[yp * f.size + cx] - f.height[ym * f.size + cx])
            / ((yp - ym).max(1) as f32 * f.cell_size);
        let n = [-dx, 1.0, -dz];
        let l = (n[0] * n[0] + 1.0 + n[2] * n[2]).sqrt();
        [n[0] / l, n[1] / l, n[2] / l]
    };

    for iy in 0..n {
        for ix in 0..n {
            let (cx, cy) = (at(ix), at(iy));
            let i = f.idx(cx, cy);
            m.positions.push([
                cx as f32 * f.cell_size,
                f.height[i],
                cy as f32 * f.cell_size,
            ]);
            m.normals.push(normal_at(cx, cy));
            // UVs in metres, so a material recipe tiles at a real-world scale
            // and does not stretch when the tile's resolution changes.
            m.uvs.push([cx as f32 * f.cell_size, cy as f32 * f.cell_size]);
            let c = ground_albedo(f, cx, cy, r);
            m.colors.push([c[0], c[1], c[2], 1.0]);
        }
    }
    for iy in 0..n.saturating_sub(1) {
        for ix in 0..n.saturating_sub(1) {
            let a = (iy * n + ix) as u32;
            let b = a + 1;
            let c = a + n as u32;
            let d = c + 1;
            // Counter-clockwise seen from above — the winding every builtin
            // here uses, and the one the guard test measures.
            m.indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }

    if skirt > 0.0 && n >= 2 {
        add_skirt(&mut m, n, skirt);
    }
    m.tangents = vec![[1.0, 0.0, 0.0, 1.0]; m.positions.len()];
    m
}

/// Hang a curtain from the tile's four edges.
fn add_skirt(m: &mut crate::MeshData, n: usize, drop: f32) {
    let rim: Vec<u32> = {
        let mut v = Vec::with_capacity(n * 4);
        for ix in 0..n {
            v.push(ix as u32); // north
        }
        for iy in 1..n {
            v.push((iy * n + n - 1) as u32); // east
        }
        for ix in (0..n - 1).rev() {
            v.push(((n - 1) * n + ix) as u32); // south
        }
        for iy in (1..n - 1).rev() {
            v.push((iy * n) as u32); // west
        }
        v
    };
    let base = m.positions.len() as u32;
    for &i in &rim {
        let p = m.positions[i as usize];
        m.positions.push([p[0], p[1] - drop, p[2]]);
        // Outward-ish and level: a skirt lit like the ground above it reads as
        // shadow rather than as a wall that appeared out of nowhere.
        m.normals.push(m.normals[i as usize]);
        m.uvs.push(m.uvs[i as usize]);
        m.colors.push(m.colors[i as usize]);
    }
    for k in 0..rim.len() {
        let k2 = (k + 1) % rim.len();
        let (t0, t1) = (rim[k], rim[k2]);
        let (b0, b1) = (base + k as u32, base + k2 as u32);
        m.indices.extend_from_slice(&[t0, b0, t1, t1, b0, b1]);
    }
}

#[cfg(test)]
mod mesh_tests {
    use super::*;

    fn field() -> (Field, TerrainRecipe) {
        let r = TerrainRecipe { relief: Relief::Hills, cell_size: 8.0, ..Default::default() };
        (generate(&r, 65, 0.0, 0.0), r)
    }

    #[test]
    fn mesh_is_well_formed_and_finite() {
        let (f, r) = field();
        let m = mesh(&f, &r, 0, 0.0);
        assert_eq!(m.positions.len(), 65 * 65);
        assert_eq!(m.normals.len(), m.positions.len());
        assert_eq!(m.colors.len(), m.positions.len());
        assert_eq!(m.indices.len(), 64 * 64 * 6);
        assert!(m.positions.iter().flatten().all(|v| v.is_finite()));
        assert!(m.indices.iter().all(|i| (*i as usize) < m.positions.len()));
        // Normals point up-ish: this is ground, not a ceiling.
        assert!(m.normals.iter().all(|n| n[1] > 0.0), "a normal pointed downward");
    }

    /// The mesh must sit exactly on the field the scatter will sample, or
    /// every tree floats or sinks.
    #[test]
    fn vertices_sit_on_the_sampled_height() {
        let (f, r) = field();
        let m = mesh(&f, &r, 0, 0.0);
        for (k, p) in m.positions.iter().enumerate() {
            let (x, y) = (k % 65, k / 65);
            assert!((p[1] - f.height_at(x as f32, y as f32)).abs() < 1e-3);
        }
    }

    /// Each LOD level halves the resolution and keeps the same footprint —
    /// that is what makes a distant tile cheap without changing its shape.
    #[test]
    fn lod_halves_resolution_and_keeps_extent() {
        let (f, r) = field();
        let full = mesh(&f, &r, 0, 0.0);
        let half = mesh(&f, &r, 1, 0.0);
        assert_eq!(half.positions.len(), 33 * 33);
        let extent = |m: &crate::MeshData| {
            let xs: Vec<f32> = m.positions.iter().map(|p| p[0]).collect();
            xs.iter().cloned().fold(f32::MIN, f32::max) - xs.iter().cloned().fold(f32::MAX, f32::min)
        };
        assert!((extent(&full) - extent(&half)).abs() < 1e-3, "LOD changed the footprint");
    }

    /// A skirt adds a rim of geometry that hangs BELOW the surface — its whole
    /// job is to be underground where the crack would be.
    #[test]
    fn skirt_hangs_below_the_rim() {
        let (f, r) = field();
        let bare = mesh(&f, &r, 1, 0.0);
        let with = mesh(&f, &r, 1, 12.0);
        assert!(with.positions.len() > bare.positions.len());
        let lowest_bare = bare.positions.iter().map(|p| p[1]).fold(f32::MAX, f32::min);
        let lowest_with = with.positions.iter().map(|p| p[1]).fold(f32::MAX, f32::min);
        assert!(lowest_with < lowest_bare - 1.0, "the skirt did not hang below");
        assert!(with.indices.iter().all(|i| (*i as usize) < with.positions.len()));
    }

    /// Ground colour must actually vary with the land — a terrain that returns
    /// one colour everywhere is a painted plane.
    #[test]
    fn ground_albedo_tracks_the_landscape() {
        let r = TerrainRecipe { relief: Relief::Alpine, cell_size: 10.0, ..Default::default() };
        let f = generate(&r, 96, 0.0, 0.0);
        let mut lo = [f32::MAX; 3];
        let mut hi = [f32::MIN; 3];
        for y in 0..f.size {
            for x in 0..f.size {
                let c = ground_albedo(&f, x, y, &r);
                for k in 0..3 {
                    lo[k] = lo[k].min(c[k]);
                    hi[k] = hi[k].max(c[k]);
                }
            }
        }
        let spread: f32 = (0..3).map(|k| hi[k] - lo[k]).sum();
        assert!(spread > 0.5, "ground colour barely varied (spread {spread:.2})");
        assert!((0..3).all(|k| lo[k] >= 0.0 && hi[k] <= 1.2), "albedo out of range");
    }
}

// ─── the manifest bridge ──────────────────────────────────────────────────

impl TerrainRecipe {
    /// Read a manifest's `environment.terrain` block.
    ///
    /// Every field has a default and an unknown relief falls back to hills, so
    /// `"terrain": {}` is a legal, working landscape — the smallest thing an
    /// author can write and still get ground.
    pub fn from_manifest(t: &infinite_manifest::Terrain) -> Self {
        Self {
            seed: t.seed,
            relief: Relief::from_str(&t.relief),
            sea_level: t.sea_level,
            latitude: t.latitude.clamp(0.0, 1.0),
            wind: t.wind,
            humidity: t.humidity.clamp(0.0, 1.0),
            // A cell smaller than a footstep buys nothing and costs the square
            // of it; larger than a house and hills stop reading as hills.
            cell_size: t.cell_size.clamp(0.5, 64.0),
        }
    }
}

#[cfg(test)]
mod manifest_tests {
    use super::*;

    #[test]
    fn an_empty_terrain_block_is_a_working_landscape() {
        let t = infinite_manifest::Terrain::default();
        let r = TerrainRecipe::from_manifest(&t);
        assert_eq!(r.relief, Relief::Hills);
        let f = generate(&r, 48, 0.0, 0.0);
        assert!(f.height.iter().all(|v| v.is_finite()));
    }

    #[test]
    fn the_recipe_survives_a_json_round_trip() {
        let json = r#"{
            "seed": 99, "relief": "alpine", "sea_level": -12.5,
            "latitude": 0.7, "wind": [0.0, 1.0], "humidity": 0.9,
            "cell_size": 6.0, "cover": { "trees": 0.4 },
            "unknown_future_field": 7
        }"#;
        let t: infinite_manifest::Terrain = serde_json::from_str(json).unwrap();
        let r = TerrainRecipe::from_manifest(&t);
        assert_eq!(r.seed, 99);
        assert_eq!(r.relief, Relief::Alpine);
        assert_eq!(r.wind, [0.0, 1.0]);
        assert!((r.sea_level + 12.5).abs() < 1e-6);
        assert!((t.cover.trees - 0.4).abs() < 1e-6);
        assert!((t.cover.grass - 1.0).abs() < 1e-6, "an unset cover defaults to full");
        // Forward compatibility: a field from a later version must survive.
        let back = serde_json::to_string(&t).unwrap();
        assert!(back.contains("unknown_future_field"), "unknown field was dropped");
    }

    /// Absurd inputs must produce a landscape, not a panic or a wall.
    #[test]
    fn hostile_values_are_clamped_not_obeyed() {
        let json = r#"{ "cell_size": 0.0001, "latitude": 40.0, "humidity": -3.0 }"#;
        let t: infinite_manifest::Terrain = serde_json::from_str(json).unwrap();
        let r = TerrainRecipe::from_manifest(&t);
        assert!(r.cell_size >= 0.5, "a sub-millimetre cell would hang the generator");
        assert!((0.0..=1.0).contains(&r.latitude));
        assert!((0.0..=1.0).contains(&r.humidity));
        let f = generate(&r, 32, 0.0, 0.0);
        assert!(f.height.iter().all(|v| v.is_finite()));
    }
}
