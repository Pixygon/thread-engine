# Grove recipes

A plant is **a species, a seed and a clock**, and the mesh is derived from
those three whenever anyone needs it. Each file here is one plant:

- the **species** — every shape, material and life-curve field — which is the
  plant's identity and what the Quarry keys a design on;
- `seed` — which individual. One number, and it decides the *whole potential
  plant*: every branch it could ever have;
- `age` and `season` — the moment. `age` counts seasons since sprouting (ticks
  the game controls, never wall-clock days); leave it out and the file shows
  the grown plant. `season` is 0..1 and wraps: bud, leaf, bloom, fruit, seed
  drop, bare;
- `withered` — what has *happened* to it. A state, not a second species.

```sh
thread grow oak.grow.json -o oak.glb --preview sheet.png   # the turntable
thread grow oak.grow.json --seed 23                        # another individual
thread grow oak.grow.json --age 3                          # a sapling
thread grow oak.grow.json --life life.png                  # six ages, one scale
thread grow oak.grow.json --year year.png                  # one year, one scale
thread grow oak.grow.json --season 0.78                    # turning, about to drop
thread grow lantern-tree.grow.json --withered              # the same tree, dead standing
thread grow lantern-tree.grow.json --hang fruit.glb --hang-kind fruit
thread grow oak.grow.json --publish                        # the Quarry regrows it
```

Age never reshuffles a plant. It **filters** — which branches have emerged —
and **scales** — how far each has grown and how thick it is. Every branch a
sapling has is a branch its grown self has, in the same place, pointing the
same way; `--life` is the proof, because it puts six ages in one frame at one
scale instead of framing each one to fill its own tile.

- `oak.grow.json` — a broadleaf with a two-generation crown (`leaves.depth: 2`),
  blooms in spring and acorns from midsummer.
- `lantern-tree.grow.json` — the Lantern Desert's crystal tree: bare, lifting
  tips (`gravity: -0.16`), veined bark, a little glow. Its lanterns are
  **fruit**: `--hang fruit.glb --hang-kind fruit` hangs one at every fruit
  socket, and out of season there are no fruit sockets, so nothing hangs.
- `withered-tree.grow.json` — **the same species and the same seed as the
  lantern tree**, with `"withered": true`. Not a second recipe: the two files
  differ by a name and one flag, which is how both halves of the map hold the
  same tree. (Before 2026-09-27 this was a separately tuned species; git
  history has it if the old silhouette is ever wanted back.)

The shape rules that matter most: `height` is the trunk up to its first
fork, `forks` split the trunk into limbs, `curve` bends each branch in an
arc, `branches` adds laterals along a parent, and `gravity` near zero lets
`curve` do the gnarl — a big droop plus a lean tips the crown over. `wobble`
is the wander on top of the arc, spread over the whole branch, so a coarse LOD
wanders the same way instead of a different way.

The life curve is three fields, and the defaults suit a tree:
`seasons_to_grown` (12) is how many seasons it takes to reach full size,
`sprout_size` (0.06) is how big it is when it sprouts, and `seasons_of_life`
(160) is the whole arc — grown, then old, then dying, each thinner in the
crown than the last. Generations arrive evenly across the growing part, so a
plant is complete exactly when it is grown.

The year needs nothing from a recipe unless the species has something to say
about it: leaves come and go on their own (`evergreen: true` opts out), and
`blooms` and `fruit` declare a crop —

```json
"fruit": { "per_tip": 2, "share": 0.4, "depth": 1, "window": [0.45, 0.8], "along": 0.3 }
```

— which is **sockets, not geometry**. What hangs at them comes from Trellis, a
carve or a recipe, and which tips carry a crop is each tip's own answer, so
the same tips bear every year and picking one says nothing about the others.

`wither` says what dying looks like for this species: `gnarl` (extra arc along
every branch, the way it was already bending), `sag`, `darken`, and `drain`
(how far the colour goes to grey — a dimmer blue is a crystal tree at night,
not a dead one, so the colour has to leave first; it drains the baked bark's
texels too, and leaves the normal and ORM maps alone, because dead wood keeps
its grain).
