# Changelog

All notable changes to **Thread Engine**. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); this file is
materialized from the Pixygon Changelog API — edit there, not here.

## [0.9.0] — 2026-09-27

### Added
- Wind! Every plant now ships with its wind baked into the mesh — a slow whole-tree lean, each limb swinging on its own beat, and leaves trembling at the tips. Because the wood knows its own hierarchy, a stand of trees ripples instead of nodding in unison, and the same branch keeps its phase at every age and after every cut. Unity picks it up automatically through the Grove Wind shader; a mesh without the channels simply stands still.
- `thread grow --impostor` makes the last LOD for any model: eight views 45° apart rendered into a single atlas, standing on eight quads so the camera always sees the view taken from its own side. You get `<stem>.impostor.glb` plus `<stem>.atlas.png` to look at. It's an ordinary model with an ordinary cutout texture, so nothing downstream needs a special case.
- The previewer can now render a single view from any angle with a transparent backdrop, so tiles can be composited instead of always arriving on the studio gradient.

### Changed
- Exported models can now carry up to four UV sets (TEXCOORD_0 through TEXCOORD_3) rather than two, with each set written only when every vertex has one.

### Improved
- Turntable previews can now spin up to eight views instead of six.

### Fixed
- Textures with holes in them — leaf cards, impostor atlases — are now exported as proper cutouts and drawn as cutouts in the previewer, so engines no longer fill in the empty space with solid colour.


## [0.8.0] — 2026-09-27

### Added
- Every vertex of a tree now names the branch it belongs to, carried in the mesh's second UV set (low 16 bits in u, high 16 in v). A game can hit-test a swing straight against the wood — leaves included, at every LOD — and know exactly which branch it struck without asking the generator anything.
- A companion `<stem>.branches.json` ships next to every exported model, saying what each branch id means: its parent, level, where it sprouts, where it ends, its radius, and whether it's a stump. That turns a raw hit on the mesh into a cut at the right joint.
- Chopping is now part of a plant's state. `cut` takes a branch by id at the joint it sprouts from: what's left is a short capped stump with a `cut` socket at the wound facing the way the branch went, and everything beyond it — leaves and fruit included — is gone. The trunk has no joint and is ignored; felling stays a separate idea.
- `--fallen <id>` grows the part that dropped: the whole subtree exactly as it stood, same seed, same clock, same leaves and fruit, brought onto its own base so a game can spawn it at the `cut` socket with no rotation and it lines up with the wound.
- Harvesting is state too. `taken` picks a socket by name with the age it grows back at — leave the age out and it stays picked until the game says otherwise. A picked socket is simply absent; its neighbours are none the wiser.
- New CLI flags on `thread grow`: `--cut <id>…`, `--take <socket>[@age]…` and `--fallen <id>`. Ids read as a plain number, `0x…`, or a socket name, so you can paste whatever the tool just told you.

### Changed
- A plant's whole mutable state is still a short, flat list — `withered`, `cut`, `taken` — small enough to sync over a network or sit in a world placement, and it reads the same in a recipe file or a Quarry submission. Everything in it filters the plant the seed already decided; nothing grows anything new.
- `grow()` now takes the state by reference and `State` is no longer `Copy`, since it carries lists. Existing recipe files and callers passing only `withered` keep working unchanged. **(BREAKING)**
- Stumps are no longer mistaken for branch tips: they get a `cut` socket instead of a `tip` socket and grow no leaves, so nothing sprouts from a wound.

### Improved
- Exported models carry a second UV set only when every single vertex has one — a half-filled channel is left out entirely rather than shipping coordinates that lie. Models without it export byte-for-byte as before.


## [0.7.0] — 2026-09-27

### Added
- Plants now move through the year. A `season` from 0 to 1 wraps through bud, leaf, bloom, fruit, seed drop and bare: the crown fills and empties smoothly, and leaves turn gold before they drop. Evergreen species opt out with one flag.
- A plant now has a whole life, not just a growing-up. Past full size it goes mature, then old, then dying, each thinner in the crown than the last — same tree, same branches, just less of them. `seasons_of_life` (default 160) sets the arc.
- Blooms and fruit are declared per species as a crop — how many per tip, what share of tips bear, and the window in the year they're on the plant. Which tips bear is each tip's own answer, so the same tips bear every year and picking one says nothing about its neighbours. Out of season the sockets simply aren't there, so nothing hangs.
- A plant can now be dead standing. `"withered": true` is a state, not a second species: the same seed, the same branches in the same order with the same ids, bent further along the arc each was already drawing, pulled down, and with the colour drained out of the wood before it dims — because a dimmer blue is still a blue tree at night. Normal and ORM maps are untouched: dead wood keeps its grain.
- `thread grow --year sheet.png` is the year's proof sheet, the way `--life` is age's: the same individual six times at one scale, bud through bare, with the crown turning gold before it drops. `--season`, `--withered` and `--hang-kind` are the other new flags.

### Changed
- Sockets are now typed — `tip`, `bloom`, `fruit`, and `cut` reserved — and named `<kind>-<branch id>`. `--hang` takes `--hang-kind` to fill a particular kind, so lanterns hang at fruit sockets rather than anywhere a branch ends. **(BREAKING)**
- The withered tree recipe is now the lantern tree's own species and seed with one flag flipped, instead of a separately tuned species. Both halves of the Lantern Desert hold the same tree. The old hand-tuned silhouette is in git history if it's ever wanted back. **(BREAKING)**
- The recipe README now documents the year, the life curve's third field, crops-as-sockets, and what `wither` does — including why colour has to leave before light does.

### Fixed
- A twig asked to carry no leaves now grows none, instead of always forcing at least one — that's what makes a bare winter crown actually bare.


## [0.6.0] — 2026-09-26

### Added
- A plant is now a species, a seed and a clock — `thread grow --age <seasons>` shows the same individual as a sapling or a grown tree, and `--season <0..1>` says where in the year it stands. Age counts game ticks, never wall-clock days, so a world that runs a season every ten minutes and one that runs a season a month use the same numbers.
- `thread grow --life sheet.png` renders six ages of a plant in one frame at one scale — the proof sheet that shows growth as growth, instead of framing each age to fill its own tile and hiding the one thing age does.
- Sockets are named after the branch that ends there (`tip-<branch id>`) and carry that id, so a fruit picked or a branch cut leaves every other fruit alone, and Unity can find the same tip at every age and every level of detail. A tip is a branch with no children *yet*, so a sapling has sockets at the ends it actually has. **(BREAKING)**
- Preview rendering gained a `fill` knob for how much of the frame a model occupies, so a wide sheet holding a row of models can use the room it has instead of sizing everything to the classic turntable framing.

### Changed
- Recipe files gained `seed`, `age` and `season` alongside the species' own fields, plus a two-field life curve: `seasons_to_grown` (12 by default) and `sprout_size` (0.06). Leave the clock out and a file shows the grown plant, so existing recipes still read as themselves — but because branch addresses changed, they grow different *individuals* than before. `grove::grow` takes three arguments now, and `GrowRecipe` is an alias for the new `Species`. **(BREAKING)**

### Fixed
- Growing older no longer reshuffles a plant. Every branch's shape is now addressed from its path through the plant rather than drawn from a stream as the tree was built, so a sapling's branches are exactly the branches its grown self has, in the same places, pointing the same way.
- Coarser levels of detail now drop leaves instead of growing a different crown — the leaves that remain are the very same leaves, and a branch's wander is the same whether it's meshed with six segments or sixty.


