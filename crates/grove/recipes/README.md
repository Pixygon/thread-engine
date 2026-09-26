# Grove recipes

Trees are rules, not vertices. Each file here is a `GrowRecipe`; `thread grow
<recipe> -o tree.glb --preview sheet.png` grows it, and `--publish` sends the
recipe to the Quarry, which grows it again itself and measures the sockets.

- `oak.grow.json` — a broadleaf with a two-generation crown (`leaves.depth: 2`).
- `lantern-tree.grow.json` — the Lantern Desert's crystal tree: bare, lifting
  tips (`gravity: -0.16`), veined bark, a little glow. Hang fruit at its tips
  with `--hang`.
- `withered-tree.grow.json` — the same tree after the aetherfall: leaning,
  gnarled (`curve`, `trunk_curve`, `lean`), dark bark.

The shape rules that matter most: `height` is the trunk up to its first
fork, `forks` split the trunk into limbs, `curve` bends each branch in an
arc, `branches` adds laterals along a parent, and `gravity` near zero lets
`curve` do the gnarl — a big droop plus a lean tips the crown over.
