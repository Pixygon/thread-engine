# Passport — v0.1

**Status:** Draft (brainstorm-derived; wire details open until the main-agent reply) ·
**Layer:** Identity · **Owner:** Infinite (spec), main agent (Pixygon issuer implementation)

One portable identity that travels across the whole Thread. A person carries a
single Passport into every world on every host — no per-site signup, no per-site
login, no cookie banners. Worlds **reference** the Passport; they never own a copy.
Like every other Thread layer (hosting, manifest, presence), it is **federated,
self-hostable, and standardized** — Pixygon is one provider among many.

## 1. The two artifacts (normative)

A Passport is two things, and only two:

1. **The token** — a signed JWT the traveler presents. Compact, verifiable
   offline against the issuer's published keys, short-lived.
2. **The descriptor** — a fetchable JSON document describing who the traveler
   is (name, avatar, consent). Long-lived, mutable by the owner, always fetched
   fresh by hosts that need it.

Everything else — inventory, entitlements, history — stays wherever it lives
today and is *referenced* through the token's `sub`.

## 2. The token

A JWS (RS256/EdDSA) with these claims:

```
{ "iss": "https://api.pixygon.io/v1/passport",   // the issuing provider
  "sub": "did:pixygon:6981e8ed…",                 // DID-style stable id — THE identity
  "name": "Anders",                               // display name (informative)
  "avatar": "https://api.pixygon.io/v1/passport/avatar/did:pixygon:6981e8ed…",
  "scopes": ["presence", "commerce"],             // what the holder consented to (§5)
  "iat": 1789000000, "exp": 1789086400 }          // short-lived; re-issue is cheap
```

- **Verification is keys-only, cross-host.** A verifier fetches
  `<iss-origin>/.well-known/thread/jwks.json` and checks the signature. No shared
  secrets, no callback to the issuer. This is the federation seam: any host can
  run an issuer; any world can verify any issuer.
- `sub` is the identity. It MUST be stable for the lifetime of the account and
  MUST be DID-style (`did:<method>:<id>`), matching
  `infinite_manifest::Identity.id` — the same form worlds already use for
  authorship.
- Issuance is provider-specific (Pixygon: `POST /v1/passport/issue`,
  authenticated with the user's existing session). v0.1 does not standardize
  issuance — only the token and its verification.

## 3. The descriptor

`GET <avatar claim URL>` (or, for self-hosters,
`https://<your-host>/.well-known/thread/passport/<sub>.json`) returns:

```
{ "passport": "0.1",
  "identity": { "id": "did:pixygon:6981e8ed…", "name": "Anders" },
  "avatar": {                                    // avatar-v0.1 — the portable "you"
    "spec": {                                    // the SHARED AvatarSpec (slot → partId)
      "bodyHeight": 0.5,
      "parts": { "Body": 11010001, "Jacket": 123456 }
    },
    "colors": { "skin": "#c8956c", "hair": "#2a1f1a" },
    "assets": "https://api.pixygon.io/v1/avatar/assets"  // catalog base for partId→GLB
  },
  "consent": { … },                              // §5 — travels WITH the identity
  "home": "thread://pixygon.io/the-archive"      // optional: the traveler's home world
}
```

- **`spec` is the shared avatar model** — the exact `AvatarSpec` the Pixygon
  Unity package (`com.pixygon.avatar`) and web package (`@pixygon/avatar`)
  persist, so one saved avatar renders identically in a Unity game, the web
  Studio, and any Thread browser. Parts are keyed by **partId** and resolve to
  self-describing GLBs the same way world assets do (via `AssetSource`). The
  full schema — slots, the Portable Item Convention, catalog resolution — is
  **[avatar-v0.1](avatar-v0.1.md)** (companion spec); this section fixes only
  the envelope. (Issuers predating avatar-v0.1 may still send the legacy
  `base`/`worn` fields; readers keep accepting them.)
- The descriptor is fetched, rendered, and **discarded** — hosts MUST NOT store
  it beyond a session cache. This is what makes erasure work (§6).

## 4. Where it's presented

- **Presence `join`** — `{ t:"join", passport, spawn? }`; the relay verifies
  against the issuer's JWKS and renders co-travelers from their descriptors
  ([presence-wire §2](presence-wire-v0.1.md)).
- **Portal handoff** — crossing a veil to a *different host*, the browser
  presents the same token to the far host; verification is identical. Identity,
  avatar, and entitlement references cross with the traveler.
- **Behavior ABI** — a world's WASM sees only `actor.passport_sub` in its
  `InteractEvent` — the id, never the token, never the descriptor.
- **Commerce / Codex tiers** — same token, scoped by `scopes`.

A world that receives no Passport MUST still work: anonymous travelers get
presence-less, commerce-less, default-tier visits. Identity is an enhancement,
never a gate on walking.

## 5. Consent rides with the Passport

The GDPR/cookie layer, fixed at the identity instead of re-negotiated per site:

```
"consent": {
  "version": 3,                                  // bump on any change; hosts re-read
  "global": { "presence": true, "commerce": true,
              "analytics": false, "voice": false },
  "hosts": { "museum.example": { "analytics": true } },   // per-host overrides
  "updated": "2026-07-14T12:00:00Z"
}
```

- Scopes in the **token** are the intersection of the descriptor's consent and
  what the user granted at issue time — a host can trust the token alone.
- Hosts MUST honour the consent object as presented and MUST re-check on each
  session (the descriptor is fetched fresh; `version` makes staleness cheap to
  detect).
- Revocation is an edit to one document, by the owner, at the provider — not a
  hundred cookie-banner journeys.

## 6. Erasure ("delete me")

Because hosts hold **references, not copies**:

- Deleting the Passport at the provider makes the descriptor `410 Gone` and stops
  token issuance. Every world's reference goes dark at once.
- Hosts MUST treat a `410` descriptor as "render anonymous, purge session cache".
- Data a host created *about* the sub (scores, purchases) remains the host's
  GDPR obligation — but discovery is trivial: it's all keyed by one `sub`.

This is strictly easier than the old web, where erasure meant finding hundreds
of independent copies of you.

## 7. Trust

v0.1 is **open trust with local policy**: any issuer whose JWKS verifies is
*authentic*; whether a host *serves* it is the host's call.

- Default: accept any verifiable issuer (the open web posture).
- A host MAY pin an issuer allowlist (e.g. a private world for one community).
- Reputation/web-of-trust layers are explicitly deferred past v0.1.

## 8. Conformance

- A **conformant provider** publishes `/.well-known/thread/jwks.json`, issues
  tokens per §2, serves descriptors per §3, and honours erasure per §6.
- A **conformant host/relay** verifies tokens keys-only, fetches descriptors
  fresh, never persists them, honours consent (§5), and admits anonymous
  travelers (§4).
- A **conformant browser** holds the token for the traveler, presents it on
  join/handoff, renders others from their descriptors, and exposes the user's
  consent object for editing.

## Open for v0.2

Standardized issuance/OIDC bridge · avatar-v0.1 full schema · quantified
entitlement portability (inventory across hosts) · reputation on top of open
trust · user-held (wallet-style) storage with provider-less verification.

## Status

The **browser side of §8 is implemented** (2026-07-14): `loom::passport` holds
the token (`INFINITE_PASSPORT`), reads its claims, and fetches the descriptor
fresh per session (`Passport` for the traveler's own, `DescriptorPool` for
co-travelers'); Codex reads present the token as a bearer (tier unlock); the
reference relay reads `sub`/`name`/`avatar` from presented tokens and carries
them on the presence wire ([presence-wire v0.1.1](presence-wire-v0.1.md) —
`Occupant` identity + the `join` broadcast), so travelers render each other
from their descriptors. Still open operationally: the Pixygon issuer
(`POST /v1/passport/issue`), the published `jwks.json`, and keys-only
verification in hosted relays.

Related: [presence-wire](presence-wire-v0.1.md) ·
[presence-topology](presence-topology-v0.1.md) ·
[thread-protocol](thread-protocol-v0.1.md) ·
seed: [passport-vision](../thread/passport-vision.md).
