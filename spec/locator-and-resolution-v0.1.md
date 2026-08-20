# Locator & Resolution — v0.1

**Status:** Draft · **Layer:** Addressing (the URL + DNS of the Thread) · **Reference impl:** `src/resolver.rs`

How a `thread://` address is written, and how a browser turns it into a world —
**including the decentralized path that lets anyone host a world on their own
domain with zero registry and zero contact with Pixygon.**

## 1. The Locator (grammar)

```
thread://<host>/<path>[@<when>][#<place>]
```
- `host` — the world host authority (a domain).
- `path` — the world's path on that host (may be empty → the host's root world).
- `@when` — OPTIONAL timeline year to arrive at.
- `#place` — OPTIONAL named anchor (a `spawn.name` or `portal.id`).

`@when` and `#place` are **client-side** navigation *within* a resolved world; they
never affect which document is fetched.

### 1.1 The scheme names the medium, not the transport (normative)

**Worlds are addressed `thread://`, never `https://`.** A scheme names
*resolution + interaction semantics*: `thread://example.com` says "this names a
place; resolve it by Thread rules (§2); expect a world, presence, time, identity."
That HTTPS carries the bytes underneath (§3) is plumbing — early cars drove on
horse roads. Owning the scheme decouples the identity of places from the transport
du jour: when transfer evolves (WebTransport/QUIC, IPFS, whatever comes), every
`thread://` address survives unchanged. Addresses are forever; transports are a
decade.

Consequences:
- Every user-facing surface — documentation, browser UI, share sheets, portals,
  QR codes — shows `thread://` Locators. **An `https://` URL presented as a
  world's address is a bug.** (The one legitimate `https://` face of a world is a
  *gateway* page, which exists precisely for old-web contexts and degrades a
  Locator gracefully into them.)
- Implementations SHOULD register `thread://` with the host OS (Windows registry,
  macOS `CFBundleURLTypes`, Linux `x-scheme-handler/thread`) so Locators in
  old-web surfaces (chat, email, documents) open a Thread browser directly.
- Second implementers MUST NOT leak resolution URLs (§3's `https://…/world.json`)
  into user space; they are internal to the resolution step.

## 2. Resolution algorithm (normative)

Given a Locator, a conformant browser resolves it in this order, using the first
that yields a valid manifest:

1. **Registry / resolver** (OPTIONAL, a value-add). If the browser is configured
   with a resolver, `GET <resolver>/thread/resolve?loc=<url-encoded locator>` →
   `{ world, manifestUrl, assetBase, presenceRelay?, codexBase?, passportIssuer? }`;
   then `GET manifestUrl`. Registries provide discovery, presence, and identity —
   but a browser MUST NOT *require* one.
2. **`.well-known` (decentralized, no registry).** Fetch the host's own manifest
   directly (§3). This is the path that makes the Thread an open web: it needs
   nothing but static hosting on the author's domain.
3. **Local** (dev/offline) — an implementation MAY resolve to a local file.

A browser MUST implement at least step 2. Steps 1 and 3 are optional conveniences.

## 3. The `.well-known/thread` convention (self-hosting)

To publish a world on `example.com` with **no registry**, serve a manifest at:

```
https://<host>/.well-known/thread[/<path>]/world.json
```
- `thread://example.com`            → `https://example.com/.well-known/thread/world.json`
- `thread://example.com/gallery`    → `https://example.com/.well-known/thread/gallery/world.json`

A host MAY instead (or additionally) serve **Thread markup** — the `.thread`
source form (see thread-markup-v0.1) — at the same location with the
`world.thread` filename. Browsers MUST try `world.json` first and fall back to
`world.thread` when the JSON is absent; both forms compile to the same World
Manifest, exactly as a web server may serve HTML rather than a DOM dump.

**Host requirements (normative):**
- Serve the manifest over **HTTPS**.
- `Content-Type: application/json` (`text/plain` or another `text/*` type for a
  `world.thread` markup source).
- **`Access-Control-Allow-Origin: *`** — browsers fetch cross-origin; without CORS
  the world won't load.
- The body MUST be a conformant `thread/0.x` World Manifest (validates against the
  reference validator).
- Relative asset URIs in the manifest resolve against the manifest's directory
  (`https://<host>/.well-known/thread[/<path>]/`). Absolute / `ipfs://` URIs pass
  through. Assets SHOULD also be served with permissive CORS.
- Portals to other hosts just carry those hosts' Locators — no coordination needed.

That's the whole contract. A `world.json` + assets on any static host = a place on
the Thread, reachable by any browser, forever, with zero contact with anyone.

## 4. Registry (optional value-add)

A registry (e.g. Pixygon's `GET /v1/thread/resolve`) MAY:
- resolve **its own** registered worlds (canonical `thread://` names, minted ids),
- **fall back** to a host's `.well-known` for unregistered hosts (server-side),
- attach discovery/presence/identity endpoints to the response.

None of this is required to author, host, or visit a world. The registry is Chrome's
Google-account layer, not the web.

## 5. Conformance

- A **conformant host** serves a valid manifest at the `.well-known` path with
  HTTPS + JSON + CORS `*`.
- A **conformant browser** resolves a bare `thread://host[/path]` via `.well-known`
  with no registry configured, applies `@when`/`#place` client-side, and renders
  the manifest. `thread doctor <host>` (tooling) checks a host end to end.
