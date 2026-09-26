//! Hang things at sockets — fruit, lanterns, leaf clusters, props.
//!
//! A grown plant ends in sockets (position, direction, radius, and the branch
//! each tip belongs to). Hanging is the first composition: pick some of those
//! tips and place one thing at each, so a lantern tree carries its lanterns
//! exactly where its branches end and the fruit is a separate, reusable model
//! (Trellis-made, carved, or grown) instanced once per tip.
//!
//! Which tips carry one is **the tip's own business**, not the list's: every
//! socket scores itself from its branch id, and the lowest scores win. A pool
//! that gains or loses tips — a sapling that has not sprouted them yet, a
//! branch cut off, a fruit picked — leaves every other tip's answer alone.
//! That is what makes picking and regrowth state instead of a re-roll.
use serde::{Deserialize, Serialize};

use crate::grow::Socket;
use crate::rand::unit;

/// Slots on a tip's branch id: whether it is chosen, and how the thing sits.
const SLOT_CHOSEN: u32 = 0;
const SLOT_SCALE: u32 = 1;
const SLOT_YAW: u32 = 2;

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

/// Place one thing per chosen socket, hanging straight down from the tip.
/// `top_y` is the hung model's own top (its bounds max Y), so its top sits
/// `drop` metres under the tip whatever its origin is.
pub fn hang(sockets: &[Socket], top_y: f32, r: &HangRecipe) -> Vec<Placement> {
    let mut pool: Vec<(f32, &Socket)> = sockets
        .iter()
        .filter(|s| s.level >= r.min_level)
        .map(|s| (unit(r.seed, s.branch, SLOT_CHOSEN), s))
        .collect();
    // Lowest score first; the name breaks ties so the order is never the
    // order the sockets happened to arrive in.
    pool.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.name.cmp(&b.1.name)));
    if r.count > 0 {
        pool.truncate(r.count as usize);
    }
    pool.iter()
        .map(|(_, s)| {
            let sc = r.scale * (1.0 + (unit(r.seed, s.branch, SLOT_SCALE) * 2.0 - 1.0) * r.scale_jitter);
            let yaw = if r.spin { unit(r.seed, s.branch, SLOT_YAW) * std::f32::consts::TAU } else { 0.0 };
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
            .map(|i| {
                let branch = 0x1000 + i as u32 * 7;
                Socket {
                    name: format!("tip-{branch:08x}"),
                    position: [i as f32, 3.0, 0.0],
                    direction: [0.0, 1.0, 0.0],
                    radius: 0.01,
                    level: (i % 3) as u32,
                    branch,
                }
            })
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
        assert_eq!(
            p.iter().map(|p| &p.socket).collect::<Vec<_>>(),
            again.iter().map(|p| &p.socket).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_tip_decides_for_itself() {
        // The pool changes — tips not yet sprouted, a branch cut off, the list
        // in another order — and every tip that stays keeps its answer.
        let all = sockets(30);
        let r = HangRecipe { count: 0, min_level: 0, ..Default::default() };
        let full = hang(&all, 0.4, &r);
        let mut shuffled = all.clone();
        shuffled.reverse();
        let same = hang(&shuffled, 0.4, &r);
        assert_eq!(
            full.iter().map(|p| &p.socket).collect::<Vec<_>>(),
            same.iter().map(|p| &p.socket).collect::<Vec<_>>(),
            "the order sockets arrive in must not matter"
        );
        // Half the tree missing: the placements that remain are unchanged.
        let half: Vec<Socket> = all.iter().take(15).cloned().collect();
        for pl in hang(&half, 0.4, &r) {
            let was = full.iter().find(|p| p.socket == pl.socket).expect("a tip appeared from nowhere");
            assert_eq!(was.translation, pl.translation);
            assert_eq!(was.scale, pl.scale);
            assert_eq!(was.rotation, pl.rotation);
        }
        // And `count` takes a stable prefix: fewer fruit is a subset, not a re-roll.
        let nine = hang(&all, 0.4, &HangRecipe { count: 9, ..r.clone() });
        let four = hang(&all, 0.4, &HangRecipe { count: 4, ..r.clone() });
        assert_eq!(
            four.iter().map(|p| &p.socket).collect::<Vec<_>>(),
            nine.iter().take(4).map(|p| &p.socket).collect::<Vec<_>>()
        );
    }
}
