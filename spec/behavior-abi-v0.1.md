# Behavior ABI — v0.1

**Status:** Draft · **Layer:** Behavior (the "JavaScript" of the Thread) · **Owner:** Infinite (browser/runtime), co-designed with the main agent (host services)

How a world's sandboxed **WASM behavior module** talks to a browser. A browser
that implements this can run any conformant behavior; a module compiled against it
runs in any browser. Capability-gated, language-agnostic, JSON-over-linear-memory
(simple first; a WIT/Component-Model binding may supersede in a later minor).

## 1. Model

- A `behavior` in the manifest names a WASM asset + the events it handles (`on[]`).
- Placements bind to a behavior by id. The browser instantiates one module
  instance per world (shared across its placements), sandboxed (no ambient
  capabilities — only the host-fns below).
- Events flow **host → module** (exported handlers); effects flow **module → host**
  (imported host-fns). The module never blocks; async host work (buy, codex fetch)
  resolves via a follow-up event.

## 2. Data passing

All strings are UTF-8 JSON passed by `(ptr: i32, len: i32)` into the module's
linear memory. The module MUST export an allocator the host calls to hand data in:

```
thread_alloc(len: i32) -> i32     // returns ptr to len writable bytes
thread_free(ptr: i32, len: i32)   // optional; host calls after a handler returns
```

The host reads returned `(ptr,len)` regions from module exports the same way.

## 3. Module exports (host → module)

```
thread_on_load()                                  // once, after instantiation
thread_on_interact(ptr: i32, len: i32) -> i64     // JSON InteractEvent; returns packed (ptr<<32|len) ActionList or 0
thread_on_event(ptr: i32, len: i32)               // JSON Event (async replies: purchase_result, codex_ready, presence)
thread_on_tick(dt_ms: i32)                         // optional; only if "tick" in on[]
```

`InteractEvent`: `{ placement, actor: {passport_sub?}, world, data }` where
`data` is the placement's manifest `data` block (e.g. `{item, price, currency}`).

`ActionList` (module → host, the return of `on_interact`): `{ actions: [Action] }`.

### Tick — the world's heartbeat

A behavior that lists `"tick"` in its manifest `on[]` receives periodic tick
events. WASM modules get `on_tick(dt_ms)` each frame. **Weft** modules are
dispatched at a steady ~4 Hz with an event record offering
`kind: "tick"` and `dt_ms` (total projection — the module's event type picks
the fields it wants), and their returned Actions are performed exactly as for
interact. Every beat is fuel-metered; worlds behave, they don't just react.

## 4. Actions (module → host, declarative effects)

The module returns *intents*; the host performs them (so the sandbox never touches
IO directly). v0.1 actions:

| Action | Shape | Host does |
|---|---|---|
| `codex.open` | `{ slug }` | fetch `codexBase/entities/:slug`, show the entry panel |
| `commerce.buy` | `{ item, price_ref }` | `POST /v1/thread/purchase` with the Passport; replies `purchase_result` |
| `navigate` | `{ to }` (Locator) | traverse the portal / veilwalk |
| `notify` | `{ text, level? }` | transient UI toast |
| `presence.emit` | `{ event, data }` | relay an interaction to other occupants |
| `set_state` | `{ placement?, patch }` | mutate visible placement state — **the scene ABI**. Implemented patch verbs (applied independently; a browser MAY support more): `text: "…"` re-typesets the named placement's text panel in place; `move_by: [x,y,z]` translates it (metres; declarative animation rides along); `tint: [r,g,b]` recolors it **per instance** (siblings of a shared prefab keep their color). |
| `spawn` | `{ builtin, position, scale?, color?, name?, animate?, anim_speed?, anim_amp? }` | conjure a builtin mesh at runtime. A **named** spawn registers as a placement the scene ABI can later move/tint; `animate` (`"spin"`/`"bob"`) gives it declarative idle motion. Browsers cap spawns per world (reference: 256). From Weft, spatial fields are `Fix` metres (`x y z scale speed amp`) or legacy integer centimetres. |

## 5. Host-fns (imports the module MAY call directly, synchronous, capability-gated)

```
host_log(ptr, len)                       // debug
host_now_ms() -> i64                     // world clock (not wall-clock; deterministic-friendly)
host_get_data(ptr, len) -> i64           // read a placement's data by name
host_emit(ptr, len)                      // fire an Action immediately (same schema as §4)
```

Everything with side effects (`buy`, `codex`, `navigate`, `presence`) goes through
**Actions**, not direct host-fns, so the host stays authoritative and the module
stays pure/testable. `host_emit` is sugar for returning a single action.

## 6. Capabilities & safety

- A module only receives events for placements bound to it, and only the host-fns
  above. No filesystem, network, clock (wall), or thread access.
- The host enforces per-frame CPU + memory limits and kills a misbehaving module
  (the world keeps rendering — **super-stable** tenet).
- Future (`0.2`): an explicit `capabilities[]` on the manifest behavior so a world
  declares intent (`commerce`, `presence`, `navigate`) and browsers can warn.

## 7. The two reference behaviors

- **`codex-viewer`** (archive): `on_interact` → `[{codex.open: {slug: data.slug}}]`.
- **`commerce/buy`** (market): `on_interact` → `[{commerce.buy: {item: data.item,
  price_ref: data.priceRef ?? data.item}}]`; on the `purchase_result` event →
  a merchant `notify` via `host_emit`. `price_ref` is a *catalog key* the
  server resolves the real price from — the displayed `data.price` is never
  sent (client prices are dead on arrival; pricing is server-authoritative).

## 8. Status

**Implemented (v0.51+):** the ABI core lives in `loom::behavior` (`Action`,
`InteractEvent`, `ActionList`, the `Behavior` seam), and a sandboxed WASM host in
`loom::behavior_wasm::WasmBehavior` (feature `behaviors`, pure-Rust `wasmi`):
JSON-over-linear-memory marshalling, the `thread_alloc`/`thread_on_*` exports and
`host_log`/`host_now_ms`/`host_emit` imports above, and **fuel-bounded calls** — a
trapping or runaway module yields no actions and the world keeps rendering.

**v0.53+:** browser wiring landed — the navigator dispatches interacts to bound
modules and the browser applies the returned actions; the `codex-viewer`
reference module is built and committed (`worlds/codex-archive`).

**v0.58+:** the async half — `thread_on_event` is loaded and called by
`WasmBehavior` (no return value; modules react through `host_emit`, per §3),
and the navigator exposes `dispatch_event` so the browser can answer with
`purchase_result` / `codex_ready`. The `commerce/buy` reference module (§7)
exists at `examples/behaviors/commerce` and is committed at
`worlds/market/behaviors/commerce.wasm` — The Bazaar's stalls sell through it.
`actor.passport_sub` is now populated by the navigator from the browser's
Passport. Both §7 reference modules are real.
