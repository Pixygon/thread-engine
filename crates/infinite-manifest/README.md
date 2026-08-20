# thread-manifest — the World Manifest format of the Thread

The **World Manifest** is the "HTML" of the Thread: an open, **renderer-agnostic**
JSON description of a place that *any* browser can resolve and render. This crate
is the reference implementation of the format — parse, validate, and address
worlds — depending only on `serde` and [`thread-id`](https://crates.io/crates/thread-id),
so anyone building for the Thread can use or reimplement it with no engine baggage.

> In this repository the crate is named `infinite-manifest` for historical reasons;
> it is published to crates.io as **`thread-manifest`**. See
> [`docs/thread/publish-thread-crates.md`](../../docs/thread/publish-thread-crates.md).

## What it gives you

- **`WorldManifest`** — the document: `world` metadata, `environment` (sky, time),
  `spawns`, `prefabs` (built-in or glTF meshes + a standard PBR material), instanced
  `placements`, `portals` (the hyperlinks — veils to other worlds), sandboxed WASM
  `behaviors`, and optional `presence`.
- **`WorldManifest::from_json` / `validate`** — parse + check conformance: the
  version tag is recognized, every reference resolves, and portals address the
  Thread. The same check the [conformance suite](../../docs/spec/conformance-v0.1.md)
  runs.
- **`markup`** — Thread markup, the HTML/CSS-like authoring form
  ([spec](../../docs/spec/thread-markup-v0.1.md)): `markup::compile` turns a
  `.thread` source into a validated manifest, and **`WorldManifest::from_text`**
  auto-detects either form — hosts may serve `world.thread` exactly as they
  serve `world.json`.
- **`Locator`** — the address of a place: `thread://host/world@when#place`, with
  `well_known_url()` for decentralized `.well-known` hosting.

```rust
use infinite_manifest::{WorldManifest, Locator};

let world = WorldManifest::from_json(json_text)?;
println!("{} — {} veils", world.world.title, world.portals.len());

let loc = Locator::parse("thread://studio.example.com/gallery#entry").unwrap();
assert_eq!(loc.host, "studio.example.com");
```

## Spec

Normative format: [`docs/spec/world-manifest-v0.1.md`](../../docs/spec/world-manifest-v0.1.md).
Overview of how it fits with addressing, behaviors, presence, and conformance:
[`docs/spec/thread-protocol-v0.1.md`](../../docs/spec/thread-protocol-v0.1.md).

License: MIT OR Apache-2.0.
