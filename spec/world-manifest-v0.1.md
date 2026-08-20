# World Manifest — v0.1

**Status:** Draft · **Layer:** Content (the "HTML" of the Thread) · **Reference impl:** [`infinite-manifest`](../../crates/infinite-manifest)

## 1. What the Thread is

The **Thread** is an open, spatial, present, time-aware successor to the web.
Where the web serves *documents* linked by *hyperlinks*, the Thread serves
**worlds** — persistent, shared *places* — linked by **portals** (veils you step
through). Worlds are *pearls*; the Thread strings them together (see the canon
entry `the-pearl-dress`).

The Thread is a **standard**, not a product. Infinite is its first *browser* (its
Mosaic); anyone may build another. This document specifies the **World
Manifest** — the open, renderer-agnostic document every browser must be able to
resolve and render. It is the narrow waist of the stack: implement it, or nothing
interoperates.

### The stack (for context)

| Layer | The Thread | Web analog |
|-------|-----------|------------|
| Address | **Locator** (`thread://…`) | URL |
| Content | **World Manifest** (this doc) | HTML |
| Transfer | Arrival (fetch manifest + stream assets over HTTPS/CDN) | HTTP |
| Presence | shared avatar/state sync via a relay | *(none)* |
| Identity | portable Passport (DID-style) | cookies/accounts |
| Behavior | sandboxed WASM modules | JavaScript |
| Client | a **browser** (Infinite is the first) | browser |

## 2. Design principles

1. **Narrow waist.** Every browser implements this format or nothing interops.
2. **Renderer-agnostic.** Geometry is glTF + prefab references + a standard PBR
   material model — never a specific engine's internals. A second browser in a
   second language must be able to render a manifest with no Infinite code.
3. **Static-first, presence-optional.** A world is just files (manifest + glTF +
   WASM); host them on any static server, CDN, or IPFS. **Presence** (multi-user)
   is an *upgrade* enabled by naming a `presence.relay`. Without one, a world
   gracefully degrades to solo — exactly like a static web page.
4. **Meaning is native.** Any placement may carry a Codex slug; a browser
   "inspect" surfaces canonical lore.
5. **Time is an axis.** A world declares `environment.year`; the same place is
   addressable at another time via the Locator's `@when`.

## 3. The Locator (addressing)

```
thread://<host>/<path>[@<when>][#<place>]
```

- `host` — the world host authority (a domain).
- `path` — the world's path on that host (may be empty for the host root).
- `@when` — OPTIONAL timeline year to arrive at (first-class time navigation).
- `#place` — OPTIONAL named anchor (a `spawn.name` or `portal.id`) to arrive at.

Examples: `thread://archive.pixygon.io/codex-archive`,
`thread://market.pixygon.io/market#entry`, `thread://amebrak.pixygon.io/caul@0`.

## 4. Manifest structure

A manifest is a JSON document. Top-level fields:

| Field | Req | Meaning |
|-------|-----|---------|
| `thread` | ✔ | Format tag; MUST be `"thread/0.1"`. |
| `world` | ✔ | Metadata: `id`, `title`, `description`, `author`, `codex[]`, `license`. Only `id` and `title` are required; **unset optional metadata is omitted, never emitted as `null` or `""`**, so a consumer can test presence rather than presence-and-emptiness. |
| `environment` | | `year` (the `@when`), `sky`, `bounds`, `rules` (opt-in game mechanics). |
| `spawns[]` | | Arrival points; the first is the default, others are `#place` anchors. |
| `assets[]` | | External content by id: `{ id, uri, kind }`, `kind ∈ gltf|texture|wasm|audio|other`. |
| `prefabs[]` | | Deduplicated renderables keyed by `StructuredId`: `{ id, mesh, material }`. |
| `placements[]` | | Instances: `{ prefab, position, rotation, scale, type?, class[]?, codex?, behavior?, interaction?, data, children[]? }`. |
| `portals[]` | | Veils: `{ id, position, …, to (Locator), label, preview }`. |
| `behaviors[]` | | WASM modules: `{ id, wasm (asset id), on[] }`. |
| `styles[]` | | CSS-like cascade rules: `{ select, interaction? }` — attach properties by type/class/id. |
| `presence` | | `{ mode?, relays[]?, relay?, rendezvous?, max_occupants?, voice }`. Absent → solo. `mode` ∈ `solo\|p2p\|relay` (see [presence-topology-v0.1](presence-topology-v0.1.md)) — a **declaration**, not a fact; see *What a world can do* below. |

**What a world can do beats what it says.** `mode` states the author's intent;
the address fields (`relays`, `relay`, `rendezvous`) are what a browser can
actually act on. A consumer MUST derive the effective tier from the addresses:

| Effective tier | When |
|---|---|
| `relay` | `relays` or `relay` names at least one relay |
| `p2p` | no relay is named and `rendezvous` is present |
| `solo` | neither |

`mode` MUST agree with that derivation. Where it disagrees the addresses win —
a world claiming `mode: "relay"` with no relay named cannot host anyone, and one
naming a relay is not solo however it is labelled — and a validator SHOULD
report the disagreement.

An **empty** `relays: []` is the absence of a relay, never a considered choice
of none. There is no way to spell "deliberately solo, do not offer me a relay";
if that distinction is ever needed it gets a field of its own rather than an
overloaded empty list.

The corollary matters for tooling: a consumer deciding whether to *supply* a
relay must ask whether one is named, never whether the `presence` key is
present. Which keys an emitter writes is the emitter's business — `presence`
absent and `presence: {"mode":"solo"}` mean the same thing, and a check for the
key passes the second one while a room built for people ships silent.

**A reader may ignore; a writer may not drop.** A browser is free to skip
fields it doesn't understand — it discards its parse when the world unloads.
A **tool** that reads a world, changes something and writes it back is under a
stronger obligation: it MUST carry unrecognised fields through unchanged. Every
field added by an emitter newer than the reader looks unrecognised, so a lossy
round trip deletes the parts of a world its author has not yet upgraded to
read. A tool that cannot preserve them MUST NOT write the world back.

This is why the effective-tier rule above is stated over *values* and not over
a particular struct: a consumer should be able to answer "is a relay named?"
from the JSON it already holds, without deserializing a whole world into a
shape that may be older than the file.

**Presence relays.** Prefer `relays` — an ordered list of interchangeable,
conformant relays (primary first, then fallbacks) — so no single relay URL is a
point of failure; a browser tries them in order and uses the first reachable one.
The singular `relay` is retained for backward compatibility. A world targeting both
old and new browsers MAY set `relay` (its primary) *and* `relays` (the full list);
the effective order is `relays` then `relay` if not already present. Any conformant
relay works — presence is federated, so a world names its *own* relay(s).

**P2P presence.** A Tier-1 (serverless mesh) world sets `mode: "p2p"` and names its
introduction endpoint in the explicit **`rendezvous`** field (`wss://…/rtc/<key>`) —
never as a `relays` entry, since a rendezvous speaks a different protocol with
different failure semantics. `relays` MAY ride along as the Tier-2 fallback when the
mesh can't form. See presence-topology-v0.1 §3.1–3.2.

### Prefabs & materials

A `prefab.mesh` sets **exactly one** of `asset` (a glTF asset id) or `builtin`
(`cube`, `sphere`, `cylinder`, `capsule`, `plane`, `quad`). `material` is a
standard PBR block: `base_color_texture`/`orm_texture`/`normal_texture` (asset
ids) plus scalar `base_color`, `metallic`, `roughness`, and `emissive`
(default `0`; `> 0` renders unlit, glowing `base_color` — browsers with HDR
SHOULD bloom it). Prefab ids use the `StructuredId` `CCSSNNNN` scheme
(imported/world prefabs live in category `60`).

### Portals

`to` is a destination **Locator** (MAY target another host — this is what makes
the Thread a *web* and not a walled world). `preview` ∈ `none | static | live`:
a `live` preview shows the real far side (crowd, weather, time-of-day) before you
step through. Crossing a portal to a new host performs an **identity handoff**
(your Passport, avatar, and inventory travel) — specified separately in the
Portal Handoff layer.

### Nesting (the tree / DOM model)

A placement MAY carry `children[]` — placements positioned **relative to their
parent**. Their transforms compose down the tree (parent world-transform × child
local-transform), so a scene is built by *containment* — a `table` with a `lamp`
and a `book` on it — instead of every object carrying absolute coordinates. Move
the parent and the whole group moves. A flat `placements[]` is just a forest of
depth-1 trees, so existing worlds are unchanged. A browser MUST compose child
transforms; colliders and interactables use the composed world position.

### Styles (the cascade)

`styles[]` is a **CSS-like cascade**: each rule has a `select` (a simple selector)
and properties it attaches to matching placements — so you define behaviour once,
by kind, instead of inline on every object. A placement carries a `type` (the
element tag, e.g. `"tree"`) and `class[]` for selectors to target; its `name` is
the id.

```json
"styles": [
  { "select": "tree",         "interaction": { "label": "Chop", "effects": ["despawn"] } },
  { "select": ".harvestable", "interaction": { "label": "Gather" } },
  { "select": "#old-oak",     "interaction": { "label": "Fell the ancient oak" } }
]
```

Selectors: `"tree"` (type), `".harvestable"` (class), `"#name"` (id). **Cascade:**
an inline `placement.interaction` always wins (like inline `style=`); otherwise
the highest-specificity matching rule applies (**id > class > type**), later rules
breaking ties (source order). Unknown properties MUST be ignored. Today the layer
cascades `interaction`; it grows to cover mesh + material. (Descendant selectors
and inheritance down the tree are a later addition.)

### Rules (a place opts into game mechanics)

`environment.rules` declares the game-y mechanics this world **enforces on its
visitors**. All default **off** — the browser ships them as capabilities and a
place turns on only what it needs (as a page opts into pointer lock). Most
threads are not games; a store or gallery leaves these off and remains a calm
place to browse.

| Rule | Meaning |
|------|---------|
| `survival` | Survival meters (hunger/…) decay and can harm the visitor. |
| `gathering` | Harvestable resource nodes exist and can be gathered. |
| `combat` | Offensive combat (attacking actors) is permitted. |

The two input verbs — **activate** (the primary interact key) and **inspect**
(examine) — are *always* available and are NOT gated by rules; they are the
browser's click / right-click, not game mechanics.

### Interactions (codeless, on a placement)

A `placement.interaction` is a **declarative, no-code** interaction — the simple
path (for anything richer, bind a `behavior` WASM module instead). The visitor's
*activate* key runs it.

```json
"interaction": {
  "label": "Chop",              // the creator-defined verb shown in the prompt
  "hits": 3,                     // activations to complete it (default 1)
  "effects": [                   // applied top-to-bottom on completion
    { "give_item": { "item": "20100001", "count": 3 } },
    "despawn",
    { "message": "You chopped the old oak." },
    { "effect": "leaves" }
  ]
}
```

Effects: `give_item` (into the visitor's inventory; `item` is a `StructuredId`),
`despawn` (remove the object — mesh + collider), `message` (a transient toast),
`effect` (a named visual/audio effect). Unknown effects MUST be ignored. A
browser that implements the declarative layer runs these without any sandbox.

## 5. Conformance

- A **conformant manifest** parses as JSON, carries a recognized `thread/…` tag,
  and passes validation: every placement references a declared prefab; every
  prefab has exactly one mesh source that resolves; every behavior/asset
  reference resolves; every portal `to` is a valid Locator. The reference
  validator is `WorldManifest::validate`.
- A **conformant browser** MUST: resolve a Locator, fetch + validate a manifest,
  render prefabs/placements with the standard material model, and traverse
  portals. It SHOULD render `live` portal previews and join presence when a relay
  is present; it MAY ignore behaviors it cannot sandbox. Unknown fields MUST be
  ignored (forward compatibility).

## 6. Versioning

The `thread` tag is `thread/MAJOR.MINOR`. Minor versions are additive and
backward-compatible (browsers ignore unknown fields). Major versions may break.
The **conformance test suite** — not this prose — is the ultimate arbiter: the
two shipped worlds (`worlds/codex-archive`, `worlds/market`) are the canonical
fixtures and MUST validate under any implementation claiming `thread/0.1`.

## 7. Example

See [`worlds/codex-archive/world.json`](../../worlds/codex-archive/world.json)
(a walkable Codex viewer) and [`worlds/market/world.json`](../../worlds/market/world.json)
(a walkable market). They portal to each other across hosts — the Thread's "two
linked pages" moment.

## Addendum (v0.1, 2026-08-17): procedural shape prefabs

`MeshRef` gains a third, mutually exclusive source: `shape` — a
signed-distance recipe tree (`prim`/`op`/`lathe` nodes; see
`crates/infinite-manifest/src/shape.rs` for the normative field set) plus
optional `resolution` (sampling density; browsers MUST clamp, reference cap
96). A browser meshes the recipe at load time. Rationale: the Thread ships
intent, not vertices — a model is a tiny, diffable recipe, and no asset
hosting is needed for carved geometry. Additive: exactly one of
`asset`/`builtin`/`shape` must be set; older browsers ignore shape prefabs.

## Addendum (v0.1, 2026-08-17): procedural material recipes

`MaterialRef` gains `texture` — a procedural PBR recipe (kind/scale/seed/
colors/roughness/metallic/height/ao/size; see
`crates/infinite-manifest/src/texture.rs` for the normative field set). A
browser bakes it locally into base-color, tangent-space normal, and packed
occlusion-roughness-metallic maps, deterministically (same recipe → same
texels everywhere) and seamlessly tiling. When set, the baked maps take the
place of the `*_texture` asset slots; scalar factors still multiply. Bake
size is browser-clamped (reference cap 512). Additive: older browsers ignore
the field and render the scalar material.
