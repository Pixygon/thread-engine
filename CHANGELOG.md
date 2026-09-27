# Changelog

All notable changes to **Thread Engine**. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); this file is
materialized from the Pixygon Changelog API — edit there, not here.

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


