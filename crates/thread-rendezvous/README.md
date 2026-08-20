# thread-rendezvous

A **self-hostable reference P2P rendezvous** for the Thread. Run one anywhere and a
world can point its `presence.rendezvous` at it to become shared *serverlessly* —
peers meet here, exchange WebRTC signaling, then talk **directly**. No pose ever
transits this service, so it stays tiny, stateless, and nearly free to operate.

This is Tier 1 of [presence-topology-v0.1](../../docs/spec/presence-topology-v0.1.md):
the rendezvous introduces up to ~16 peers per room and gets out of the way. For
bigger crowds a world declares `presence.relays[]` (Tier 2 — see
[`thread-relay`](../thread-relay/)) and clients fall back automatically.

Like everything on the Thread, presence is **federated**: a world names its own
rendezvous in its manifest, so this is *one introducer among many* — no global
chokepoint. The standard is the wire format (`Signal`, §3.2), not any single server.

## Run it

```bash
# from source
cargo run -p thread-rendezvous --release

# with config
THREAD_RENDEZVOUS_ADDR=0.0.0.0:4100 thread-rendezvous
```

Clients connect to `…/rtc/<key>`; peers are grouped by that room key. The `/rtc`
path distinguishes a rendezvous from a relay's `/thread/<key>` when both share a
host.

### Docker

```bash
docker build -f crates/thread-rendezvous/Dockerfile -t thread-rendezvous .
docker run -p 4100:4100 thread-rendezvous
```

### TLS

The rendezvous speaks plain WebSocket (`ws://`); terminate TLS (`wss://`) at a
reverse proxy. Caddy, two lines:

```
presence.yourdomain.com {
    reverse_proxy /rtc/* localhost:4100
}
```

(A relay and a rendezvous can share one domain — route by path.)

## The wire, in one glance

```jsonc
{"t":"announce","peer":<u32>,"world":"<key>"}   // client → rv, on join (provisional id)
{"t":"welcome","id":<u32>}                      // rv → client: your assigned id (before peers!)
{"t":"peers","peers":[<u32>,…]}                 // rv → newcomer: who's here
{"t":"peers","peers":[<newcomer>]}              // rv → the room: join delta (required —
                                                //   the lower id offers, so existing
                                                //   members must learn of the newcomer)
{"t":"offer"/"answer"/"candidate", "from":…, "to":…, …}  // relayed verbatim to `to`
{"t":"leave","peer":<u32>}                      // rv → room, on a disconnect
```

## Certify it

The rendezvous certifies itself in its unit tests against the same public checker
anyone else would use, and you can probe a live instance:

```bash
cargo run -p thread-conformance -- --rendezvous ws://localhost:4100/rtc/smoke-test
```

The full end-to-end proof (two real WebRTC peers exchanging poses through an
in-process rendezvous) lives in Loom:

```bash
cargo test -p loom --features p2p --test p2p_live -- --ignored
```

## Point a world at it

```jsonc
"presence": {
  "mode": "p2p",
  "rendezvous": "wss://presence.yourdomain.com/rtc/yourdomain.com/world",
  "relays": ["wss://presence.yourdomain.com/thread/yourdomain.com/world"]  // optional fallback
}
```
