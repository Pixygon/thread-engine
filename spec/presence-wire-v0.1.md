# Presence Wire — v0.1

**Status:** Draft · **Layer:** Presence · **Owner:** main agent (relay), co-designed with Infinite (client interpolation)

The wire format for shared presence on the Thread. Transport: **WebSocket** first
(`wss://<relay>/thread/:worldId`), path to WebTransport/QUIC later. This spec fixes
the coordinate frame, the pose message, and the timing model so the relay and any
browser interpolate identically.

## 1. Coordinate frame (normative)

Match the engine + glTF: **right-handed, +Y up, meters**. Rotation is a unit
**quaternion `[x, y, z, w]`**. This is the same frame the World Manifest uses, so
poses need no conversion.

## 2. Session

- `join` (client→relay): `{ t:"join", passport, spawn?: string }` — relay verifies
  the Passport against the issuer `jwks.json`, assigns an occupant `id` (u32), and
  replies `welcome { id, occupants:[Occupant], tick_hz }`.
- **`Occupant` is `{ id, sub?, name?, avatar? }`** — the identity claims the relay
  read from that occupant's Passport (`sub` the stable id, `name` the display
  name, `avatar` the descriptor URL per passport-v0.1 §3). All three absent for
  an anonymous traveler. This is what lets a browser fetch a co-traveler's
  descriptor and render *them* instead of a placeholder (§6).
- `join` (relay→others): `{ t:"join", id, sub?, name?, avatar? }` — broadcast to
  the world when someone joins, so identity precedes their first pose (poses
  carry only an id). Not AoI-gated, like `leave`.
- `leave` (either way): `{ t:"leave", id }`.
- `observe` (client→relay): `{ t:"observe" }` — **watch without entering.** The
  relay replies with the same `welcome { occupants, tick_hz }` a joiner gets
  (plus `observer: true`, `id: 0`) and then forwards every `join` and `leave`
  for that world. An observer is **not an occupant**: it never appears in
  anyone's `welcome.occupants[]`, is never counted, is never sent poses,
  voice or interactions, and its disconnect broadcasts **no** `leave` —
  nothing of it was ever in the world. This is what a status bar, a
  dashboard, a directory's "who's home" light or a world's own front page
  needs; without it, every such reader has to `join`, and a phantom body
  appears standing at the origin.

- **Identity comes from the Passport and nowhere else.** The relay reads `sub`,
  `name` and `avatar` from the presented Passport's claims; a top-level `name`
  (or `sub`, or `avatar`) in the `join` frame is **not** honoured. It isn't
  rejected either — unknown fields are ignored, per §1 — so sending
  `{ t:"join", passport:"", name:"ada" }` connects successfully and produces an
  *anonymous* occupant, which is the correct behaviour and a quiet one. An
  implementer who wants a display name without an issuer has to render it
  locally; the wire has no place to put an unattested one, deliberately, since
  a name a relay repeats without verifying is a name anyone can claim.

- The relay is authoritative on the occupant list and does area-of-interest culling
  (only relay poses within a radius / same sub-area).

## 3. Pose (the hot path)

Clients send at **10–20 Hz**; the relay fans out (optionally batched). Wire fields:

```
{ t:"pose", id:u32, ts:u32,          // ts = relay-epoch milliseconds (u32 wraps ~49d; fine)
  p:[f32;3],                          // position (m)
  r:[f32;4],                          // rotation quaternion xyzw (MAY send yaw-only via `y:f32` for bandwidth)
  v:[f32;3],                          // linear velocity (m/s) — enables extrapolation
  a:u8 }                              // animation/locomotion state enum (idle/walk/run/jump/…)
```

Bandwidth profile (v0.1 keeps it simple; quantization is an optional relay
capability, not required for conformance):
- **Full:** floats as above (~40 B/pose).
- **Quantized (optional):** `p` as int16 cm relative to an area origin; `r` as
  smallest-three (3×int16 + 2-bit index); `v` as int8 dm/s. Negotiated in `welcome`.

## 4. Timing & interpolation (client contract)

- The relay stamps every pose with `ts` on receipt (single clock source; clients
  never trust each other's clocks).
- Clients render presence **~100 ms in the past** (interpolation buffer), lerping
  position and **slerping** rotation between the two poses that bracket
  `render_time = latest_ts - 100ms`.
- On a gap (missing pose), **extrapolate** with `v` for up to ~250 ms, then freeze.
- The local player is never interpolated (rendered from live input).

This is why `v` and a server `ts` are mandatory: they're exactly what the client
needs to hide jitter without trusting peer clocks.

## 5. Interactions & voice

- `interact` (client→relay→others): `{ t:"interact", id, target, action, data }` —
  mirrors the Behavior ABI Action `presence.emit`, so a world's WASM can broadcast
  a gameplay event (a lever pulled, an item shown) to co-present travelers.
- **Voice** (default-on where presence is on — speaking is as native to a shared
  place as walking): voice travels as **binary WebSocket frames** beside the JSON
  text messages, so the hot audio path never touches JSON.
  - **Client → relay:** the raw payload — **16 kHz mono PCM16 little-endian,
    20 ms frames** (320 samples / 640 bytes) in v0.1. A future codec field
    upgrade (Opus) will be a new wire minor; PCM16 stays the floor every
    implementation can speak.
  - **Relay → others:** the same payload prefixed with the **sender's occupant
    id as a `u32` LE** (4 bytes), fanned out to the sender's world within the
    SAME area-of-interest as poses — **a voice carries exactly as far as
    presence does**. Never echoed to the sender. Frames from connections that
    haven't `join`ed, and frames over **8 KiB**, are dropped.
  - **Receiver contract:** attenuate each speaker by distance to their last
    interpolated `p` (inverse-square-ish; conversational range ~2 m full,
    inaudible ~26 m), mix locally, and derive a smoothed amplitude per speaker
    to animate their avatar's mouth. A client with no audio device simply
    doesn't send/mix — voice is never required to walk.
  - Muting is client-side (stop sending); worlds opt out entirely with
    `presence.voice=false` in the manifest.

## 6. Conformance

- A **conformant relay** accepts `observe` and keeps observers out of the
  occupant list, the occupant count, and every fan-out; verifies Passports on
  `join`, stamps `ts`, maintains the
  occupant list, and fans out `pose`/`interact` within area-of-interest.
- A **conformant browser** buffers ~100 ms, interpolates per §4, and renders other
  travelers' avatars from their Passport descriptor (`GET /v1/passport/avatar/:sub`).
- Presence is **optional** (per the manifest): no `presence.relay` → solo, no wire
  traffic at all.

**v0.1.1 (2026-07-14):** fixed the `Occupant` shape (`{ id, sub?, name?, avatar? }`)
and added the relay→others `join` broadcast, so identity crosses the wire and §6's
descriptor rendering is implementable. Both changes are additive: an anonymous
traveler is `{ id }`, exactly the old shape, and clients ignore unknown tags.
Reference: [`thread-relay`](../../crates/thread-relay) (relay side),
`loom::presence` (client side).
