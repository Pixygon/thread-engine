//! The relay **Hub** — the socket-free core of a Thread presence relay.
//!
//! A relay is deliberately simple and *stateless-in-spirit*: it holds only
//! ephemeral occupant positions, so a restart just means clients reconnect. All the
//! wire behaviour ([presence-wire-v0.1]) lives here as a pure state machine —
//! `handle(inbound) -> [outbound]` — so it's unit-testable with no network, and can
//! be certified against the very same wire checker (`thread_conformance::relay`)
//! that any browser author would run.
//!
//! [presence-wire-v0.1]: https://github.com/Pixygon/thread-spec/blob/main/specs/presence-wire-v0.1.md
//!
//! Decentralization by design: this is a *reference* relay anyone can `cargo run`
//! (or `docker run`) on a cheap host. A world names its own relay(s) in its
//! manifest; Pixygon runs at most one relay among many. The standard is the wire,
//! not any single server — so presence survives any one operator disappearing.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

/// A connection handle assigned by the transport layer (one per WebSocket).
pub type ConnId = u64;

/// An event coming *into* the hub.
pub enum Inbound {
    /// A socket connected to `world_id` (from the `…/thread/:worldId` path).
    Connect(ConnId, String),
    /// A raw wire message arrived on a connection.
    Text(ConnId, String),
    /// A binary frame arrived — a **voice packet** (presence-wire §6). Opaque
    /// audio payload; the hub only routes it.
    Binary(ConnId, Vec<u8>),
    /// A socket closed.
    Disconnect(ConnId),
}

/// A message the hub wants sent to a specific connection.
pub struct Outbound {
    pub conn: ConnId,
    pub body: Body,
}

/// A wire payload: JSON text (join/pose/leave) or a binary voice frame.
#[derive(Clone)]
pub enum Body {
    Text(String),
    Binary(Vec<u8>),
}

impl Outbound {
    /// Convenience: this outbound's text, or "" for binary (tests + JSON paths).
    pub fn text(&self) -> &str {
        match &self.body {
            Body::Text(t) => t,
            Body::Binary(_) => "",
        }
    }
}

/// Voice frames above this size are dropped (abuse guard) — 20 ms of 48 kHz
/// stereo PCM16 is ~7.7 KB; conformant 16 kHz mono frames are ~640 bytes.
pub const MAX_VOICE_FRAME: usize = 8 * 1024;

struct Occupant {
    id: u32,
    world: String,
    pos: [f32; 3],
    /// Identity claims read from the presented Passport (passport-v0.1 §2):
    /// the stable `sub`, display `name`, and descriptor `avatar` URL. Carried
    /// in `welcome.occupants[]` and the `join` broadcast so co-travelers can
    /// render "you" from your descriptor. `None`s for anonymous travelers.
    sub: Option<String>,
    name: Option<String>,
    avatar: Option<String>,
}

impl Occupant {
    /// The occupant as it appears on the wire (id + identity claims, if any).
    fn wire_json(&self) -> Value {
        let mut v = json!({ "id": self.id });
        if let Some(s) = &self.sub {
            v["sub"] = json!(s);
        }
        if let Some(n) = &self.name {
            v["name"] = json!(n);
        }
        if let Some(a) = &self.avatar {
            v["avatar"] = json!(a);
        }
        v
    }
}

/// The relay hub: worlds, occupants, and the wire state machine.
pub struct Hub {
    /// Area-of-interest radius in metres; poses are only fanned to occupants within
    /// it. `0.0` = unlimited (fan to the whole world). This is the primary scale
    /// lever: a 10k-occupant world only exchanges with the handful nearby.
    aoi_radius: f32,
    /// Suggested client send rate, advertised in `welcome`.
    tick_hz: u32,
    next_id: u32,
    /// Which world a connection joined (from its URL path), set on `Connect`.
    conn_world: HashMap<ConnId, String>,
    /// Active occupants, keyed by connection.
    occupants: HashMap<ConnId, Occupant>,
    /// **Observers**: connections watching a world's roster without entering
    /// it. A status bar, a dashboard, a directory's "who's home" light — all
    /// want the roster and nothing else, and none of them should appear as a
    /// body standing at the origin. Kept apart from `occupants` on purpose:
    /// they are never counted, never fanned poses or voice, and their
    /// leaving broadcasts nothing, because nothing of them was ever there.
    observers: HashMap<ConnId, String>,
}

impl Hub {
    pub fn new(aoi_radius: f32, tick_hz: u32) -> Self {
        Hub {
            aoi_radius,
            tick_hz,
            next_id: 1,
            conn_world: HashMap::new(),
            occupants: HashMap::new(),
            observers: HashMap::new(),
        }
    }

    /// Current number of live occupants across all worlds. Observers are not
    /// occupants and are not counted — that is the whole point of them.
    pub fn occupant_count(&self) -> usize {
        self.occupants.len()
    }

    /// Connections watching without entering.
    pub fn observer_count(&self) -> usize {
        self.observers.len()
    }

    /// Advance the state machine by one event, returning messages to send.
    pub fn handle(&mut self, ev: Inbound) -> Vec<Outbound> {
        match ev {
            Inbound::Connect(conn, world) => {
                self.conn_world.insert(conn, world);
                Vec::new()
            }
            Inbound::Text(conn, raw) => match serde_json::from_str::<Value>(&raw) {
                Ok(v) => self.on_message(conn, &v),
                Err(_) => Vec::new(), // ignore malformed frames (robustness)
            },
            Inbound::Binary(conn, data) => {
                // A voice frame (presence-wire §6): prefix the sender's id
                // (u32 LE) and fan it to the sender's world within the same
                // area-of-interest as poses — voices carry as far as presence.
                if data.is_empty() || data.len() > MAX_VOICE_FRAME {
                    return Vec::new();
                }
                let Some(occ) = self.occupants.get(&conn) else {
                    return Vec::new();
                };
                let mut framed = Vec::with_capacity(4 + data.len());
                framed.extend_from_slice(&occ.id.to_le_bytes());
                framed.extend_from_slice(&data);
                let (world, pos) = (occ.world.clone(), occ.pos);
                self.occupants
                    .iter()
                    .filter(|(c, o)| **c != conn && o.world == world && self.in_range(pos, o.pos))
                    .map(|(c, _)| Outbound {
                        conn: *c,
                        body: Body::Binary(framed.clone()),
                    })
                    .collect()
            }
            Inbound::Disconnect(conn) => {
                self.conn_world.remove(&conn);
                // An observer leaving is a non-event: no body, no farewell.
                self.observers.remove(&conn);
                self.remove_occupant(conn)
            }
        }
    }

    fn on_message(&mut self, conn: ConnId, v: &Value) -> Vec<Outbound> {
        match v.get("t").and_then(Value::as_str) {
            Some("join") => self.on_join(conn, v),
            Some("pose") => self.on_pose(conn, v),
            Some("interact") => self.on_interact(conn, v),
            Some("observe") => self.on_observe(conn),
            Some("leave") => self.remove_occupant(conn),
            _ => Vec::new(),
        }
    }

    /// `{"t":"observe"}` — watch this world's roster without joining it.
    ///
    /// The observer receives the same `welcome` a joiner gets (so it learns
    /// who is already present) and every subsequent `join`/`leave` for that
    /// world, and nothing else: no poses, no voice, no interactions. It is
    /// not an occupant, so it never appears in anyone's `welcome.occupants`,
    /// is never counted, and its own departure is silent.
    fn on_observe(&mut self, conn: ConnId) -> Vec<Outbound> {
        let Some(world) = self.conn_world.get(&conn).cloned() else {
            return Vec::new();
        };
        let occupants: Vec<Value> = self
            .occupants
            .values()
            .filter(|o| o.world == world)
            .map(Occupant::wire_json)
            .collect();
        self.observers.insert(conn, world);
        let welcome = json!({
            "t": "welcome", "id": 0, "observer": true,
            "occupants": occupants, "tick_hz": self.tick_hz,
        });
        vec![Outbound {
            conn,
            body: Body::Text(welcome.to_string()),
        }]
    }

    /// Everyone who should hear that a world's roster changed: its occupants
    /// (except the one it is about) plus its observers.
    fn roster_audience(&self, world: &str, except: ConnId) -> Vec<ConnId> {
        self.occupants
            .iter()
            .filter(|(c, o)| **c != except && o.world == world)
            .map(|(c, _)| *c)
            .chain(
                self.observers
                    .iter()
                    .filter(|(_, w)| *w == world)
                    .map(|(c, _)| *c),
            )
            .collect()
    }

    fn on_join(&mut self, conn: ConnId, v: &Value) -> Vec<Outbound> {
        let Some(world) = self.conn_world.get(&conn).cloned() else {
            return Vec::new();
        };
        // NOTE: a production relay verifies the Passport against the issuer's
        // jwks.json here. This reference relay runs "open" (dev/self-host default);
        // pair it with a verifying proxy or extend this for production. It still
        // *reads* the token's identity claims so co-travelers can render each
        // other from their descriptors — unverified claims, open-trust posture.
        let (sub, name, avatar) = passport_claims(v.get("passport").and_then(Value::as_str));
        let id = self.next_id;
        self.next_id += 1;

        // The joiner learns the occupants already present (so it can render them
        // at once).
        let others: Vec<Value> = self
            .occupants
            .values()
            .filter(|o| o.world == world)
            .map(Occupant::wire_json)
            .collect();

        let joiner = Occupant {
            id,
            world: world.clone(),
            pos: [0.0; 3],
            sub,
            name,
            avatar,
        };

        // Everyone already present learns the joiner *with identity* now (poses
        // alone carry only an id). Not AoI-gated — identity precedes proximity.
        let mut join = joiner.wire_json();
        join["t"] = json!("join");
        let join_text = join.to_string();
        let mut out: Vec<Outbound> = self
            .roster_audience(&world, conn)
            .into_iter()
            .map(|c| Outbound {
                conn: c,
                body: Body::Text(join_text.clone()),
            })
            .collect();

        self.occupants.insert(conn, joiner);

        let welcome =
            json!({ "t": "welcome", "id": id, "occupants": others, "tick_hz": self.tick_hz });
        out.push(Outbound {
            conn,
            body: Body::Text(welcome.to_string()),
        });
        out
    }

    fn on_pose(&mut self, conn: ConnId, v: &Value) -> Vec<Outbound> {
        // Update the sender's authoritative position first.
        let (id, world, pos) = {
            let Some(occ) = self.occupants.get_mut(&conn) else {
                return Vec::new();
            };
            if let Some(p) = vec3(v, "p") {
                occ.pos = p;
            }
            (occ.id, occ.world.clone(), occ.pos)
        };

        // Re-stamp with the authoritative occupant id + a server timestamp (the
        // single clock source clients interpolate against). Preserve p/r/y/v/a.
        let mut pose = v.clone();
        pose["id"] = json!(id);
        pose["ts"] = json!(now_ms());
        let text = pose.to_string();

        self.fan_out(&world, conn, pos, &text)
    }

    fn on_interact(&mut self, conn: ConnId, v: &Value) -> Vec<Outbound> {
        let Some((world, pos)) = self.occupants.get(&conn).map(|o| (o.world.clone(), o.pos)) else {
            return Vec::new();
        };
        self.fan_out(&world, conn, pos, &v.to_string())
    }

    /// Send `text` to every other occupant in `world` within area-of-interest of
    /// `from_pos`.
    fn fan_out(&self, world: &str, from: ConnId, from_pos: [f32; 3], text: &str) -> Vec<Outbound> {
        self.occupants
            .iter()
            .filter(|(conn, o)| {
                **conn != from && o.world == world && self.in_range(from_pos, o.pos)
            })
            .map(|(conn, _)| Outbound {
                conn: *conn,
                body: Body::Text(text.to_string()),
            })
            .collect()
    }

    fn remove_occupant(&mut self, conn: ConnId) -> Vec<Outbound> {
        let Some(occ) = self.occupants.remove(&conn) else {
            return Vec::new();
        };
        let leave = json!({ "t": "leave", "id": occ.id }).to_string();
        // Tell the rest of the world they left (AoI doesn't gate leaves — everyone
        // who might be rendering them needs to drop them), and tell the
        // watchers too, since a roster change is exactly what they are here for.
        self.roster_audience(&occ.world, conn)
            .into_iter()
            .map(|c| Outbound {
                conn: c,
                body: Body::Text(leave.clone()),
            })
            .collect()
    }

    fn in_range(&self, a: [f32; 3], b: [f32; 3]) -> bool {
        if self.aoi_radius <= 0.0 {
            return true;
        }
        let d2 = (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2);
        d2 <= self.aoi_radius * self.aoi_radius
    }
}

/// Serve the relay on an already-bound listener until the process ends. This is
/// the whole transport layer — the binary is just env parsing plus this call —
/// exposed so tests (and embedders) can run an in-process relay on an ephemeral
/// port, exactly like `thread_rendezvous::serve_listener`.
pub async fn serve_listener(listener: tokio::net::TcpListener, aoi_radius: f32, tick_hz: u32) {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    type Senders = Arc<Mutex<HashMap<ConnId, tokio::sync::mpsc::UnboundedSender<Body>>>>;

    /// Feed an event to the hub and route the resulting messages out.
    fn dispatch(hub: &Mutex<Hub>, senders: &Senders, ev: Inbound) {
        let outs: Vec<Outbound> = hub.lock().unwrap().handle(ev);
        if outs.is_empty() {
            return;
        }
        let s = senders.lock().unwrap();
        for o in outs {
            if let Some(tx) = s.get(&o.conn) {
                let _ = tx.send(o.body);
            }
        }
    }

    /// Handle one WebSocket connection start-to-finish.
    async fn serve(
        stream: tokio::net::TcpStream,
        conn: ConnId,
        hub: Arc<Mutex<Hub>>,
        senders: Senders,
    ) -> Result<(), String> {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
        use tokio_tungstenite::tungstenite::Message;

        // The world id is the `…/thread/:worldId` path segment of the upgrade.
        let world = Arc::new(Mutex::new(String::new()));
        let ws = {
            let slot = world.clone();
            tokio_tungstenite::accept_hdr_async(stream, move |req: &Request, resp: Response| {
                let path = req.uri().path();
                let w = path
                    .strip_prefix("/thread/")
                    .unwrap_or(path.trim_start_matches('/'))
                    .to_string();
                *slot.lock().unwrap() = w;
                Ok(resp)
            })
            .await
            .map_err(|e| e.to_string())?
        };
        let world = world.lock().unwrap().clone();

        // Register an outbound channel and a writer task for this connection.
        let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<Body>();
        senders.lock().unwrap().insert(conn, out_tx);
        let (mut write, mut read) = ws.split();
        let writer = tokio::spawn(async move {
            while let Some(body) = out_rx.recv().await {
                let msg = match body {
                    Body::Text(t) => Message::Text(t.into()),
                    Body::Binary(b) => Message::Binary(b.into()),
                };
                if write.send(msg).await.is_err() {
                    break;
                }
            }
        });

        dispatch(&hub, &senders, Inbound::Connect(conn, world));

        // Pump inbound frames into the hub.
        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(t)) => {
                    dispatch(&hub, &senders, Inbound::Text(conn, t.to_string()))
                }
                Ok(Message::Binary(b)) => {
                    dispatch(&hub, &senders, Inbound::Binary(conn, b.to_vec()))
                }
                Ok(Message::Close(_)) => break,
                Ok(_) => {} // ignore ping/pong
                Err(_) => break,
            }
        }

        // Cleanup: broadcast the leave, drop the sender, stop the writer.
        dispatch(&hub, &senders, Inbound::Disconnect(conn));
        senders.lock().unwrap().remove(&conn);
        writer.abort();
        Ok(())
    }

    let hub = Arc::new(Mutex::new(Hub::new(aoi_radius, tick_hz)));
    let senders: Senders = Arc::new(Mutex::new(HashMap::new()));
    let conn_seq = AtomicU64::new(1);

    loop {
        match listener.accept().await {
            Ok((stream, _peer)) => {
                let conn = conn_seq.fetch_add(1, Ordering::Relaxed);
                let hub = hub.clone();
                let senders = senders.clone();
                tokio::spawn(async move {
                    if let Err(e) = serve(stream, conn, hub, senders).await {
                        tracing::debug!("conn {conn} ended: {e}");
                    }
                });
            }
            Err(e) => tracing::warn!("accept error: {e}"),
        }
    }
}

/// Read `(sub, name, avatar)` from a Passport token's payload, best-effort.
/// Claims are *read*, not verified (open-trust reference posture — see the note
/// in `on_join`); opaque/absent tokens yield an anonymous occupant. Mirrors
/// `loom::passport` (a standalone relay can't depend on the engine).
fn passport_claims(token: Option<&str>) -> (Option<String>, Option<String>, Option<String>) {
    let Some(payload) = token.and_then(|t| t.split('.').nth(1)) else {
        return (None, None, None);
    };
    let Some(bytes) = base64url_decode(payload) else {
        return (None, None, None);
    };
    let Ok(claims) = serde_json::from_slice::<Value>(&bytes) else {
        return (None, None, None);
    };
    let s = |k: &str| claims.get(k).and_then(Value::as_str).map(str::to_string);
    (s("sub"), s("name"), s("avatar"))
}

/// Minimal base64url (RFC 4648 §5, padding optional) — enough to read a JWT
/// payload without a dependency.
fn base64url_decode(s: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some(u32::from(c - b'A')),
            b'a'..=b'z' => Some(u32::from(c - b'a') + 26),
            b'0'..=b'9' => Some(u32::from(c - b'0') + 52),
            b'-' => Some(62),
            b'_' => Some(63),
            _ => None,
        }
    }
    let s = s.trim_end_matches('=');
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut buf, mut bits) = (0u32, 0u32);
    for &c in s.as_bytes() {
        buf = (buf << 6) | val(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Some(out)
}

/// Relay-epoch milliseconds (u32 wraps ~49 days — acceptable per the spec).
fn now_ms() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u32)
        .unwrap_or(0)
}

/// Extract a `[f32; 3]` at `key`, if present and well-formed.
fn vec3(v: &Value, key: &str) -> Option<[f32; 3]> {
    let a = v.get(key)?.as_array()?;
    if a.len() != 3 {
        return None;
    }
    let f = |i: usize| a[i].as_f64().map(|x| x as f32);
    Some([f(0)?, f(1)?, f(2)?])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use thread_conformance::clauses_pass;
    use thread_conformance::relay::{check_pose, check_welcome};

    fn parse(out: &Outbound) -> Value {
        serde_json::from_str(out.text()).unwrap()
    }

    #[test]
    fn join_produces_a_conformant_welcome() {
        let mut hub = Hub::new(0.0, 15);
        hub.handle(Inbound::Connect(1, "lobby".into()));
        let out = hub.handle(Inbound::Text(1, r#"{"t":"join","passport":"tok"}"#.into()));
        assert_eq!(out.len(), 1);
        let welcome = parse(&out[0]);
        // The relay's own output must pass the public wire checker.
        assert!(
            clauses_pass(&check_welcome(&welcome)),
            "welcome not conformant: {welcome}"
        );
        assert_eq!(welcome["id"], 1);
        assert_eq!(welcome["occupants"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn pose_is_stamped_conformant_and_fanned_to_others() {
        let mut hub = Hub::new(0.0, 15);
        // Two occupants in the same world.
        hub.handle(Inbound::Connect(1, "plaza".into()));
        hub.handle(Inbound::Connect(2, "plaza".into()));
        hub.handle(Inbound::Text(1, r#"{"t":"join","passport":"a"}"#.into()));
        hub.handle(Inbound::Text(2, r#"{"t":"join","passport":"b"}"#.into()));

        // Occupant 1 sends a client pose (no ts, id 0) — the relay must fix both.
        let raw = r#"{"t":"pose","id":0,"p":[1,2,3],"r":[0,0,0,1],"v":[0.5,0,0],"a":1}"#;
        let out = hub.handle(Inbound::Text(1, raw.into()));

        // Fanned to occupant 2 only (not echoed to the sender).
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].conn, 2);
        let pose = parse(&out[0]);
        assert!(
            clauses_pass(&check_pose(&pose)),
            "pose not conformant: {pose}"
        );
        assert!(pose["ts"].as_u64().is_some(), "relay stamped ts");
        assert_eq!(pose["id"], 1, "relay set the authoritative occupant id");
    }

    /// Voice frames (presence-wire §6): binary in → sender-id-prefixed binary
    /// out, fanned only within area-of-interest, never echoed, size-capped —
    /// a voice carries exactly as far as presence does.
    #[test]
    fn voice_frames_fan_within_earshot_with_sender_prefix() {
        let mut hub = Hub::new(10.0, 15);
        for c in [1, 2, 3] {
            hub.handle(Inbound::Connect(c, "plaza".into()));
            hub.handle(Inbound::Text(c, r#"{"t":"join","passport":"x"}"#.into()));
        }
        // Occupant 2 stands near the speaker, occupant 3 out of earshot.
        hub.handle(Inbound::Text(
            2,
            r#"{"t":"pose","id":0,"p":[3,0,0],"v":[0,0,0],"r":[0,0,0,1],"a":0}"#.into(),
        ));
        hub.handle(Inbound::Text(
            3,
            r#"{"t":"pose","id":0,"p":[100,0,0],"v":[0,0,0],"r":[0,0,0,1],"a":0}"#.into(),
        ));

        let pcm = vec![7u8; 640]; // one 20 ms 16 kHz mono PCM16 frame
        let out = hub.handle(Inbound::Binary(1, pcm.clone()));
        assert_eq!(out.len(), 1, "only the near occupant hears");
        assert_eq!(out[0].conn, 2);
        let Body::Binary(b) = &out[0].body else {
            panic!("voice must stay binary")
        };
        assert_eq!(&b[..4], &1u32.to_le_bytes(), "sender id prefixed");
        assert_eq!(&b[4..], &pcm[..], "payload untouched");

        // Oversized frames are dropped, not relayed.
        let out = hub.handle(Inbound::Binary(1, vec![0u8; MAX_VOICE_FRAME + 1]));
        assert!(out.is_empty());
        // A connection that never joined has no voice.
        hub.handle(Inbound::Connect(9, "plaza".into()));
        assert!(hub.handle(Inbound::Binary(9, vec![1, 2, 3])).is_empty());
    }

    #[test]
    fn area_of_interest_culls_distant_occupants() {
        let mut hub = Hub::new(10.0, 15); // 10 m radius
        for c in [1, 2, 3] {
            hub.handle(Inbound::Connect(c, "field".into()));
            hub.handle(Inbound::Text(c, r#"{"t":"join","passport":"x"}"#.into()));
        }
        // Place occupant 2 near, occupant 3 far.
        hub.handle(Inbound::Text(
            2,
            r#"{"t":"pose","id":0,"p":[3,0,0],"v":[0,0,0],"r":[0,0,0,1],"a":0}"#.into(),
        ));
        hub.handle(Inbound::Text(
            3,
            r#"{"t":"pose","id":0,"p":[100,0,0],"v":[0,0,0],"r":[0,0,0,1],"a":0}"#.into(),
        ));

        // Occupant 1 poses at origin: only the near occupant (2) should hear it.
        let out = hub.handle(Inbound::Text(
            1,
            r#"{"t":"pose","id":0,"p":[0,0,0],"v":[0,0,0],"r":[0,0,0,1],"a":0}"#.into(),
        ));
        let targets: Vec<ConnId> = out.iter().map(|o| o.conn).collect();
        assert_eq!(
            targets,
            vec![2],
            "only the in-range occupant receives the pose"
        );
    }

    /// An observer watches the roster from outside: it learns who is here,
    /// hears arrivals and departures, and is itself invisible — not in
    /// anyone's welcome, not in the count, no phantom standing at the origin,
    /// and no farewell when it closes the tab.

    /// A join frame that *asserts* a name doesn't get one. An unattested name a
    /// relay repeats is a name anyone can claim, so identity is read from the
    /// Passport's claims and from nothing else — and the frame is accepted
    /// rather than rejected, because unknown fields are ignored. That makes the
    /// mistake a quiet one, which is why it is pinned here: a future change
    /// that "helpfully" honours the field would be a silent impersonation bug,
    /// not a feature. (An outside implementer hit this within twenty minutes of
    /// reading the spec, which is how it came to be written down.)
    #[test]
    fn a_name_asserted_in_the_join_frame_is_not_an_identity() {
        let mut hub = Hub::new(0.0, 15);
        hub.handle(Inbound::Connect(1, "commons".into()));
        hub.handle(Inbound::Text(
            1,
            r#"{"t":"join","passport":"","name":"ada","sub":"did:x:ada","avatar":"https://x/a.json"}"#
                .into(),
        ));
        assert_eq!(hub.occupant_count(), 1, "the frame is accepted, not refused");

        // What the world is told about them carries none of it.
        hub.handle(Inbound::Connect(2, "commons".into()));
        let out = hub.handle(Inbound::Text(2, r#"{"t":"join","passport":""}"#.into()));
        let welcome: Value = out
            .iter()
            .find(|o| o.conn == 2)
            .map(|o| serde_json::from_str(o.text()).unwrap())
            .expect("the joiner is welcomed");
        let them = &welcome["occupants"][0];
        assert!(them["id"].is_number(), "they are there: {them}");
        for claimed in ["name", "sub", "avatar"] {
            assert!(
                them[claimed].is_null(),
                "'{claimed}' was asserted on the wire and must not be honoured: {them}"
            );
        }
    }

    #[test]
    fn an_observer_watches_without_entering() {
        let mut hub = Hub::new(0.0, 15);
        hub.handle(Inbound::Connect(1, "commons".into()));
        hub.handle(Inbound::Text(1, r#"{"t":"join","passport":""}"#.into()));

        // The watcher arrives and is welcomed with the roster as it stands.
        hub.handle(Inbound::Connect(9, "commons".into()));
        let out = hub.handle(Inbound::Text(9, r#"{"t":"observe"}"#.into()));
        assert_eq!(out.len(), 1, "only the watcher is told anything");
        assert_eq!(out[0].conn, 9);
        let w: Value = serde_json::from_str(out[0].text()).unwrap();
        assert_eq!(w["t"], "welcome");
        assert_eq!(w["observer"], true);
        assert_eq!(
            w["occupants"].as_array().unwrap().len(),
            1,
            "sees who is here"
        );
        assert_eq!(hub.occupant_count(), 1, "watching is not occupying");
        assert_eq!(hub.observer_count(), 1);

        // A real arrival reaches both the occupant and the watcher…
        hub.handle(Inbound::Connect(2, "commons".into()));
        let out = hub.handle(Inbound::Text(2, r#"{"t":"join","passport":""}"#.into()));
        let told: Vec<ConnId> = out
            .iter()
            .filter(|o| o.text().contains("\"join\""))
            .map(|o| o.conn)
            .collect();
        assert!(
            told.contains(&1) && told.contains(&9),
            "join reached {told:?}"
        );
        // …but the joiner's welcome lists only the real occupant, never the watcher.
        let welcome: Value =
            serde_json::from_str(out.iter().find(|o| o.conn == 2).expect("welcome").text())
                .unwrap();
        assert_eq!(welcome["occupants"].as_array().unwrap().len(), 1);

        // Poses are never fanned to a watcher — it wants the roster, not the room.
        let out = hub.handle(Inbound::Text(
            1,
            r#"{"t":"pose","p":[1,0,1],"r":[0,0,0,1],"v":[0,0,0],"a":0}"#.into(),
        ));
        assert!(
            out.iter().all(|o| o.conn != 9),
            "no pose reached the watcher"
        );

        // A departure reaches the watcher…
        let out = hub.handle(Inbound::Disconnect(2));
        assert!(out
            .iter()
            .any(|o| o.conn == 9 && o.text().contains("leave")));

        // …and the watcher's own departure says nothing to anyone.
        let out = hub.handle(Inbound::Disconnect(9));
        assert!(
            out.is_empty(),
            "an observer leaving is a non-event ({} messages sent)",
            out.len()
        );
        assert_eq!(hub.observer_count(), 0);
        assert_eq!(hub.occupant_count(), 1);
    }

    #[test]
    fn disconnect_broadcasts_a_leave() {
        let mut hub = Hub::new(0.0, 15);
        hub.handle(Inbound::Connect(1, "hall".into()));
        hub.handle(Inbound::Connect(2, "hall".into()));
        hub.handle(Inbound::Text(1, r#"{"t":"join","passport":"a"}"#.into()));
        hub.handle(Inbound::Text(2, r#"{"t":"join","passport":"b"}"#.into()));

        let out = hub.handle(Inbound::Disconnect(1));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].conn, 2);
        let leave = parse(&out[0]);
        assert_eq!(leave["t"], "leave");
        assert_eq!(leave["id"], 1);
        assert_eq!(hub.occupant_count(), 1);
    }

    /// An unsigned JWT-shaped token around a JSON payload (claims are read, not
    /// verified, so a bare `alg:none` shape is enough to exercise the path).
    fn token_with_payload(payload: &str) -> String {
        fn b64url(data: &[u8]) -> String {
            const ALPHABET: &[u8] =
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
            let mut out = String::new();
            for chunk in data.chunks(3) {
                let mut buf = [0u8; 3];
                buf[..chunk.len()].copy_from_slice(chunk);
                let n = (u32::from(buf[0]) << 16) | (u32::from(buf[1]) << 8) | u32::from(buf[2]);
                for (i, shift) in [18u32, 12, 6, 0].iter().enumerate() {
                    if i <= chunk.len() {
                        out.push(ALPHABET[((n >> shift) & 63) as usize] as char);
                    }
                }
            }
            out
        }
        format!(
            "{}.{}.sig",
            b64url(br#"{"alg":"none"}"#),
            b64url(payload.as_bytes())
        )
    }

    #[test]
    fn identity_claims_cross_the_wire() {
        let mut hub = Hub::new(0.0, 15);
        let token = token_with_payload(
            r#"{"sub":"did:pixygon:abc","name":"Anders","avatar":"https://api.pixygon.io/v1/passport/avatar/did:pixygon:abc"}"#,
        );

        // First joiner carries a Passport; the room is empty, so just a welcome.
        hub.handle(Inbound::Connect(1, "plaza".into()));
        let out = hub.handle(Inbound::Text(
            1,
            format!(r#"{{"t":"join","passport":"{token}"}}"#),
        ));
        assert_eq!(out.len(), 1);

        // Second joiner (anonymous): occupant 1 hears a `join` broadcast for it,
        // and its welcome lists occupant 1 *with identity*.
        hub.handle(Inbound::Connect(2, "plaza".into()));
        let out = hub.handle(Inbound::Text(
            2,
            r#"{"t":"join","passport":"opaque"}"#.into(),
        ));
        assert_eq!(out.len(), 2);
        let (join, welcome) = (parse(&out[0]), parse(&out[1]));
        assert_eq!(out[0].conn, 1);
        assert_eq!(join["t"], "join");
        assert_eq!(join["id"], 2);
        assert!(join.get("sub").is_none(), "anonymous joiner has no claims");
        assert_eq!(out[1].conn, 2);
        let occupants = welcome["occupants"].as_array().unwrap();
        assert_eq!(occupants.len(), 1);
        assert_eq!(occupants[0]["id"], 1);
        assert_eq!(occupants[0]["sub"], "did:pixygon:abc");
        assert_eq!(occupants[0]["name"], "Anders");
        assert_eq!(
            occupants[0]["avatar"],
            "https://api.pixygon.io/v1/passport/avatar/did:pixygon:abc"
        );
    }

    #[test]
    fn worlds_are_isolated_from_each_other() {
        let mut hub = Hub::new(0.0, 15);
        hub.handle(Inbound::Connect(1, "world-a".into()));
        hub.handle(Inbound::Connect(2, "world-b".into()));
        hub.handle(Inbound::Text(1, r#"{"t":"join","passport":"a"}"#.into()));
        hub.handle(Inbound::Text(2, r#"{"t":"join","passport":"b"}"#.into()));

        // A pose in world-a must never reach world-b.
        let out = hub.handle(Inbound::Text(
            1,
            r#"{"t":"pose","id":0,"p":[0,0,0],"v":[0,0,0],"r":[0,0,0,1],"a":0}"#.into(),
        ));
        assert!(out.is_empty(), "cross-world leakage");
    }
}
