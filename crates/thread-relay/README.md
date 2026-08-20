# thread-relay

A **self-hostable reference presence relay** for the Thread. Run one anywhere and a
world can point its `presence.relay` at it to become shared and multi-user.

Presence on the Thread is **federated**: a world names its own relay(s) in its
manifest, so this is *one relay among many* — no global chokepoint, and presence
keeps working no matter which operator disappears. This binary is the reference
anyone can run; the standard is the [wire format](../../docs/spec/presence-wire-v0.1.md),
not any single server.

## Run it

```bash
# from source
cargo run -p thread-relay --release

# with config
THREAD_RELAY_ADDR=0.0.0.0:4000 \
THREAD_RELAY_AOI=80 \            # area-of-interest radius in metres (0 = unlimited)
THREAD_RELAY_TICK=15 \          # suggested client send rate, advertised in `welcome`
thread-relay
```

Clients connect to `…/thread/:worldId`; occupants are grouped by that world id.

### Docker

```bash
docker build -f crates/thread-relay/Dockerfile -t thread-relay .
docker run -p 4000:4000 -e THREAD_RELAY_AOI=80 thread-relay
```

### TLS

The relay speaks plain WebSocket (`ws://`) and stays lean; terminate TLS (`wss://`)
at a reverse proxy. Caddy, two lines:

```
presence.yourdomain.com {
    reverse_proxy 127.0.0.1:4000
}
```

Now worlds can use `wss://presence.yourdomain.com/thread/<worldId>`.

## Verify it conforms

The relay certifies against the same checker any browser author would run:

```bash
thread-conformance --relay ws://127.0.0.1:4000/thread/plaza
```

## How it scales

- **Area-of-interest culling** (`THREAD_RELAY_AOI`) — a pose is only fanned to
  occupants within the radius, so a world with thousands present still only
  exchanges with the handful nearby. This is the main scale lever.
- **Stateless in spirit** — the relay holds only ephemeral occupant positions; a
  restart just means clients reconnect. Nothing durable to lose or back up.
- Bigger worlds shard by area (run an instance per cell); cross-relay federation for
  a single huge world is a later addition (the wire is designed to allow it).

## Conformance notes

Per [presence-wire-v0.1](../../docs/spec/presence-wire-v0.1.md), a conformant relay
assigns occupant ids on `join`, stamps every `pose` with a server `ts`, maintains
the occupant list, and fans out within area-of-interest. This reference relay does
all of that. It runs **open** (no Passport verification) by default for dev and
self-hosting; a production deployment verifies Passports against the issuer's
`jwks.json` (extend `on_join`, or gate at the proxy).

Identity still crosses the wire in the open posture: the relay *reads* each
presented Passport's `sub` / `name` / `avatar` claims (unverified) and carries
them in `welcome.occupants[]` and a `join` broadcast to the room, so browsers
can fetch co-travelers' descriptors and render *them* — names and looks — from
their own Passports. Anonymous travelers are plain `{ id }`, as before.
