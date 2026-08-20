# Weft v0.1 — the native code of the Thread

Status: **exploratory draft** — reference implementation begun in
[`crates/weft`](../../crates/weft) (canonical encoding + content addressing,
verifier with transitive effect rows and static fuel bounds, contracts, and
the deterministic reference interpreter; first-order/non-recursive v0 core).
Part of the Thread spec suite; the sibling of [Behavior ABI](behavior-abi-v0.1.md),
which remains the polyglot floor.

> In weaving, the **warp** is the fixed thread the loom holds; the **weft** is
> what's woven through it to make cloth. Loom is the engine, the Thread is the
> medium — Weft is the code woven through them.

## 0. The one-line thesis

**The old web ships instructions and hopes; the Thread ships intent and
verifies.** Weft is not a nicer syntax or a faster bytecode — it is the
decision that *what crosses the wire stays semantic*: a typed, total,
effect-explicit, content-addressed program graph, verified as-is, compiled
locally by each browser into whatever its silicon runs best.

**Weft's target author is not a human — it is the agent.** Every prior
content-addressed language asked people to abandon source text and was right
but lonely for it. Weft doesn't ask that of anyone: it is designed for the
world where software factories write the code — no source text to bikeshed,
contracts as the durable artifact, hash-identity packages verified locally,
agents exchanging contract deltas instead of diffs. What looks like Weft's
biggest weakness ("nobody will write this by hand") is its founding
advantage: **nothing here asks a human to.** Humans keep first-class read
access — faithful projections, audits, provenance — while the ground truth
stays machine-checkable. A medium whose places are built by agents needs a
code format agents can trust sight-unseen; that format is Weft.

## 1. Principles

1. **Human-optional, machine-verifiable.** Weft has no source text. The
   canonical form is a typed graph; names are metadata; human-readable
   projections (any style, any natural language) are derived on demand and
   guaranteed faithful because they are renderings, never sources. Humans lose
   write access to the ground truth, not read access — audit is a first-class
   right (commerce and Passports demand it).
2. **The spec is the source; the implementation is derived.** Every definition
   carries its contract (types, effects, pre/postconditions) as the durable
   artifact. Implementations are disposable and regenerable; agents exchange
   contract deltas, not diffs.
3. **The nervous system, not the muscles.** Weft reacts to events and returns
   intent. It never sits in a per-frame or per-vertex path — rendering,
   physics, and audio belong to the browser's native engine loop. This is a
   normative rule, not advice: a conformant browser MUST NOT require Weft
   evaluation to produce a frame. (This is *why* the Thread out-renders
   code-driven scenes: content is data, and the hot loop is native.)
4. **Verify once, trust by hash.** Verification (types, totality, effects,
   contracts) is a property of the content-addressed artifact. Once any party
   has verified hash `H`, the result is cacheable and shareable forever.
5. **WASM stays as the floor.** Any language can still target the Thread via
   the Behavior ABI. Weft is the native path, never a wall.

## 2. Three roles, separated

| Role | Old web | Thread |
|---|---|---|
| Distribution format | JS text / WASM binary | **Weft graph** (canonical encoding) |
| Verification format | none / WASM validation (byte-soup level) | **the same Weft graph** — types, effects, contracts intact |
| Execution format | JIT-internal | **backend-chosen**: reference interpreter → native → parallel/GPU |

WASM's fatal compromise for our purposes is linear memory: one flat mutable
byte array. Everything Weft is for — locality, contracts, effect types,
content addressing — is erased by lowering to byte soup before the trust
boundary. Weft refuses to lower until *after* verification, on the receiving
machine.

## 3. The core calculus

A Weft **module** is a set of content-addressed **definitions**. A definition
is a pure, **total** function represented as a typed term graph.

**Node kinds (v0.1):**
- **Literals** — integers, booleans, fixed-point rationals, text, byte blobs.
- **Data** — algebraic constructors and pattern matching (sums + products;
  records with structural typing).
- **Pure operations** — arithmetic/logic/comparison on the deterministic core
  (§4), text/blob/list/map primitives.
- **Calls** — reference another definition **by hash** (never by name).
- **Effect requests** — reified as data (§5); constructing one performs
  nothing.
- **Fold** — the only recursion: structural recursion over finite data, or
  explicitly fuel-bounded iteration. General recursion is not expressible;
  every Weft evaluation terminates, and its cost is statically boundable per
  event.

**The behavior model is event-driven dataflow.** A Weft behavior is not a
program with a `main`; it is a function
`(State, Event) → (State, [Action])` — the same shape the Behavior ABI already
fixed, now with the whole body verifiable. Computation per event is bounded by
the graph; there is no unbounded loop in the hot path, by construction.

## 4. The determinism boundary

The Weft core is **bit-deterministic across hosts**: integers, fixed-point
arithmetic, and deterministic data structures only. IEEE floats — whose
results legally vary across hardware — are quarantined at the rendering edge
(positions handed to the engine may become floats *after* Weft is done with
them).

Determinism is not pedantry; it is the unlock for everything spatial-native:
- **The network is the memoization table** — any host's result for
  `eval(H_fn, H_args)` is valid for every host, cacheable by hash, shareable
  like an asset over the same CDN that serves meshes.
- **Behaviors migrate.** A behavior's state snapshot is content-addressed
  data; it can veilwalk between hosts the way a traveler does, resuming
  bit-exactly.
- **Time is an axis for computation too.** Deterministic replay from an event
  log means a world's computational state is addressable along the timeline —
  `@when` for code, the founding move of the Thread applied to its programs.

## 5. Effects and capabilities

Every definition carries an **effect row** — the set of effect kinds it may
request (`codex.open`, `commerce.buy`, `navigate`, `notify`, `presence.emit`,
`state.set`, …: the Behavior ABI's Action vocabulary, now as types).

- Constructing an effect is pure; only the **browser** performs effects, after
  the event returns — exactly the sandbox inversion the Thread already runs.
- **The effect row IS the permission system.** A behavior's declared effects
  must be a subset of what its manifest entry grants; the checker enforces it
  at verification time and the browser at runtime. Language type, manifest
  permission, and conformance clause are one object. A behavior whose row
  excludes `commerce.buy` *cannot spend a visitor's gold* — not "is trusted
  not to", cannot express it.
- No ambient authority anywhere: capabilities arrive as values in the event.

## 6. Contracts

Every definition MAY attach preconditions, postconditions, and state
invariants — as Weft terms (pure, total, evaluable). Tiers of assurance,
cheapest first, all cacheable by hash:

1. **Checked**: types + totality + effect rows (mandatory, decidable).
2. **Tested**: property tests auto-derived from contracts, run by any party.
3. **Proven**: machine-checkable proof objects attached alongside (optional;
   format deferred to v0.2 — the slot is reserved now so proofs can arrive
   without a wire break).

A browser MAY refuse behaviors below a tier for sensitive effects (a world can
demand tier-3 for `commerce.*`).

## 7. Content addressing

- The canonical encoding is a deterministic serialization of the type-annotated
  graph (definitions sorted, hashes as links). `weft:<hash>` names a
  definition forever.
- **Names are metadata**: a separate, editable name↔hash index for search and
  projection. Renames touch no code. Two "versions" of a function are just two
  hashes; dependency hell is structurally impossible.
- **A patch is not a diff.** A change ships as: new hashes + an **intent
  record** — either an equivalence claim (machine-checkable: the contract is
  unchanged and tests/proofs carry over) or a contract delta (the thing agents
  actually negotiate about).

## 8. Concurrency: parallel by default, in the only durable sense

JavaScript is single-threaded **by semantic contract** — unfixable. WASM
threads reintroduce shared memory and data races inside the sandbox. Weft
takes the third road:

- No shared mutable state exists in the language. Any two subgraphs without a
  data dependency MAY be evaluated in any order or simultaneously, and MUST
  yield bit-identical results (confluence). **Data races are not a bug class;
  they are unexpressible.**
- Therefore the program never says "thread." Parallelism is a **backend
  decision**: the v0.1 reference interpreter is serial and correct; a
  work-stealing backend parallelizes the same artifact with zero changes; an
  interaction-net or GPU backend (§9) more still. Weft programs are
  permanently positioned to soak up whatever cores exist — hardware gets wider
  every year, and no source ever needs revisiting.
- Across behaviors: each behavior's event handling is a transaction on its own
  state; distinct behaviors are trivially concurrent.

## 9. Execution backends

The backend contract, for any implementation:
1. Consume verified canonical graphs; never re-derive trust.
2. Preserve core determinism (§4) bit-exactly.
3. Meter **fuel**: an implementation-independent cost unit (graph-rewrite
   count in the reference semantics), with per-event budgets — the same
   super-stable posture the WASM sandbox has today (a runaway behavior yields
   nothing; the world keeps rendering).

Planned ladder: **reference interpreter** (correctness oracle; ships first) →
native compilation on arrival (the browser "uploads" code like a mesh) →
**parallel/graph-reduction backends** (interaction-net style, GPU-capable) as
that substrate matures. The wire format never changes as the ladder is climbed.

## 10. Manifest integration

`behaviors[]` entries gain a kind: `{ "id": …, "weft": "<asset-id | weft:hash>",
"on": [...], "effects": [...] }` beside the existing `wasm` form. The declared
`effects` array must equal the artifact's checked effect row. Everything else
(events, `on`, per-placement binding) is unchanged — Weft slots into the seam
the Behavior ABI already cut.

## 11. Conformance (sketch)

- W1: canonical encoding round-trips; hash agrees across implementations.
- W2: verifier rejects: non-total terms, effect-row violations, type errors.
- W3: reference-interpreter differential: all implementations produce
  bit-identical results and fuel counts on the corpus.
- W4: browser never blocks a frame on Weft evaluation (§1.3).
- W5: effect enforcement: an undeclared effect request is dropped and reported.

## 12. Benchmarks (see docs/weft/benchmarks.md)

Claims in this spec that MUST be earned with published numbers, not asserted:
render advantage of data-shipped worlds vs. code-driven scenes (measured in
frame time, vsync off, escalating load); behavior throughput vs. the wasmi
floor; and parallel scaling with cores. The benchmark harness is part of the
project, and results ride with the spec.

## 13. Open questions (v0.2 agenda)

Proof-object format (§6.3) · the projection standard (how renderings cite
hashes) · interaction-net operational semantics as the normative cost model ·
inter-behavior message effects · the Weft ↔ Codex link (contracts that cite
canon).


## Appendix: v0.1.1 — fixed-point (`Fix`)

Weft's answer to floating point is **not** floating point. `Fix` is a second
numeric type: an i64 counting **millionths** (`FIX_SCALE = 1_000_000`), so
`1.5` is `Fix(1_500_000)`. Every property the calculus promises survives:

- **Exact**: `0.1 +. 0.2` is *exactly* `0.3` — there is no representation
  error to accumulate, no platform variance, no NaN, no rounding modes.
  Multiplication and division run through 128-bit intermediates, then
  rescale; division by zero is `0` (total, like `Int`).
- **Deterministic**: the same weave yields the same `Fix` bits on every
  host — fixed-point is integer arithmetic wearing a decimal point.
- **Typed apart**: `Fix` and `Int` never mix silently. The only bridges are
  `fix` (`FixOfInt`) and `trunc` (`IntOfFix`); adding a `Fix` to an `Int` is
  a verification error.

New prims (appended — encodings never reorder): `FAdd FSub FMul FDiv`
(`Fix×Fix→Fix`), `FLt FLe EqFix` (`→Bool`), `FixOfInt`, `IntOfFix`,
`FixToText` (canonical decimal text: `"1.5"`, `"-0.25"`; no exponents, no
trailing zeros). New term literal: `Fix(raw)` (tag 12); new type tag: 6.
Projection renders `Fix` literals as decimals and the fixed ops as dotted
operators (`+.`, `<.`, …).

At the effect seam, `Fix` fields cross to the browser as plain JSON numbers
(millionths ÷ 10⁶ — exact in f64 at these magnitudes). The `spawn` effect
prefers `Fix` metres (`x`/`y`/`z`/`scale`, plus `speed`/`amp` for its
declarative `animate`); the older integer-centimetre fields remain valid.
