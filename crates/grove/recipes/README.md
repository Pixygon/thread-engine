# Grove recipes

A plant is **a species, a seed and a clock**, and the mesh is derived from
those three whenever anyone needs it. Each file here is one plant:

- the **species** — every shape, material and life-curve field — which is the
  plant's identity and what the Quarry keys a design on;
- `seed` — which individual. One number, and it decides the *whole potential
  plant*: every branch it could ever have;
- `age` and `season` — the moment. `age` counts seasons since sprouting (ticks
  the game controls, never wall-clock days); leave it out and the file shows
  the grown plant, which is what these three do.

```sh
thread grow oak.grow.json -o oak.glb --preview sheet.png   # the turntable
thread grow oak.grow.json --seed 23                        # another individual
thread grow oak.grow.json --age 3                          # a sapling
thread grow oak.grow.json --life life.png                  # six ages, one scale
thread grow lantern-tree.grow.json --hang fruit.glb --hang-count 12
thread grow oak.grow.json --publish                        # the Quarry regrows it
```

Age never reshuffles a plant. It **filters** — which branches have emerged —
and **scales** — how far each has grown and how thick it is. Every branch a
sapling has is a branch its grown self has, in the same place, pointing the
same way; `--life` is the proof, because it puts six ages in one frame at one
scale instead of framing each one to fill its own tile.

- `oak.grow.json` — a broadleaf with a two-generation crown (`leaves.depth: 2`).
- `lantern-tree.grow.json` — the Lantern Desert's crystal tree: bare, lifting
  tips (`gravity: -0.16`), veined bark, a little glow. Hang fruit at its tips
  with `--hang`.
- `withered-tree.grow.json` — the same tree after the aetherfall: leaning,
  gnarled (`curve`, `trunk_curve`, `lean`), dark bark.

The shape rules that matter most: `height` is the trunk up to its first
fork, `forks` split the trunk into limbs, `curve` bends each branch in an
arc, `branches` adds laterals along a parent, and `gravity` near zero lets
`curve` do the gnarl — a big droop plus a lean tips the crown over. `wobble`
is the wander on top of the arc, spread over the whole branch, so a coarse LOD
wanders the same way instead of a different way.

The life curve is two fields, and the defaults suit a tree: `seasons_to_grown`
(12) is how many seasons it takes to reach full size, and `sprout_size` (0.06)
is how big it is when it sprouts. Generations arrive evenly across that life —
with `levels: 4` the trunk fills the first fifth, then each generation the
next — so a plant is complete exactly when it is grown.
