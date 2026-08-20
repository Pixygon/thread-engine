# Avatar v0.1 — one portable "you", everywhere

Status: **draft**, companion to [Passport v0.1](passport-v0.1.md) §3.
Part of the Thread spec suite — see `thread-protocol-v0.1.md` for the map.

## 1. What it is

The Thread's avatar layer is the Pixygon **Portable Item Convention**, adopted
whole: the same model that `com.pixygon.avatar` (Unity) and `@pixygon/avatar`
(web) already implement. One avatar, made anywhere, renders everywhere — a
Unity game, the web Avatar Studio, and any Thread browser draw the same "you"
from the same data. *Miis + Pokémon, truly global.*

Two ideas carry the whole design:

1. **An avatar is a spec** — which part sits in which slot, plus body height.
2. **A part is one GLB file that carries its own meaning** — mesh, materials,
   textures, AND identity/stats/lore baked into the file itself.

## 2. The spec (the assembled avatar)

```json
{ "bodyHeight": 0.5, "parts": { "Body": 11010001, "Jacket": 123456 } }
```

- `parts` — slot name → **partId**. Slot names are the shared `AvatarSlot`
  enum (26 slots, append-only): biology (`Body`, `SkinType`, `SkinTone`,
  `Eyes`, `Ears`, `Tail`, `Claws`, `Gills`, `Webbing`, `Horns`, `Wings`,
  `Snout`, `Coat`), hair (`Hair`, `HairColor`), clothing (`Shirt`, `Pants`,
  `Shoes`, `Jacket`, `Headgear`, `Socks`, `Gloves`), accessories
  (`AccessoryHead`, `AccessoryBody`, `AccessoryLapel`, `Offhand`).
- `bodyHeight` — 0..1, lerps overall body scale. Default `0.5`.
- **A race is a body mode, not an outfit**: races fill biology slots
  (a Hedningr is scale skin + tail + claws + horns); any race wears any shirt.
- **partId is THE key** — the stable IdObject id shared by Unity, the web,
  and the server catalog. Never key parts by name or database id.

The reference implementations of this shape: `infinite-avatar` (Rust —
`AvatarSpec`), `com.pixygon.avatar` `AvatarSpec`/`AvatarData` (C#),
`@pixygon/avatar` (TypeScript).

## 3. The parts (the Portable Item Convention)

A part is **one GLB** with its meaning baked in at
`asset.extras.pixygonItem` (schema 1, append-only):

```json
{ "schema": 1, "id": 123456, "kind": "garment", "slot": "Jacket",
  "title": "Wayfarer's Coat", "codexSlug": "wayfarers-coat",
  "stats": [ {"id": 40001, "key": "Defense.Defense", "value": 6} ],
  "glb": { "units": "meters", "up": "+Y", "forward": "+Z", "origin": "anchor" } }
```

Non-negotiable GLB rules: units **meters** · **Y-up, +Z forward** · origin at
**grip** (held) / **worn anchor** (garments) / **feet** (bodies) · **PBR
metallic-roughness** with textures **embedded** · bodies and clothing are
**skinned meshes binding to the shared skeleton by bone name**
(`attachType: "skinned"`); rigid props snap to a named bone
(`attachType: "bone"` + `snapBone`).

Parts are catalogued as **`AvatarAsset`** documents (partId, GLB URL,
attachType, snapBone, and an optional `fix` — per-asset load-time corrections
applied scale → rotation → position, then boneMap renames, on every load,
before binding). A renderer resolves a spec: partId → catalog doc → fetch the
GLB (any Thread `AssetSource` can) → read the manifest → bind.

Manifest fallback order (all readers): GLB `extras` → `.json` sidecar → the
catalog doc. **Never mutate meaning client-side** — the GLB is the source of
truth; browsers and studios edit *configurations*, not parts.

## 4. Passports carry avatars (the "follows you through" part)

The [Passport descriptor](passport-v0.1.md) §3 embeds this spec:

```json
"avatar": {
  "spec": { "bodyHeight": 0.5, "parts": { "Body": 11010001, "Jacket": 123456 } },
  "colors": { "skin": "#c8956c", "hair": "#2a1f1a" },
  "assets": "https://api.pixygon.io/v1/avatar/assets"
}
```

- `spec` — the shared model above. Change your jacket once — in a Unity game,
  in the web Studio, anywhere — and every world on the Thread renders it,
  because they all read this one spec through your Passport.
- `assets` — the catalog base for partId → GLB resolution. Hosts without it
  fall back to the issuer's default catalog.
- The descriptor is fetched, rendered, and **discarded** (passport-v0.1 §3) —
  erasure keeps working because worlds hold references, never copies.
- Presence: co-travelers' descriptors arrive via the relay's `join`
  (presence-wire §2); browsers render each traveler from *their* spec.
  A traveler with no Passport (or no spec) renders as the placeholder — walking
  anonymously always works.

## 5. Conformance

- A browser rendering avatars MUST resolve parts by partId and MUST apply
  `fix` before binding. It SHOULD fall back gracefully (placeholder body) for
  parts it cannot fetch.
- A catalog host MUST serve part GLBs with `Access-Control-Allow-Origin: *`,
  like every Thread asset.
- The Rust reference (`infinite-avatar`): `AvatarSpec` (wire shape),
  `manifest::ItemManifest` + `manifest_from_glb` (the meaning reader),
  `manifest::AvatarAssetDoc` + `index_by_part_id` + `resolve_spec` (catalog
  resolution). Slot-name parity across runtimes is pinned by test.
