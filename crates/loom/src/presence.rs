//! Presence relay client — the live transport for shared worlds.
//!
//! Built against [presence-wire-v0.1](https://github.com/Pixygon/thread-spec/blob/main/specs/presence-wire-v0.1.md): connects to a
//! WebSocket relay (`wss://<relay>/thread/:worldId`), sends the local player's
//! pose, and forwards other travelers' poses into [`PresenceState`]. Runs its
//! socket on a background thread and talks to the render loop over channels, so
//! the frame never blocks and a missing/unreachable relay simply degrades to
//! solo (the world keeps running). Ready to connect the instant the relay
//! deploys; until then it logs and stays solo.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;

use glam::{Quat, Vec3};
use crate::traveler::{PoseSample, PresenceState};
use serde::{Deserialize, Serialize};

/// Messages the client sends to the relay (`t`-tagged, per the wire spec).
#[derive(Serialize)]
#[serde(tag = "t")]
enum ClientMsg {
    #[serde(rename = "join")]
    Join {
        passport: String,
        spawn: Option<String>,
    },
    #[serde(rename = "pose")]
    Pose {
        ts: u64,
        p: [f32; 3],
        r: [f32; 4],
        v: [f32; 3],
        a: u8,
    },
}

/// Messages the relay sends to the client.
#[derive(Deserialize)]
#[serde(tag = "t")]
enum ServerMsg {
    #[serde(rename = "welcome")]
    Welcome {
        #[serde(default)]
        occupants: Vec<Occupant>,
    },
    /// A traveler joined mid-session — identity precedes their first pose.
    #[serde(rename = "join")]
    Join {
        id: u32,
        #[serde(default)]
        sub: Option<String>,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        avatar: Option<String>,
    },
    #[serde(rename = "pose")]
    Pose {
        id: u32,
        p: [f32; 3],
        #[serde(default = "ident_quat")]
        r: [f32; 4],
        #[serde(default)]
        v: [f32; 3],
        #[serde(default)]
        a: u8,
    },
    #[serde(rename = "leave")]
    Leave { id: u32 },
    /// interact / voice / anything else — ignored for now.
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
struct Occupant {
    id: u32,
    /// Identity claims the relay read from the traveler's Passport (may be
    /// absent — anonymous travelers are always welcome).
    #[serde(default)]
    sub: Option<String>,
    #[serde(default)]
    name: Option<String>,
    /// Descriptor URL — where "them" can be fetched from (passport-v0.1 §3).
    #[serde(default)]
    avatar: Option<String>,
}

fn ident_quat() -> [f32; 4] {
    [0.0, 0.0, 0.0, 1.0]
}

/// A traveler's announced identity (from `welcome.occupants[]` or a `join`),
/// handed back by [`PresenceClient::drain`] so the browser can fetch their
/// descriptor and render "them" instead of a placeholder.
#[derive(Debug, Clone)]
pub struct TravelerHello {
    pub id: u32,
    pub sub: Option<String>,
    pub name: Option<String>,
    /// Their descriptor URL, if they carried a Passport with one.
    pub avatar: Option<String>,
}

/// Something to send up the socket: a JSON wire message or a voice frame.
enum Outgoing {
    Msg(ClientMsg),
    Voice(Vec<u8>),
}

/// A decoded event handed to the render loop.
enum Event {
    Join(TravelerHello),
    Pose {
        id: u32,
        pos: Vec3,
        rot: Quat,
        vel: Vec3,
        anim: u8,
    },
    Leave(u32),
}

/// Handle to the presence relay connection.
pub struct PresenceClient {
    events: Receiver<Event>,
    poses: tokio::sync::mpsc::Sender<Outgoing>,
    /// Received voice frames `(speaker id, PCM16 payload)` — presence-wire §6.
    voice: Receiver<(u32, Vec<u8>)>,
    connected: Arc<AtomicBool>,
}

impl PresenceClient {
    /// Connect to a single relay for `world_id`. Convenience over [`connect_any`].
    pub fn connect(
        relay: String,
        world_id: String,
        passport: String,
        spawn: Option<String>,
    ) -> Self {
        Self::connect_any(vec![relay], world_id, passport, spawn)
    }

    /// Connect to the first reachable relay in `relays` (primary first, then
    /// fallbacks — no single relay is a point of failure). Non-blocking: the socket
    /// runs on a background thread; if none is reachable, the world degrades to solo.
    pub fn connect_any(
        relays: Vec<String>,
        world_id: String,
        passport: String,
        spawn: Option<String>,
    ) -> Self {
        let urls: Vec<String> = relays
            .into_iter()
            .map(|relay| {
                if relay.contains("/thread/") {
                    relay
                } else {
                    format!("{}/thread/{}", relay.trim_end_matches('/'), world_id)
                }
            })
            .collect();
        let (evt_tx, evt_rx) = channel();
        let (voice_tx, voice_rx) = channel();
        let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel::<Outgoing>(256);
        let connected = Arc::new(AtomicBool::new(false));
        let conn2 = connected.clone();
        std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    tracing::warn!("presence: runtime init failed: {e}");
                    return;
                }
            };
            rt.block_on(run(urls, passport, spawn, evt_tx, voice_tx, cmd_rx, conn2));
        });
        Self {
            events: evt_rx,
            poses: cmd_tx,
            voice: voice_rx,
            connected,
        }
    }

    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    /// Apply all received traveler events to the presence set (stamping arrival
    /// time `now_ms` so interpolation shares the world clock). Returns the
    /// identities announced this drain, so the caller can fetch their
    /// descriptors and upgrade their look from the placeholder.
    pub fn drain(&self, presence: &mut PresenceState, now_ms: u64) -> Vec<TravelerHello> {
        let mut hellos = Vec::new();
        while let Ok(evt) = self.events.try_recv() {
            match evt {
                Event::Join(hello) => {
                    let name = hello
                        .name
                        .clone()
                        .or_else(|| hello.sub.clone())
                        .unwrap_or_else(|| format!("traveler {}", hello.id));
                    presence.join(hello.id, name, traveler_color(hello.id));
                    hellos.push(hello);
                }
                Event::Leave(id) => presence.leave(id),
                Event::Pose {
                    id,
                    pos,
                    rot,
                    vel,
                    anim,
                } => presence.pose(
                    id,
                    PoseSample {
                        t_ms: now_ms,
                        pos,
                        rot,
                        vel,
                        anim,
                    },
                ),
            }
        }
        hellos
    }

    /// Send the local player's pose (dropped if the outbound queue is full).
    pub fn send_pose(&self, pos: Vec3, rot: Quat, vel: Vec3, anim: u8) {
        let _ = self.poses.try_send(Outgoing::Msg(ClientMsg::Pose {
            ts: 0,
            p: pos.to_array(),
            r: [rot.x, rot.y, rot.z, rot.w],
            v: vel.to_array(),
            a: anim,
        }));
    }

    /// Send one voice frame (raw PCM16 payload, presence-wire §6). Dropped if
    /// the queue is full — voice is realtime; late frames are worthless.
    pub fn send_voice(&self, frame: Vec<u8>) {
        let _ = self.poses.try_send(Outgoing::Voice(frame));
    }

    /// Voice frames received since the last call, as `(speaker id, PCM16)`.
    pub fn drain_voice(&self) -> Vec<(u32, Vec<u8>)> {
        let mut out = Vec::new();
        while let Ok(f) = self.voice.try_recv() {
            out.push(f);
        }
        out
    }
}

/// A stable colour per traveler id (until avatar descriptors carry real looks).
fn traveler_color(id: u32) -> [f32; 4] {
    let h = id.wrapping_mul(2654435761);
    [
        0.4 + ((h & 0xFF) as f32 / 255.0) * 0.5,
        0.4 + (((h >> 8) & 0xFF) as f32 / 255.0) * 0.5,
        0.4 + (((h >> 16) & 0xFF) as f32 / 255.0) * 0.5,
        1.0,
    ]
}

/// The socket loop: connect (trying each relay in order), send `join`, then relay
/// poses both ways.
async fn run(
    urls: Vec<String>,
    passport: String,
    spawn: Option<String>,
    events: std::sync::mpsc::Sender<Event>,
    voice: std::sync::mpsc::Sender<(u32, Vec<u8>)>,
    mut poses: tokio::sync::mpsc::Receiver<Outgoing>,
    connected: Arc<AtomicBool>,
) {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    // Try relays in order; the first that connects wins (fallback on failure).
    let mut connection = None;
    for url in &urls {
        match tokio_tungstenite::connect_async(url).await {
            Ok((ws, _)) => {
                connection = Some((ws, url.clone()));
                break;
            }
            Err(e) => tracing::warn!("presence: relay {url} unreachable: {e}"),
        }
    }
    let (ws, url) = match connection {
        Some(c) => c,
        None => {
            tracing::warn!(
                "presence: no relay reachable ({} tried) — staying solo",
                urls.len()
            );
            return;
        }
    };
    tracing::info!("presence: connected to {url}");
    connected.store(true, Ordering::Relaxed);
    let (mut write, mut read) = ws.split();

    // Announce ourselves.
    if let Ok(join) = serde_json::to_string(&ClientMsg::Join { passport, spawn }) {
        let _ = write.send(Message::Text(join)).await;
    }

    // Keepalive independent of the render loop: an occluded window stops
    // pumping poses (compositor redraw throttling), and a silent socket gets
    // idle-closed by reverse proxies. Ping from the async task instead.
    let mut keepalive = tokio::time::interval(std::time::Duration::from_secs(10));
    keepalive.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = keepalive.tick() => {
                if write.send(Message::Ping(Vec::new())).await.is_err() {
                    break;
                }
            }
            incoming = read.next() => match incoming {
                Some(Ok(Message::Binary(b))) => {
                    // A voice frame: 4-byte LE speaker id, then the payload.
                    if b.len() > 4 {
                        let id = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                        let _ = voice.send((id, b[4..].to_vec()));
                    }
                }
                Some(Ok(msg)) if msg.is_text() => {
                    if let Ok(txt) = msg.to_text() {
                        forward(txt, &events);
                    }
                }
                Some(Ok(msg)) if msg.is_close() => break,
                Some(Err(_)) | None => break,
                _ => {}
            },
            outgoing = poses.recv() => match outgoing {
                Some(Outgoing::Msg(msg)) => {
                    if let Ok(txt) = serde_json::to_string(&msg) {
                        if write.send(Message::Text(txt)).await.is_err() {
                            break;
                        }
                    }
                }
                Some(Outgoing::Voice(frame)) => {
                    if write.send(Message::Binary(frame)).await.is_err() {
                        break;
                    }
                }
                None => break,
            }
        }
    }
    connected.store(false, Ordering::Relaxed);
    tracing::info!("presence: disconnected from {url}");
}

/// Decode one server message and forward it as an [`Event`].
fn forward(txt: &str, events: &std::sync::mpsc::Sender<Event>) {
    let Ok(msg) = serde_json::from_str::<ServerMsg>(txt) else {
        return;
    };
    match msg {
        ServerMsg::Welcome { occupants } => {
            tracing::info!(
                "presence: welcomed — {} traveler(s) already here",
                occupants.len()
            );
            for o in occupants {
                let _ = events.send(Event::Join(TravelerHello {
                    id: o.id,
                    sub: o.sub,
                    name: o.name,
                    avatar: o.avatar,
                }));
            }
        }
        ServerMsg::Join {
            id,
            sub,
            name,
            avatar,
        } => {
            tracing::info!(
                "presence: traveler joined — id {id}{}",
                name.as_deref()
                    .map(|n| format!(" ({n})"))
                    .unwrap_or_default()
            );
            let _ = events.send(Event::Join(TravelerHello {
                id,
                sub,
                name,
                avatar,
            }));
        }
        ServerMsg::Pose { id, p, r, v, a } => {
            let _ = events.send(Event::Pose {
                id,
                pos: Vec3::from_array(p),
                rot: Quat::from_xyzw(r[0], r[1], r[2], r[3]),
                vel: Vec3::from_array(v),
                anim: a,
            });
        }
        ServerMsg::Leave { id } => {
            let _ = events.send(Event::Leave(id));
        }
        ServerMsg::Other => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_pose_per_wire_spec() {
        let json = serde_json::to_string(&ClientMsg::Pose {
            ts: 12,
            p: [1.0, 2.0, 3.0],
            r: [0.0, 0.0, 0.0, 1.0],
            v: [0.5, 0.0, 0.0],
            a: 1,
        })
        .unwrap();
        assert!(json.contains(r#""t":"pose""#));
        assert!(json.contains(r#""p":[1.0,2.0,3.0]"#));
    }

    #[test]
    fn decodes_server_messages_and_forwards_events() {
        let (tx, rx) = channel();
        forward(r#"{"t":"welcome","occupants":[{"id":7},{"id":9}]}"#, &tx);
        forward(
            r#"{"t":"pose","id":7,"p":[3.0,0.0,-1.0],"r":[0,0,0,1],"v":[1,0,0],"a":2}"#,
            &tx,
        );
        forward(r#"{"t":"leave","id":9}"#, &tx);
        forward(r#"{"t":"voice","id":7}"#, &tx); // ignored

        let mut presence = PresenceState::new();
        let mut joins = 0;
        let mut poses = 0;
        let mut leaves = 0;
        while let Ok(evt) = rx.try_recv() {
            match evt {
                Event::Join(_) => joins += 1,
                Event::Pose { .. } => poses += 1,
                Event::Leave(_) => leaves += 1,
            }
        }
        assert_eq!((joins, poses, leaves), (2, 1, 1));
        let _ = &mut presence; // (drain covered by the presence module's tests)
    }

    #[test]
    fn identity_rides_welcome_occupants_and_join_broadcasts() {
        let (tx, rx) = channel();
        // A welcome occupant with claims, and a mid-session join broadcast.
        forward(
            r#"{"t":"welcome","occupants":[{"id":7,"sub":"did:pixygon:abc","name":"Anders","avatar":"https://api.pixygon.io/v1/passport/avatar/did:pixygon:abc"}]}"#,
            &tx,
        );
        forward(r#"{"t":"join","id":11,"name":"Kaya"}"#, &tx);

        let Ok(Event::Join(a)) = rx.try_recv() else {
            panic!("expected a join")
        };
        assert_eq!(a.id, 7);
        assert_eq!(a.sub.as_deref(), Some("did:pixygon:abc"));
        assert_eq!(a.name.as_deref(), Some("Anders"));
        assert!(a.avatar.as_deref().unwrap().ends_with("did:pixygon:abc"));

        let Ok(Event::Join(b)) = rx.try_recv() else {
            panic!("expected a join")
        };
        assert_eq!(
            (b.id, b.name.as_deref(), b.sub, b.avatar),
            (11, Some("Kaya"), None, None)
        );
    }
}
