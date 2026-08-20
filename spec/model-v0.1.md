# Model v0.1 — 3D as a program

Status: **draft**. Reference implementation: [`infinite-manifest::model`]
(format), [`chisel`] (carve, bake, export, preview), [`weft::model_lib`]
(the standard library). Part of the Thread spec suite; the sibling of
[weft-v0.1](weft-v0.1.md), whose rules it inherits.

## 0. Why this exists

Agents make 3D now, and the tool they reach for was designed for people
typing at a scene graph. That works — and it stops working exactly where it
matters: nothing is *checked*, nothing is *addressed*, the same code gives
different meshes on different machines, materials are somebody else's
download, and the output is a pile of vertices nobody can diff.

A model on the Thread is a **program that computes geometry and its own PBR
materials**, and the properties fall out of that:

| | three.js-style scene code | a Thread model |
|---|---|---|
| Parameterised | yes | yes |
| Verified before it runs | no | **yes** (total, fuel-bounded, effect-free) |
| Identity | a file path | **its hash** — the model *is* its content |
| Same output everywhere | not guaranteed | **guaranteed** (fixed-point, no floats in the language) |
| Materials | find/author textures | **generated with the model** (albedo, normal, metallic, roughness, AO) |
| Ships as | JS + assets | a few hundred bytes of graph |
| Exports | you write the exporter | `.glb`, PBR-complete, always |

## 1. The wire form: a flat sequence of carving steps

Weft has no recursive types, so a tree cannot be a value — but a **list of
uniform records** can. That constraint produced a better format than a tree
would have: modeling reads as *steps* ("start with a box, blend a sphere,
cut a cylinder"), steps are generable by loops, diffable line by line, and
streamable.

```jsonc
{ "name": "lantern",
  "nodes": [
    { "prim": "box", "w": 0.4, "h": 0.5, "d": 0.4, "y": 0.6, "round": 0.06 },
    { "prim": "box", "mode": "cut", "w": 0.3, "h": 0.36, "d": 0.6, "y": 0.6 },
    { "prim": "sphere", "part": 1, "r": 0.12, "y": 0.6 }
  ],
  "materials": [
    { "name": "iron",  "texture": { "kind": "fbm", "colors": [[0.2,0.19,0.22],[0.35,0.33,0.36]],
                                    "smoothness": [0.5,0.7], "metallic": [0.9,1.0], "height": 0.2 } },
    { "name": "flame", "color": [1,0.7,0.3,1], "emissive": 1.0 }
  ] }
```

**Node** (every field optional, every default sane): `prim` — `sphere` ·
`box` · `cylinder` · `capsule` · `cone` · `torus` · `lathe`; `mode` — `add`
(default) · `blend` (smooth union over `k` metres) · `cut` · `intersect`
(the first step of a part is its seed whatever its mode); `part` — which
material group; `x`/`y`/`z`, `rot`, `axis`; `r`, `r2`, `h`, `w`, `d`,
`round`, `k`; `profile` — flat `r, y` pairs for `lathe`.

**Material** — `texture` (a [procedural recipe](world-manifest-v0.1.md), the
full PBR set), `color`, `emissive`, `resolution`, `uv`
(`auto`|`box`|`cylindrical`|`spherical`), `uv_scale`, `name`.

A model with no materials is still PBR-complete: it gets the default one.

## 2. The code form: a Weft program that returns that value

The same value, computed. The standard library (`weft-model`, published on
[wpm](weft-pack-v0.1.md)) supplies constructors (`sphere`, `cube`,
`cylinder`, `lathe`…), transforms (`at`, `spin`, `cut`, `blend`, `part`,
`round`, `lay`), composition (`join`), **repeaters** (`ring_of`, `row_of`),
parametric parts (`column`, `stairs`, `arch`, `vase`, `bowl`, `table`,
`rock`), materials (`marble`, `granite`, `sandstone`, `wood`, `iron`,
`brass`, `terracotta`, `plaster`, `moss`, plus `tint`, `glow`, `weathered`)
and assembly (`model`, `model1`).

```text
stairs(12, 0.18, 0.28, 1.2)      → twelve solid steps, computed by a fold
column(5.2, 0.44)                → a turned classical profile from two numbers
model1("hall-column", …, marble())
```

A conformant implementation MUST evaluate model programs **outside the frame
loop** (weft-v0.1 §1.3) and MUST bound the evaluation with fuel.

## 3. What an implementation owes the author

These are the defaults that make a model *arrive finished*:

1. **The full PBR set, always** — base color, tangent-space normal, metallic,
   roughness, ambient occlusion. Baked from the recipe, deterministically and
   seamlessly tiling. `smoothness` is accepted as the inverse of `roughness`
   for authors who think in that vocabulary.
2. **Real UVs and real tangents** — projection chosen from the shape
   (cylindrical for turned forms, spherical for balls, box otherwise), seams
   resolved per face, tangents derived from the UV derivatives (not a guessed
   axis) so normal maps line up.
3. **Efficient geometry** — plain boxes are emitted exactly (12 triangles, no
   sampling); disjoint unions are meshed per component on their own tight
   grids rather than one coarse grid over everything; vertices that agree in
   every attribute are welded.
4. **Export that loses nothing** — one self-contained `.glb`: every part a
   mesh, every material complete (`baseColorTexture`,
   `metallicRoughnessTexture`, `occlusionTexture`, `normalTexture`,
   `emissiveFactor`), textures embedded as PNG, tangents and vertex colors
   included.
5. **A way to see it** — a preview the author can look at without a GPU or a
   browser, shaded with the same maps the export carries.

## 4. Conformance

The reference tools are the arbiter (conformance-v0.1 §"the suite is the
standard"): `thread model <model.json>` builds any conformant model;
`thread model --lib <part>` builds from the standard library;
`--preview <png>` renders it. A model is conformant when it validates
(§1 vocabulary), resolves to at least one part, and its materials validate.
