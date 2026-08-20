# thread-engine — the open half of the Thread

**The Thread** is an open, spatial successor to the web: addressable 3D worlds
that link to one another the way pages do. This repository is the reference
implementation of everything a browser needs in order to speak it — and it is
deliberately *not* a browser. [Infinite](https://pixygon.io) is the branded
browser built on these crates; this is the engine, the language, the mesher and
the tools, under a permissive licence, so that Infinite is not the only browser
anyone can build.

## The crates

| Crate | What it is |
|---|---|
| [`thread-manifest`](crates/infinite-manifest) | The **World Manifest** — the document format of the Thread, the "HTML" of a place. Renderer-agnostic. |
| [`thread-engine`](crates/loom) | **Loom** — the embeddable engine: resolution, veils, presence, Codex, the 2D-web reader. |
| [`weft-lang`](crates/weft) | **Weft** — the Thread's native code format. Content-addressed, verified, total program graphs that cannot diverge, allocate unboundedly, or lie about what they are. |
| [`thread-chisel`](crates/chisel) | **Chisel** — the mesher. Shape recipes and Weft model programs become PBR-complete meshes. Blender for agents, one word at a time. |
| [`thread-avatar`](crates/infinite-avatar) | Portable avatars (avatar-v0.1): slots, specs, the Portable Item Convention. No rendering. |
| [`thread-structured-id`](crates/thread-id) | The `CCSSNNNN` id scheme worlds address their parts with. |
| [`thread-cli`](crates/thread-cli) | `thread` — scaffold, validate, lint, model, build levels, publish, doctor a live host. |
| [`thread-conformance`](crates/thread-conformance) | Prove a corpus, a host or a relay honours the spec — **without** this implementation. |
| [`thread-relay`](crates/thread-relay) | A self-hostable presence relay. |
| [`thread-rendezvous`](crates/thread-rendezvous) | A self-hostable P2P signalling rendezvous. |
| [`weft-pack`](crates/weft-pack) | Weft package tooling. |

## Try it in two minutes

```bash
cargo install --path crates/thread-cli

thread level --figure hall \
    --args '["My Hall", 14, 5.2, 12, "classical", "marble", "dusk"]' \
    --no-store -o world.json
thread lint world.json
```

That is a complete, walkable, lint-clean hall — floor, walled colonnade, lit,
with a doorway out. `--no-store` means every model in it was meshed locally, so
the folder is self-contained and a rebuild is byte-identical.

## The specification

The normative documents live in [Pixygon/thread-spec](https://github.com/Pixygon/thread-spec);
`spec/` here is a snapshot of the same files. **The spec is the standard and
this is one implementation of it** — where they disagree, the spec is right and
this is a bug.

`worlds/` is the reference constellation: nineteen conformant worlds, the same
ones served at `thread://pixygon.io`. Point the conformance suite at them:

```bash
cargo run -p thread-conformance -- worlds
```

## Licence

MIT OR Apache-2.0, at your option.
