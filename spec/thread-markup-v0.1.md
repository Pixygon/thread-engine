# Thread Markup v0.1 — the authoring form of the World Manifest

Status: **draft**, format tag `thread/0.1` (compiles to World Manifest v0.1).
Part of the Thread spec suite — see `thread-protocol-v0.1.md` for the map.

## 1. What it is

Thread markup is the **HTML of the Thread**: a source form that reads like
HTML + CSS and compiles to a World Manifest (the "DOM" — see
`world-manifest-v0.1.md`). The two forms are equivalent; markup exists for the
low floor. A `.thread` file is a complete world in one readable page:

```html
<world id="grove" title="The Grove" description="A quiet clearing"
       sky="0.1 0.1 0.2 / 0.5 0.3 0.2" rules="gathering">
  <spawn at="0 0 8" yaw="180"/>
  <tree class="harvestable" at="3 0 -2"/>
  <cube at="0 0 -5" rot="0 45 0" codex="the-thread">
    <sphere at="0 1 0"/>
  </cube>
  <quad id="sign" at="0 1.4 -6" scale="5 2 0.2" url="https://example.com"/>
  <portal to="thread://market" at="0 1.2 -8" label="Market"/>
</world>
<style>
  tree { mesh: "trees/oak.glb" }
  .harvestable {
    interaction: "Chop";
    interaction-hits: 3;
    interaction-gives: 20100001 3;
    interaction-message: "You chopped the old oak.";
    interaction-despawns: true;
  }
  #sign { color: 0.9 0.87 0.78; emissive: 0.4 }
</style>
```

**Serving markup is first-class.** A host MAY publish `world.thread` instead of
`world.json` at the `.well-known/thread` location (see
`locator-and-resolution-v0.1.md` §3); browsers MUST accept either. Compilation
MUST validate: a `.thread` file that compiles is by definition a conformant
manifest.

## 2. Document structure

A document is one `<world>` element plus an optional `<style>` block. Syntax is
XML-ish: elements nest, attributes are `name="value"` (single or double
quotes), self-closing `<tag/>` is allowed, `<!-- comments -->` are skipped,
text between tags is ignored.

### `<world>` (required root)

| attribute | meaning | manifest field |
|---|---|---|
| `id` | stable world id | `world.id` |
| `title` | display title | `world.title` |
| `description` | one-liner | `world.description` |
| `sky` | a named preset — `dawn` `day` `dusk` `night` `void` — or `"zr zg zb / hr hg hb [/ sx sy sz]"` (zenith / horizon / optional sun direction) | `environment.sky` |
| `year` | timeline year (integer, may be negative) | `environment.year` |
| `rules` | space/comma list of `survival` `gathering` `combat` | `environment.rules` |
| `presence` | `none` (default) · `p2p` · a `wss://` relay URL (comma-separate fallbacks) — the creator's whole presence decision (presence-topology-v0.1 §2.1) | `presence` |

Unknown `rules` tokens are a **compile error** (a typo must not silently
default a mechanic off), and so is a `presence` value that is neither a mode
word nor a relay URL.

### `<spawn at="x y z" name="…" yaw="deg"/>`

An arrival point; `yaw` is in degrees. A world with no `<spawn>` gets a default
one (compilers MUST emit at least one spawn so every compiled world is
enterable).

### Object elements

Any of the builtin tags — `cube` `sphere` `cylinder` `capsule` `plane` `quad`
— places that primitive. **Any other tag** requires a `mesh:` style rule
(§3); the tag name becomes the placement's `type` for selector matching.

| attribute | meaning |
|---|---|
| `id` | placement name + `#id` selector target |
| `class` | space-separated classes for `.class` selectors |
| `at` | position `"x y z"` (default origin) |
| `rot` | Euler degrees `"x y z"`, applied yaw (Y) → pitch (X) → roll (Z) |
| `scale` | `"x y z"` (default unit) |
| `codex` | Codex slug — inspect opens its lore |
| `url` | a web link — the browser's reader opens it (signboards) |
| `data-*` | arbitrary per-object data; values that parse as numbers/booleans are typed |

Elements nest: a child's transform is **relative to its parent** (the DOM/tree
model of the manifest's `children`).

### `<portal to="thread://…" at label rot scale/>`

A veil. `to` MUST be a valid Locator.

### Structure vocabulary — `<room>` and `<wall>`

Architecture as elements, so an author (or a generator) never hand-maths a
colonnade out of cubes:

```html
<room at="0 0 0" r="15" h="4" columns="20" gates="180 -55 55">
  <cylinder id="pedestal" at="0 0.4 0" scale="2.6 0.8 2.6"/>
  <portal to="thread://elsewhere" at="0 1.4 -12" label="Onward"/>
</room>
<wall from="-5 0" to="5 0" h="2.5" thick="0.4"/>
```

- `<room at r h columns gates gate-width>` compiles to a floor disc, wall
  segments between columns, and columns with capitals at every joint.
  **Gates** are azimuths in degrees (0 faces −z, away from the default
  spawn; 180 faces +z, toward arrivals) — each gate azimuth opens the wall
  there. Children — placements, portals, spawns — are authored **relative to
  the room's centre** and move with it. Style properties: `color` (walls),
  `floor-color`, `accent-color` (capitals).
- `<wall from="x z" to="x z" h thick y>` is one straight wall between two
  ground points; colour from the cascade.

### Hyperlinks in text — `[[locator|phrase]]`

Inside any `text="…"` attribute, `[[thread://host/path|phrase]]` marks
`phrase` as a **hyperlink** (`[[thread://host/path]]` links the bare Locator).
This compiles to the manifest's `TextPanel.links` — the Thread's `<a>` tag.
A conformant browser renders link phrases distinctly (link-blue, underlined)
and follows them when the traveler interacts with the panel: one link
veilwalks immediately; several open the browser's link menu. Links live *in*
the text, exactly like the old web.

### Light — `light="r g b [intensity] [range]"`

Any object element (attribute or cascaded style property) may **emit a point
light** from its position — compiled to the manifest placement's `light`
field. Lamps are content, not renderer configuration; a conformant browser's
lighting honours them (`light="true"` = default warm lamplight, range 10 m).

### Weft bindings — `weft=` and `weft-on=`

`weft="<uri>"` binds a verified **Weft** module (weft-v0.1) to an object.
`weft-on="interact tick"` picks its events (default `interact`); a `tick`
subscription gives the module the world's heartbeat (behavior-abi §3) — the
Atrium's living clock is one line of markup plus one woven module.

## 3. The `<style>` block

CSS-like rules: `selector { property: value; … }`. Selectors are a single
type (`tree`), class (`.harvestable`), or id (`#sign`); specificity is
id > class > type, later rules break ties, each property cascades
independently.

**Appearance properties** (resolved at compile time into synthesized prefabs —
one prefab per unique look, meshes deduplicated):

| property | value |
|---|---|
| `mesh` | builtin name, or a glTF URI (relative, absolute, or `ipfs://`) |
| `color` | named (`white black red green blue yellow orange brown gray`) or `r g b [a]` |
| `metallic` | 0–1 |
| `roughness` | 0–1 |
| `emissive` | `true` (= 1) or a strength; > 0 glows unlit |

**Interaction properties** (carried into the manifest's runtime `styles`
cascade): `interaction` is the verb shown on the interact prompt; longhands
fill the rest. Longhands without an `interaction` verb are a compile error.

| property | value |
|---|---|
| `interaction` | the verb (`"Chop"`) |
| `interaction-hits` | completions needed (default 1) |
| `interaction-gives` | `<item StructuredId> [count]` |
| `interaction-message` | transient message on completion |
| `interaction-effect` | named visual/audio effect |
| `interaction-despawns` | `true` → object is removed |

Effects apply in order: give, effect, message, despawn.

## 4. Conformance

- The reference compiler is `infinite-manifest`'s (`thread-manifest`'s)
  `markup::compile`; `WorldManifest::from_text` auto-detects source form by the
  first non-whitespace byte (`{` → JSON, else markup).
- The conformance suite accepts `world.thread` corpus entries wherever it
  accepts `world.json`.
- `thread validate` and `thread doctor` accept both forms; `thread compile`
  materializes markup to JSON when a build step is preferred.

## 9. Carved shapes (`<shape>` / `shape=`) — v0.1 addendum

A top-level `<shape name="…" [resolution]>` defines a **signed-distance
recipe** — primitives (`<sphere> <box> <cylinder> <capsule> <cone> <torus>`,
rotational ones take `axis="x|y|z"`), combiners (`<union> <blend k> <cut>
<intersect>`), and `<lathe profile="r y, r y, …">` revolutions. Any element
may then take `shape="name"` in place of a builtin tag; the compiler embeds
the recipe in the prefab (`mesh.shape` + `mesh.resolution`, world-manifest
addendum) and browsers mesh it locally at load time — never in the frame
loop, resolution clamped browser-side. Unknown shape words and unknown
`shape=` names are compile errors. Browsers that predate shapes skip such
prefabs (additive-only rule upheld).
