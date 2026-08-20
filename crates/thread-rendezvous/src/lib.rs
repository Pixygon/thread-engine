//! The rendezvous **Room** hub — the socket-free core of a Thread P2P rendezvous.
//!
//! A rendezvous is even simpler than a relay: it introduces peers and relays their
//! WebRTC signaling (`offer`/`answer`/`candidate`) *blindly* — it never sees a pose
//! and holds no durable state ([presence-topology-v0.1] §3.2). Restart-safe, cattle
//! not pets. All the wire behaviour lives here as a pure state machine —
//! `handle(inbound) -> [outbound]` — so it's unit-testable with no network, exactly
//! like `thread-relay`'s `Hub`.
//!
//! [presence-topology-v0.1]: ../../../docs/spec/presence-topology-v0.1.md
//!
//! Wire contract (all messages JSON, tagged `"t"` lowercase — [`loom::mesh::Signal`]):
//! - client → rv `announce{peer,world}`: join. The rendezvous **assigns** a
//!   collision-free id and replies `welcome{id}` **before** `peers{…}` — the
//!   ordering contract clients rely on to adopt their id before negotiating.
//! - rv → newcomer `peers{peers:[…]}`: everyone already present.
//! - rv → each existing occupant `peers{peers:[<newcomer>]}`: a **delta** — required,
//!   or nobody would ever offer to the newcomer (the glare rule makes the *lower* id
//!   initiate, and assigned ids only grow).
//! - `offer`/`answer`/`candidate`: relayed **verbatim** to the peer named in `to`,
//!   same room only. The rendezvous never parses the SDP.
//! - `leave{peer}`: broadcast to the room on departure or disconnect.
//!
//! Decentralization by design: this is a *reference* rendezvous anyone can
//! `cargo run` (or `docker run`) on a cheap host. A world names its own rendezvous
//! in its manifest (`presence.rendezvous`); Pixygon runs at most one among many.

use std::collections::HashMap;

use serde_json::{json, Value};

/// A connection handle assigned by the transport layer (one per WebSocket).
pub type ConnId = u64;

/// A peer id on the mesh wire (u32, server-assigned here).
pub type PeerId = u32;

/// An event coming *into* the hub.
pub enum Inbound {
    /// A socket connected to room `key` (from the `…/rtc/<key>` path).
    Connect(ConnId, String),
    /// A raw wire message arrived on a connection.
    Text(ConnId, String),
    /// A socket closed.
    Disconnect(ConnId),
}

/// A message the hub wants sent to a specific connection.
pub struct Outbound {
    pub conn: ConnId,
    pub text: String,
}

struct Member {
    id: PeerId,
    room: String,
}

/// The rendezvous hub: rooms, members, and the signaling state machine.
#[derive(Default)]
pub struct Hub {
    next_id: PeerId,
    /// Which room a connection belongs to (from its URL path), set on `Connect`.
    conn_room: HashMap<ConnId, String>,
    /// Announced members, keyed by connection.
    members: HashMap<ConnId, Member>,
}

impl Hub {
    pub fn new() -> Self {
        Hub {
            next_id: 1,
            conn_room: HashMap::new(),
            members: HashMap::new(),
        }
    }

    /// Current number of announced members across all rooms.
    pub fn member_count(&self) -> usize {
        self.members.len()
    }

    /// Advance the state machine by one event, returning messages to send.
    pub fn handle(&mut self, ev: Inbound) -> Vec<Outbound> {
        match ev {
            Inbound::Connect(conn, room) => {
                self.conn_room.insert(conn, room);
                Vec::new()
            }
            Inbound::Text(conn, raw) => match serde_json::from_str::<Value>(&raw) {
                Ok(v) => self.on_message(conn, &v),
                Err(_) => Vec::new(), // ignore malformed frames (robustness)
            },
            Inbound::Disconnect(conn) => {
                self.conn_room.remove(&conn);
                self.remove_member(conn)
            }
        }
    }

    fn on_message(&mut self, conn: ConnId, v: &Value) -> Vec<Outbound> {
        match v.get("t").and_then(Value::as_str) {
            Some("announce") => self.on_announce(conn),
            Some("offer") | Some("answer") | Some("candidate") => self.relay_to(conn, v),
            Some("leave") => self.remove_member(conn),
            _ => Vec::new(),
        }
    }

    fn on_announce(&mut self, conn: ConnId) -> Vec<Outbound> {
        let Some(room) = self.conn_room.get(&conn).cloned() else {
            return Vec::new();
        };
        if self.members.contains_key(&conn) {
            return Vec::new(); // double-announce: already introduced
        }
        // Assign a collision-free id — the client's provisional announce id is
        // discarded (it MUST adopt the welcome id; see loom::mesh::Signal docs).
        let id = self.next_id;
        self.next_id += 1;

        let existing: Vec<(ConnId, PeerId)> = self
            .members
            .iter()
            .filter(|(_, m)| m.room == room)
            .map(|(c, m)| (*c, m.id))
            .collect();

        self.members.insert(conn, Member { id, room });

        // Ordering contract: `welcome` lands before `peers`, so the newcomer has
        // its assigned id before any offer/answer is negotiated.
        let mut outs = vec![
            Outbound { conn, text: json!({ "t": "welcome", "id": id }).to_string() },
            Outbound {
                conn,
                text: json!({ "t": "peers", "peers": existing.iter().map(|(_, id)| *id).collect::<Vec<_>>() })
                    .to_string(),
            },
        ];
        // The delta to everyone already present — without it the newcomer (highest
        // id) would wait forever, since the glare rule makes the lower id offer.
        let delta = json!({ "t": "peers", "peers": [id] }).to_string();
        outs.extend(existing.iter().map(|(c, _)| Outbound {
            conn: *c,
            text: delta.clone(),
        }));
        outs
    }

    /// Relay a signaling message verbatim to the member named in `to`, same room.
    fn relay_to(&self, conn: ConnId, v: &Value) -> Vec<Outbound> {
        let Some(sender) = self.members.get(&conn) else {
            return Vec::new(); // must announce before signaling
        };
        let Some(to) = v.get("to").and_then(Value::as_u64).map(|n| n as PeerId) else {
            return Vec::new();
        };
        self.members
            .iter()
            .filter(|(c, m)| **c != conn && m.room == sender.room && m.id == to)
            .map(|(c, _)| Outbound {
                conn: *c,
                text: v.to_string(),
            })
            .collect()
    }

    fn remove_member(&mut self, conn: ConnId) -> Vec<Outbound> {
        let Some(member) = self.members.remove(&conn) else {
            return Vec::new();
        };
        let leave = json!({ "t": "leave", "peer": member.id }).to_string();
        self.members
            .iter()
            .filter(|(_, m)| m.room == member.room)
            .map(|(c, _)| Outbound {
                conn: *c,
                text: leave.clone(),
            })
            .collect()
    }
}

/// Serve the rendezvous on an already-bound listener. Split out from `main` so
/// tests (and embedders) can run an in-process rendezvous on an ephemeral port.
///
/// Speaks plain WebSocket (`ws://`); terminate TLS (`wss://`) at a reverse proxy.
/// Clients connect to `…/rtc/<key>`; members are grouped by that room key.
pub async fn serve_listener(listener: tokio::net::TcpListener) {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    let hub = Arc::new(Mutex::new(Hub::new()));
    let senders: Senders = Arc::new(Mutex::new(HashMap::new()));
    let conn_seq = Arc::new(AtomicU64::new(1));

    loop {
        match listener.accept().await {
            Ok((stream, _peer)) => {
                let conn = conn_seq.fetch_add(1, Ordering::Relaxed);
                let hub = hub.clone();
                let senders = senders.clone();
                tokio::spawn(async move {
                    if let Err(e) = serve_conn(stream, conn, hub, senders).await {
                        tracing::debug!("conn {conn} ended: {e}");
                    }
                });
            }
            Err(e) => tracing::warn!("accept error: {e}"),
        }
    }
}

type Senders =
    std::sync::Arc<std::sync::Mutex<HashMap<ConnId, tokio::sync::mpsc::UnboundedSender<String>>>>;

/// Handle one WebSocket connection start-to-finish.
async fn serve_conn(
    stream: tokio::net::TcpStream,
    conn: ConnId,
    hub: std::sync::Arc<std::sync::Mutex<Hub>>,
    senders: Senders,
) -> Result<(), String> {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
    use tokio_tungstenite::tungstenite::Message;

    // The room key is the `…/rtc/<key>` path segment of the upgrade request.
    let room = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let ws = {
        let slot = room.clone();
        tokio_tungstenite::accept_hdr_async(stream, move |req: &Request, resp: Response| {
            let path = req.uri().path();
            let key = path
                .strip_prefix("/rtc/")
                .unwrap_or(path.trim_start_matches('/'))
                .to_string();
            *slot.lock().unwrap() = key;
            Ok(resp)
        })
        .await
        .map_err(|e| e.to_string())?
    };
    let room = room.lock().unwrap().clone();

    // Register an outbound channel and a writer task for this connection.
    let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    senders.lock().unwrap().insert(conn, out_tx);
    let (mut write, mut read) = ws.split();
    let writer = tokio::spawn(async move {
        while let Some(text) = out_rx.recv().await {
            if write.send(Message::Text(text)).await.is_err() {
                break;
            }
        }
    });

    dispatch(&hub, &senders, Inbound::Connect(conn, room));

    // Pump inbound frames into the hub.
    while let Some(msg) = read.next().await {
        match msg {
            Ok(Message::Text(t)) => dispatch(&hub, &senders, Inbound::Text(conn, t.to_string())),
            Ok(Message::Close(_)) => break,
            Ok(_) => {} // ignore binary/ping/pong for v0.1
            Err(_) => break,
        }
    }

    // Cleanup: broadcast the leave, drop the connection's sender, stop the writer.
    dispatch(&hub, &senders, Inbound::Disconnect(conn));
    senders.lock().unwrap().remove(&conn);
    writer.abort();
    Ok(())
}

/// Feed an event to the hub and route the resulting messages to their connections.
fn dispatch(hub: &std::sync::Mutex<Hub>, senders: &Senders, ev: Inbound) {
    let outs: Vec<Outbound> = hub.lock().unwrap().handle(ev);
    if outs.is_empty() {
        return;
    }
    let s = senders.lock().unwrap();
    for o in outs {
        if let Some(tx) = s.get(&o.conn) {
            let _ = tx.send(o.text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn parse(out: &Outbound) -> Value {
        serde_json::from_str(&out.text).unwrap()
    }

    fn announce(hub: &mut Hub, conn: ConnId, room: &str) -> Vec<Outbound> {
        hub.handle(Inbound::Connect(conn, room.into()));
        hub.handle(Inbound::Text(
            conn,
            r#"{"t":"announce","peer":99,"world":"x"}"#.into(),
        ))
    }

    #[test]
    fn announce_yields_welcome_before_peers_with_assigned_id() {
        let mut hub = Hub::new();
        let out = announce(&mut hub, 1, "plaza");
        assert_eq!(out.len(), 2, "welcome + peers to the newcomer");
        let welcome = parse(&out[0]);
        let peers = parse(&out[1]);
        // Ordering contract: welcome FIRST — the client adopts its id from it
        // before any negotiation starts.
        assert_eq!(welcome["t"], "welcome");
        assert_eq!(welcome["id"], 1, "server-assigned, not the provisional 99");
        assert_eq!(peers["t"], "peers");
        assert_eq!(
            peers["peers"].as_array().unwrap().len(),
            0,
            "room was empty"
        );
        // Self-certification: our own output must pass the public wire checker.
        use thread_conformance::clauses_pass;
        use thread_conformance::rendezvous::{check_peers, check_welcome};
        assert!(
            clauses_pass(&check_welcome(&welcome)),
            "welcome not conformant: {welcome}"
        );
        assert!(
            clauses_pass(&check_peers(&peers)),
            "peers not conformant: {peers}"
        );
    }

    #[test]
    fn newcomer_is_introduced_to_the_room_as_a_delta() {
        let mut hub = Hub::new();
        announce(&mut hub, 1, "plaza");
        let out = announce(&mut hub, 2, "plaza");
        // welcome{2} + peers{[1]} to the newcomer, peers{[2]} delta to member 1.
        assert_eq!(out.len(), 3);
        assert_eq!(parse(&out[0])["id"], 2);
        assert_eq!(parse(&out[1])["peers"], serde_json::json!([1]));
        assert_eq!(out[2].conn, 1, "existing member gets the delta");
        assert_eq!(parse(&out[2])["peers"], serde_json::json!([2]));
        // Glare rule sanity: exactly one side of the (1,2) pair offers — the lower
        // id (1), which is why the delta to existing members is REQUIRED.
    }

    #[test]
    fn signaling_is_relayed_verbatim_to_the_named_peer_only() {
        let mut hub = Hub::new();
        announce(&mut hub, 1, "plaza");
        announce(&mut hub, 2, "plaza");
        announce(&mut hub, 3, "plaza");
        let raw = r#"{"t":"offer","from":1,"to":2,"sdp":"v=0 fake"}"#;
        let out = hub.handle(Inbound::Text(1, raw.into()));
        assert_eq!(out.len(), 1, "only the addressee");
        assert_eq!(out[0].conn, 2);
        // Verbatim: the rendezvous relays blindly, it never rewrites the SDP.
        assert_eq!(parse(&out[0]), serde_json::from_str::<Value>(raw).unwrap());
    }

    #[test]
    fn rooms_are_isolated_from_each_other() {
        let mut hub = Hub::new();
        announce(&mut hub, 1, "room-a");
        let out = announce(&mut hub, 2, "room-b");
        assert_eq!(out.len(), 2, "no delta crosses rooms");
        assert_eq!(parse(&out[1])["peers"].as_array().unwrap().len(), 0);
        // Signaling addressed across rooms goes nowhere.
        let out = hub.handle(Inbound::Text(
            1,
            r#"{"t":"offer","from":1,"to":2,"sdp":"x"}"#.into(),
        ));
        assert!(out.is_empty(), "cross-room signaling leak");
    }

    #[test]
    fn disconnect_broadcasts_a_leave() {
        let mut hub = Hub::new();
        announce(&mut hub, 1, "plaza");
        announce(&mut hub, 2, "plaza");
        let out = hub.handle(Inbound::Disconnect(1));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].conn, 2);
        let leave = parse(&out[0]);
        assert_eq!(leave["t"], "leave");
        assert_eq!(leave["peer"], 1);
        assert_eq!(hub.member_count(), 1);
    }

    #[test]
    fn unannounced_or_malformed_traffic_is_ignored() {
        let mut hub = Hub::new();
        hub.handle(Inbound::Connect(1, "plaza".into()));
        // Signaling before announce: dropped (not a member yet).
        assert!(hub
            .handle(Inbound::Text(
                1,
                r#"{"t":"offer","from":9,"to":1,"sdp":"x"}"#.into()
            ))
            .is_empty());
        // Garbage frames: dropped, no panic.
        assert!(hub.handle(Inbound::Text(1, "not json".into())).is_empty());
        assert!(hub
            .handle(Inbound::Text(1, r#"{"nope":true}"#.into()))
            .is_empty());
        assert_eq!(hub.member_count(), 0);
    }
}
