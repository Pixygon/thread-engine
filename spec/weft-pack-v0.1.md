# weft-pack v0.1 — packages for a content-addressed language

Status: **draft**. Reference implementation: `weft::pack` (crates/weft) +
the `weftpack` CLI (crates/weft-pack). First published package:
`packages/weft-form` (geometry vocabulary — the three.js seed).

## 1. The thesis

npm distributes *names that move*; weft-pack distributes **hashes that
can't**. Because a Weft definition's identity is the SHA-256 of its
canonical bytes (weft-v0.1 §7), the entire package problem collapses:

| npm's problem | weft-pack's answer |
|---|---|
| left-pad (unpublish breaks the world) | a hash can't be unpublished out from under you — anyone holding the bytes holds the truth, and the bytes verify |
| version conflicts / diamond dependency | linking is a **set union** of hash-keyed defs; the same def twice is one entry, different defs are different hashes — there is nothing to resolve |
| typosquatting / hijacked names | names are **petnames** — authoring sugar mapping to hashes; the wire and the linked module carry only hashes |
| registry trust / supply-chain injection | **verification is local**: types, effect rows, contracts, fuel bounds are re-checked by the consumer's verifier, whoever served the bytes |
| install scripts, postinstall malware | Weft has no IO; a package is inert data until a browser evaluates it inside the sandbox, under its declared effect row |

A **registry is a static file host** — a directory, a CDN, or a Thread host's
`.well-known/weft/`. There is no publish endpoint to secure and no account
system to breach.

## 2. The package format

`<name>.weftpack.json`:

```json
{
  "name": "weft-form",
  "exports": { "ring": "weft:9e3d…", "lerp": "weft:834f…" },
  "defs": { "weft:9e3d…": { …Def… }, … }
}
```

- `name` — informational; identity lives in the hashes.
- `exports` — petname → def hash. MUST point into `defs`.
- `defs` — every definition the exports transitively need, keyed by hash.

**Self-verifying**: a conformant tool MUST refuse a package where (a) any
def's key ≠ the hash of its bytes, (b) any export dangles, or (c) the def
set fails full weft verification. `weftpack fetch` verifies **before** the
bytes touch disk.

## 3. Linking

`link(packages, local_defs, entry)` unions all def maps (hash-keyed — no
order, no conflicts), inserts the consumer's own defs, trims to the entry's
transitive call closure, and re-verifies the result as a module. The output
is an ordinary weft Module: consumers of the module cannot tell packages
were involved.

## 4. The CLI

```
weftpack verify <pkg.weftpack.json>            local trust decision
weftpack show   <pkg> <petname>                projection to human text
weftpack link   <pkg…> --entry <petname> -o m  runnable module from an export
weftpack fetch  <url> [-o file]                pull + verify from any host
```

## 5. weft-form — the first package

The seed of the Thread's three.js: geometry vocabulary over `Fix`, built on
weft-v0.1.1's `Iota`/`Map`/`Fold` and the deterministic integer `sin`/`cos`.
Exports: `lerp`, `ring-point`, `ring`, `grid-point`, `grid`. The Atrium's
clock consumes it — its 50-second mote ring is `ring(12, 6.0)` called by
hash — the first Weft behavior built on a Weft package.

## 6. Publishing on the Thread

Serve the file. Conventions, in preference order:

1. **wpm** — the reference registry: `weftpack publish <pkg> --registry
   https://wpm.pixygon.io` (`POST /publish`; the server runs the full weft
   verifier before storing anything). Fetch via
   `https://wpm.pixygon.io/packages/<name>.weftpack.json`; index at `/`.
2. `https://<host>/.well-known/weft/<name>.weftpack.json`
3. any URL ending `.weftpack.json` (CDN, repo raw, IPFS gateway)

wpm (github.com/Pixygon/wpm) is a *directory of hashes*, never an authority
over content: it verifies what it stores, mirrors are equals, and every
consumer re-verifies locally. If wpm vanished tomorrow, nothing published
would break.
