//! # Grove — everything that grows
//!
//! Chisel carves the inorganic (walls, vases, machines); Avatar carries the
//! humanoid; **Grove grows plants**. A tree, a shrub, a reed bed, a crystal
//! forest are rules, not vertices: a recipe and a seed grow the same
//! individual everywhere, so a plant is Quarry-publishable and re-derivable
//! like any other model.
//!
//! Every plant Grove makes is born game-ready:
//! - **wind** in the vertex-colour alpha (1.0 = rigid at the root, tips sway
//!   most — the engine convention, TERRA §6.3), readable by Infinite and by a
//!   Unity shader from the same channel;
//! - **LODs** from the same recipe (coarser sides, fewer generations);
//! - **sockets** at every terminal tip for fruit, lanterns, foliage and props.
//!
//! Layers, inside-out: [`grow`] is the wood (trunk, limbs, twigs);
//! [`flora`] is where plants stand (species, blue-noise scatter, clumping);
//! foliage and fruit attach at sockets next.
//!
//! Grove leans on Chisel for the substrate — [`chisel::MeshData`], bark
//! texture baking and glb export — and never on its carving vocabulary.
pub mod flora;
pub mod grow;
pub mod hang;

pub use grow::{grow, GrowRecipe, Grown, Socket};
pub use hang::{hang, HangRecipe, Placement};
