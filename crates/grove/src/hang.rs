//! Hang things at sockets — fruit, lanterns, leaf clusters, props.
//!
//! A grown tree ends in sockets (position, direction, radius at every
//! terminal tip). Hanging is the first composition: pick some of those tips
//! and place one thing at each, so a lantern tree carries its lanterns
//! exactly where its branches end and the fruit is a separate, reusable
//! model (Trellis-made, carved, or grown) instanced once per tip. The
//! recipe is rules — how many, how deep in the crown, how big, how far the
//! stem drops — and the same seed picks the same tips everywhere.
use serde::{Deserialize, Serialize};

use crate::grow::Socket;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HangRecipe {
    /// How many sockets get one (0 = every eligible socket).
    pub count: u32,
    /// Only tips at this branching generation or deeper.
    pub min_level: u32,
    /// Uniform scale of the hung thing.
    pub scale: f32,
    /// ± fraction of random scale per instance.
    pub scale_jitter: f32,
    /// Stem: metres between the tip and the top of the hung thing.
    pub drop: f32,
    /// Random yaw per instance (a hanging thing turns freely).
    pub spin: bool,
    pub seed: u32,
}

impl Default for HangRecipe {
    fn default() -> Self {
        Self { count: 0, min_level: 0, scale: 1.0, scale_jitter: 0.15, drop: 0.15, spin: true, seed: 1 }
    }
}

/// One placed instance (TRS, glTF conventions; rotation is `[x y z w]`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Placement {
    pub socket: String,
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
}

fn rnd(state: &mut u32) -> f32 {
    *state = state.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    let mut x = *state;
    x = ((x >> ((x >> 28).wrapping_add(4))) ^ x).wrapping_mul(277_803_737);
    x = (x >> 22) ^ x;
    (x as f32) / (u32::MAX as f32)
}

/// Place one thing per chosen socket, hanging straight down from the tip.
/// `top_y` is the hung model's own top (its bounds max Y), so its top sits
/// `drop` metres under the tip whatever its origin is.
pub fn hang(sockets: &[Socket], top_y: f32, r: &HangRecipe) -> Vec<Placement> {
    let mut rng = r.seed.wrapping_mul(2_246_822_519).wrapping_add(7);
    let mut pool: Vec<&Socket> = sockets.iter().filter(|s| s.level >= r.min_level).collect();
    // Seeded shuffle, then take `count`.
    for i in (1..pool.len()).rev() {
        let j = (rnd(&mut rng) * (i as f32 + 1.0)) as usize;
        pool.swap(i, j.min(i));
    }
    if r.count > 0 {
        pool.truncate(r.count as usize);
    }
    pool.iter()
        .map(|s| {
            let sc = r.scale * (1.0 + (rnd(&mut rng) * 2.0 - 1.0) * r.scale_jitter);
            let yaw = if r.spin { rnd(&mut rng) * std::f32::consts::TAU } else { 0.0 };
            let (sy, cy) = (yaw * 0.5).sin_cos();
            Placement {
                socket: s.name.clone(),
                translation: [s.position[0], s.position[1] - r.drop - top_y * sc, s.position[2]],
                rotation: [0.0, sy, 0.0, cy],
                scale: [sc, sc, sc],
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sockets(n: usize) -> Vec<Socket> {
        (0..n)
            .map(|i| Socket { name: format!("tip-{i}"), position: [i as f32, 3.0, 0.0], direction: [0.0, 1.0, 0.0], radius: 0.01, level: (i % 3) as u32 })
            .collect()
    }

    #[test]
    fn picks_count_and_hangs_below_the_tip() {
        let s = sockets(30);
        let r = HangRecipe { count: 9, min_level: 2, scale: 0.5, drop: 0.2, ..Default::default() };
        let p = hang(&s, 0.4, &r);
        assert_eq!(p.len(), 9);
        for pl in &p {
            let src = s.iter().find(|x| x.name == pl.socket).unwrap();
            assert!(src.level >= 2);
            // top of the thing = translation.y + top_y*scale = tip.y - drop
            let top = pl.translation[1] + 0.4 * pl.scale[1];
            assert!((top - (3.0 - 0.2)).abs() < 1e-4);
        }
        let again = hang(&s, 0.4, &r);
        assert_eq!(p.iter().map(|p| &p.socket).collect::<Vec<_>>(), again.iter().map(|p| &p.socket).collect::<Vec<_>>());
    }
}
