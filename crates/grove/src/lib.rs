//! # Grove — everything that grows
//!
//! Chisel carves the inorganic (walls, vases, machines); Avatar carries the
//! humanoid; **Grove grows plants**. A tree, a shrub, a reed bed, a crystal
//! forest are rules, not vertices.
//!
//! A plant here is three things and only three:
//!
//! - a **[species](grow::Species)** — the rule set, and the plant's identity:
//!   how it branches, where leaves attach, what the bark wears, how long it
//!   takes to grow up;
//! - a **seed** — one number, which decides the *whole potential plant*: every
//!   branch it could ever have, already shaped;
//! - a **[clock](clock::Clock)** — `age` in seasons since sprouting and where
//!   the year stands, in ticks the game controls.
//!
//! [`grow(species, seed, clock)`](grow::grow) derives the mesh from those
//! three, and age only ever *filters* (which branches have emerged) and
//! *scales* (how far they have grown). Nothing is consumed as the plant grows,
//! because randomness is [addressed](rand), not streamed — so `{ species,
//! seed, age, season }` is the whole plant, small enough to sync in a game and
//! to sit in a World Manifest placement, and the same plant comes out in Unity,
//! in Thread and in the Quarry.
//!
//! Every plant Grove makes is born game-ready:
//! - **wind** in the vertex-colour alpha (1.0 = rigid at the root, tips sway
//!   most — the engine convention, TERRA §6.3), readable by Infinite and by a
//!   Unity shader from the same channel;
//! - **LODs** from the same seed (coarser sides, fewer generations, thinner
//!   crowns — the same plant, not another one);
//! - **sockets** at every tip, named after the branch that ends there, for
//!   fruit, lanterns, foliage and props.
//!
//! Layers, inside-out: [`rand`] is the addressed randomness everything shares;
//! [`grow`] is the wood (trunk, limbs, twigs); [`foliage`] the leaves,
//! [`hang`] what hangs at the sockets; [`flora`] is where plants stand
//! (species, blue-noise scatter, clumping).
//!
//! Grove leans on Chisel for the substrate — [`chisel::MeshData`], bark
//! texture baking and glb export — and never on its carving vocabulary.
//!
//! **Vendored callers**: `grow` takes three arguments now. A caller that holds
//! a whole recipe file (the Quarry) reads it with [`grow::Planting`] and calls
//! [`grow::grow_planting`].
pub mod clock;
pub mod flora;
pub mod foliage;
pub mod grow;
pub mod hang;
pub mod rand;

pub use clock::Clock;
pub use foliage::{leaves, LeafRecipe, LeafSite};
pub use grow::{grow, grow_planting, GrowRecipe, Grown, Planting, Socket, Species};
pub use hang::{hang, HangRecipe, Placement};
