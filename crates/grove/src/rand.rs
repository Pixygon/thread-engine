//! Addressed randomness — the reason one seed can decide a whole plant.
//!
//! A stream is the wrong shape for a plant. Grove used to consume one PCG
//! stream as the tree grew, so the numbers a twig got depended on how many
//! branches had been drawn before it. That is fine for growing one fixed tree
//! and fatal for a clock: hide the branches a sapling has not sprouted yet and
//! every branch that remains gets different numbers — the whole tree
//! reshuffles instead of ageing.
//!
//! So randomness here is *addressed*, never consumed. There is no stream and
//! no state: every value is `hash(seed, key, slot)`, where the **key** is the
//! thing asking (a branch, a leaf site, a tip — its identity, hashed from its
//! path through the plant) and the **slot** is what the number is for. Ask for
//! the same address from anywhere, at any age, at any level of detail, in any
//! order, and the answer is the same. That is `same seed, same plant,
//! everywhere`, made structural instead of hoped for.

/// One deterministic value at an address.
///
/// Not cryptography — a geometry hash: cheap, and mixed well enough that
/// neighbouring keys and slots look unrelated.
pub fn hash(seed: u32, key: u32, slot: u32) -> u32 {
    let mut x = seed.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    x ^= key.wrapping_mul(2_654_435_761);
    x = x.wrapping_mul(1_597_334_677);
    x ^= slot.wrapping_mul(374_761_393).wrapping_add(0x9E37_79B9);
    // PCG-style output permutation: the bits get mixed, not merely shifted.
    let y = ((x >> ((x >> 28).wrapping_add(4))) ^ x).wrapping_mul(277_803_737);
    (y >> 22) ^ y
}

/// The value at an address as 0..1.
pub fn unit(seed: u32, key: u32, slot: u32) -> f32 {
    (hash(seed, key, slot) as f32) / (u32::MAX as f32)
}

/// A branch's identity, hashed from its parent's: path, not order. `kind`
/// separates a fork from a lateral so the two never collide, and `index` says
/// which child it is. Nothing in a key depends on how much of the plant has
/// been built, so a branch keeps its key as the plant ages and as coarser
/// LODs drop generations.
pub fn child_key(parent: u32, kind: u32, index: u32) -> u32 {
    hash(parent, kind.wrapping_mul(0x0501_1A1B) ^ 0xB17E_5EED, index.wrapping_add(1))
}

/// The draws belonging to one key: stateless, so the order they are asked in
/// cannot matter.
#[derive(Debug, Clone, Copy)]
pub struct Rnd {
    seed: u32,
    key: u32,
}

impl Rnd {
    pub fn new(seed: u32, key: u32) -> Self {
        Self { seed, key }
    }

    /// The value at a named slot, 0..1.
    pub fn at(&self, slot: u32) -> f32 {
        unit(self.seed, self.key, slot)
    }

    /// The value at a named slot, -1..1.
    pub fn signed(&self, slot: u32) -> f32 {
        self.at(slot) * 2.0 - 1.0
    }

    /// A smooth -1..1 wander sampled at `t` (0..1) from `controls` values in
    /// slots `slot..slot+controls`. A branch's wander is a curve the seed
    /// decided, not one value per segment: sample it at six points or at sixty
    /// and it is the same wander, so a coarse LOD bends the way LOD0 does.
    pub fn noise(&self, slot: u32, controls: u32, t: f32) -> f32 {
        let c = controls.max(2);
        let f = t.clamp(0.0, 1.0) * (c - 1) as f32;
        let i = (f.floor() as u32).min(c - 2);
        let u = f - i as f32;
        // Smoothstep between control values: no kinks at the joins.
        let w = u * u * (3.0 - 2.0 * u);
        let a = self.signed(slot + i);
        let b = self.signed(slot + i + 1);
        a + (b - a) * w
    }

    /// A key derived from this one for a sub-thing that has no branch of its
    /// own — a leaf site along a twig, a cluster, a socket's contents.
    pub fn sub(&self, kind: u32, index: u32) -> Rnd {
        Rnd { seed: self.seed, key: child_key(self.key, kind, index) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_always_answers_the_same() {
        assert_eq!(hash(7, 42, 3), hash(7, 42, 3));
        assert_ne!(hash(7, 42, 3), hash(7, 42, 4));
        assert_ne!(hash(7, 42, 3), hash(7, 43, 3));
        assert_ne!(hash(7, 42, 3), hash(8, 42, 3));
        let r = Rnd::new(9, 5);
        assert_eq!(r.at(3), unit(9, 5, 3));
        assert!((-1.0..=1.0).contains(&r.signed(3)));
    }

    #[test]
    fn values_spread_over_the_unit_range() {
        let n = 20_000;
        let mut sum = 0.0f64;
        let mut lo = 1.0f32;
        let mut hi = 0.0f32;
        let mut buckets = [0u32; 10];
        for i in 0..n {
            let v = unit(3, i, 1);
            sum += v as f64;
            lo = lo.min(v);
            hi = hi.max(v);
            buckets[((v * 10.0) as usize).min(9)] += 1;
        }
        let mean = sum / n as f64;
        assert!((mean - 0.5).abs() < 0.01, "mean {mean}");
        assert!(lo < 0.001 && hi > 0.999, "range {lo}..{hi}");
        for (i, b) in buckets.iter().enumerate() {
            assert!(*b > n / 20, "bucket {i} thin: {b}");
        }
    }

    #[test]
    fn keys_are_paths_not_order() {
        let root = 1u32;
        let a = child_key(root, 0, 0);
        let b = child_key(root, 0, 1);
        let c = child_key(root, 1, 0);
        assert_ne!(a, b, "siblings differ");
        assert_ne!(a, c, "a fork and a lateral of the same index differ");
        assert_eq!(a, child_key(root, 0, 0));
        assert_ne!(child_key(a, 0, 0), child_key(b, 0, 0), "cousins differ");
        // 4 096 keys three generations deep, all distinct: no collisions in
        // the sizes a plant actually reaches.
        let mut keys: Vec<u32> = Vec::new();
        let mut level = vec![root];
        for _ in 0..3 {
            let mut next = Vec::new();
            for p in level {
                for kind in 0..2 {
                    for i in 0..4 {
                        let k = child_key(p, kind, i);
                        keys.push(k);
                        next.push(k);
                    }
                }
            }
            level = next;
        }
        let n = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), n, "keys collided");
    }

    #[test]
    fn noise_is_a_curve_not_a_sample_count() {
        let r = Rnd::new(5, 77);
        for k in 0..=6 {
            let t = k as f32 / 6.0;
            assert_eq!(r.noise(80, 8, t), r.noise(80, 8, t));
        }
        let mut max = 0.0f32;
        for k in 0..=100 {
            max = max.max(r.noise(80, 8, k as f32 / 100.0).abs());
        }
        assert!(max <= 1.0 && max > 0.2, "noise should wander inside -1..1 (max {max})");
        // Smooth: no jump between neighbouring samples.
        let mut worst = 0.0f32;
        for k in 0..1000 {
            let a = r.noise(80, 8, k as f32 / 1000.0);
            let b = r.noise(80, 8, (k + 1) as f32 / 1000.0);
            worst = worst.max((b - a).abs());
        }
        assert!(worst < 0.05, "noise should be smooth (worst step {worst})");
    }
}
