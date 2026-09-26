# Grove — the roadmap

Agreed with the founder 2026-09-26. Grove is the Thread's plant grower:
everything that grows lives here, and Chisel carves the inorganic, Avatar
carries the humanoid. This document is the brief for the session that
builds it. Read `src/grow.rs`, `src/foliage.rs`, `src/hang.rs` and
`recipes/README.md` first; they are the state of play.

## The idea

A plant is not a mesh. It is **a species, a seed and a clock**, and the mesh
is derived from those three whenever anyone needs it — in Unity, in Thread,
in the Quarry — and always comes out the same.

- **Species** is the rule set: branching, where leaves, blooms and fruit
  attach, bark and leaf materials, wind character, life curve. Today this is
  `GrowRecipe`. It becomes the stable identity the Quarry stores.
- **Seed** is one number. It must decide the *whole potential plant up
  front* — every branch it could ever have — so that age only chooses which
  branches have emerged and how far they have grown. Today the random stream
  is consumed as the tree grows, so gating by age would reshuffle the tree.
  This refactor comes first; everything else stands on it.
- **Clock** is two values: **age** in seasons since sprouting (sapling,
  mature, old, dying) and **position in the year** (bud, leaf, bloom, fruit,
  seed drop, bare). Time is ticks the game controls, not wall-clock days.
- **Events** are the third input. The Lantern Desert is the proof case: the
  withered lantern tree is not a second recipe but the lantern species in a
  `withered` state — leaves off, fruit dropped, gnarl up, bark darkened —
  same seed, same tree, both halves of the map.

## Harvest, pick and chop are state, not meshes

- Every attachment ends at a **socket**. Sockets get kinds: `tip`, `bloom`,
  `fruit`, `cut`. Picking records the socket as taken with a regrow time;
  the only mesh change is one instance disappearing.
- Chopping needs the mesh to know its branches: every vertex carries its
  **branch id** in a spare channel so Unity can hit-test a swing without
  asking anyone. The cut is recorded as state; the fallen part is derived as
  the subtree grown on its own. **Cut at branch joints only.**
- A plant's state is therefore a short list — taken sockets, cut branches,
  health events — small enough to sync in a game and to sit in a World
  Manifest placement: `{ species, seed, age, season, state }`.

## Looking great in Unity and Thread

- **Wind**: four vertex channels — trunk sway weight, branch sway weight,
  leaf flutter, per-branch phase — replacing today's single alpha. The wood
  already knows its hierarchy. One Shader Graph in `com.pixygon.quarry`
  reads it (TERRA.md §6.3 stays the convention for alpha = rigidity).
- **Textures**: bark is recipe-baked already. Leaves need a leaf recipe with
  veins and translucency. Blooms and fruit come from Trellis, a carve or a
  recipe, hung at sockets — three suppliers, one pipeline.
- **LODs** exist; the missing last step is an **impostor**: the previewer
  already renders turntables, so eight views become a cross-quad billboard.
- **Runtime cost**: a forest never derives a mesh per plant. Per species,
  per life stage, a handful of individuals (SpeedTree's way); per-plant
  state is applied on top by hiding branches and instances.

## Build order

1. **Species / seed / clock split.** ✅ *landed 2026-09-26 — see "Where it
   stands".* The seed decides the full potential graph; age filters and
   scales. `grow(species, seed, clock)`. The Quarry's design id stays the
   species (it already hashes only non-default fields — keep it that way,
   never let a schema default reach the hash).
2. **Life stages and the year**, typed sockets for bloom and fruit, and the
   lantern tree's `withered` event state as the test. Turntable next to the
   concept before it is called done (`thread grow --preview`).
3. **Branch ids in the mesh, cut state, subtree derivation** for the fallen
   part. Joints only.
4. **Wind channels, the Unity shader, impostors.**
5. **Species from the Codex**: prose in the Codex → an agent drafts species
   rules → the turntable decides.

## Ground rules for the session

- thread-engine is not a pearl: commit by path, push, no release. The Quarry
  (`~/repos/quarry`) vendors these crates with `scripts/sync-vendor.sh`, does
  not redeploy on push (trigger Coolify by uuid), and **wipes its store on
  every redeploy** until the founder mounts persistent storage — republish
  `recipes/*.grow.json` after each deploy. Tokens come from
  `~/.config/dyson-swarm/config.toml` into the environment, never printed.
- Every visual claim is a turntable next to the concept. The previewer
  washes strong emissive to white; Unity is the judge for glow.
- Same seed, same plant, everywhere — a test for every generator.

## Where it stands

**Step 1 is in.** The refactor it stood on was the randomness: Grove consumed
one stream as the tree grew, so gating by age reshuffled everything that
remained. Randomness is **addressed** now (`src/rand.rs`) — every value is
`hash(seed, branch key, slot)`, and a branch's key is hashed from its path
through the plant, never from the order anything was built in. Nothing is
consumed, so nothing can drift.

On that: `potential()` builds the whole plant the seed decided, with no
knowledge of age at all, and `at_clock()` is the clock's entire job — drop the
branches that have not emerged, cut each remaining one where it has grown to,
scale the plant by its age. A branch emerges when its parent has grown past the
point it sprouts from, so the joints are never loose, and both growth curves
reach 1 at full size, which makes the grown plant *exactly* the potential plant.

What else came with it:

- `Species` is the rule set and the identity; the seed left it, and a
  a `Clock` (`age` in seasons, `season` in the year) joined it. `Planting`
  is the three together — what a `.grow.json` file holds, flat, and what
  travels to the Quarry.
- Sockets are named after the branch that ends there (`tip-<branch id>`) and
  carry it, so state can name a tip and mean it. A tip is now a branch with no
  children *yet*, so a sapling has sockets at the ends it actually has. Step 3
  gets its ids for free.
- Hanging picks per tip, not per list: a socket scores itself from its branch
  id, so a fruit picked or a branch cut leaves every other fruit alone.
- Leaf clusters are keyed the same way, so a coarser LOD drops leaves instead
  of growing a different crown.
- `thread grow --life sheet.png` renders six ages in one frame at one scale
  (`chisel::preview` gained a `fill` knob for it). Framing each age to fill its
  own tile hides the one thing age does — this is the proof sheet for every
  species from here on.
- `thread grow --age <seasons> --season <0..1>` on top of `--seed`.

**Ground truth for the next session:** `cargo test --workspace` is green (34
in Grove). The three recipes are unchanged files and still read as themselves
— they are different *individuals* than before the refactor, because the
addresses changed; the species did not.

**One thing the Quarry needs at the next `sync-vendor.sh`:** `grove::grow`
takes three arguments now. `src/entry.rs` should read the submission with
`grove::Planting::from_value(&value)?` and call `grove::grow_planting(&p)?`,
and `canonical_recipe` should strip against `grove::Planting::default().to_value()`
instead of `GrowRecipe::default()`. That keeps `seed` (and now `age`) inside the
design id, so nothing that is already published forks. `GrowRecipe` still names
the species, so the type in that file keeps resolving.
