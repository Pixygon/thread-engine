# Changelog

All notable changes to **Thread Engine**. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); this file is
materialized from the Pixygon Changelog API — edit there, not here.

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


