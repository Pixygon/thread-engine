//! **Tier 1 presence — the serverless P2P mesh** (presence-topology-v0.1 §3).
//!
//! For small gatherings, presence needs *no dedicated server*: peers connect
//! directly and each sends its pose to the others. The only shared infrastructure
//! is a **stateless rendezvous** used once, at join, to introduce peers — it never
//! sees poses and holds no session state, so if every server on earth is gone, two
//! people who can reach each other still share presence. That's the survivability
//! floor of the whole design.
//!
//! This module is the transport-agnostic core: [`MeshCoordinator`] manages the peer
//! set, fans out the local pose, and feeds received peer poses into
//! [`PresenceState`] — exactly what a browser needs, with the actual byte transport
//! ([`MeshTransport`]) left to a binding. The real binding is WebRTC data channels;
//! tests use an in-memory loopback, so the mesh logic is verified with no network.
//! [`Signal`] is the rendezvous wire (SDP/ICE introduction), defined here so a
//! rendezvous service — or another peer — can implement it.

use crate::traveler::{PoseSample, PresenceState};
use serde::{Deserialize, Serialize};

/// A peer's occupant id within a mesh (assigned at the rendezvous).
pub type PeerId = u32;

/// Max peers in a mesh before a world should upgrade to a relay (topology §3): a
/// full mesh is O(n²) in connections, so past this it's cheaper to run a Tier-2
/// relay.
pub const MAX_MESH_PEERS: usize = 16;

/// A pose on the mesh wire — the same shape as a presence-wire pose, but
/// **sender-stamped**: with no server clock, the sender fills `ts` from its own
/// monotonic clock and receivers interpolate against *that sender's* timeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeshPose {
    pub p: [f32; 3],
    pub r: [f32; 4],
    pub v: [f32; 3],
    pub a: u8,
    pub ts: u32,
}

/// Rendezvous signaling — the messages peers exchange (via a stateless rendezvous)
/// to introduce themselves and negotiate a direct connection. The rendezvous relays
/// these blindly; it never handles a [`MeshPose`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "lowercase")]
pub enum Signal {
    /// "I'm here" — a peer joining a world announces itself. `peer` is the
    /// client's *provisional* self-picked id; a rendezvous that assigns ids
    /// replies with [`Signal::Welcome`] to override it.
    Announce { peer: PeerId, world: String },
    /// Rendezvous → newcomer: your **server-assigned** peer id (collision-free).
    /// Sent before [`Signal::Peers`]. A stateless rendezvous MAY omit it, in which
    /// case the client keeps its provisional `announce` id.
    Welcome { id: PeerId },
    /// Rendezvous → newcomer: the peers already present (to offer to).
    Peers { peers: Vec<PeerId> },
    /// WebRTC session description (connection offer).
    Offer {
        from: PeerId,
        to: PeerId,
        sdp: String,
    },
    /// WebRTC session description (connection answer).
    Answer {
        from: PeerId,
        to: PeerId,
        sdp: String,
    },
    /// An ICE candidate (a possible network path to a peer).
    Candidate {
        from: PeerId,
        to: PeerId,
        candidate: String,
    },
    /// A peer left the world.
    Leave { peer: PeerId },
}

/// A peer connection lifecycle event surfaced by the transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshEvent {
    Connected(PeerId),
    Disconnected(PeerId),
}

/// The transport seam: how the coordinator actually moves bytes between peers. The
/// real implementation is WebRTC data channels (a feature-gated binding); tests use
/// an in-memory loopback. Keeping this a trait is what makes the mesh logic testable
/// with no network and swappable across transports.
pub trait MeshTransport {
    /// Send `data` to every connected peer (unreliable/unordered is fine for poses).
    fn broadcast(&mut self, data: &[u8]);
    /// Drain inbound `(sender, data)` messages received since the last poll.
    fn poll(&mut self) -> Vec<(PeerId, Vec<u8>)>;
    /// Drain peer connect/disconnect events since the last poll.
    fn events(&mut self) -> Vec<MeshEvent>;
}

/// The socket-free P2P mesh coordinator: peer-set management + pose fan-out +
/// feeding received poses into [`PresenceState`]. Tier-1 presence, no server.
pub struct MeshCoordinator {
    me: PeerId,
    peers: std::collections::BTreeSet<PeerId>,
}

impl MeshCoordinator {
    pub fn new(me: PeerId) -> Self {
        Self {
            me,
            peers: std::collections::BTreeSet::new(),
        }
    }

    /// This peer's own id.
    pub fn id(&self) -> PeerId {
        self.me
    }

    /// Adopt a **server-assigned** id (a rendezvous `welcome{id}`), replacing the
    /// provisional self-picked one. The rendezvous sends `welcome` before `peers`,
    /// so this lands before any peer connection exists.
    pub fn set_id(&mut self, id: PeerId) {
        self.me = id;
    }

    /// Number of connected peers (excludes self).
    pub fn peer_count(&self) -> usize {
        self.peers.len()
    }

    /// Whether the mesh is at capacity — a world should upgrade to a relay past this.
    pub fn is_full(&self) -> bool {
        self.peers.len() >= MAX_MESH_PEERS
    }

    /// Broadcast the local player's pose to all peers. `ts` is the sender's clock.
    pub fn send_pose(
        &self,
        tx: &mut dyn MeshTransport,
        pos: [f32; 3],
        rot: [f32; 4],
        vel: [f32; 3],
        anim: u8,
        ts: u32,
    ) {
        if self.peers.is_empty() {
            return;
        }
        let pose = MeshPose {
            p: pos,
            r: rot,
            v: vel,
            a: anim,
            ts,
        };
        if let Ok(bytes) = serde_json::to_vec(&pose) {
            tx.broadcast(&bytes);
        }
    }

    /// Pump the transport once: apply connect/disconnect events and decode inbound
    /// peer poses into `presence`. Poses are stamped with the local `now_ms` (like
    /// the relay client) so interpolation uses one clock and cross-peer skew is a
    /// non-issue.
    pub fn pump(&mut self, tx: &mut dyn MeshTransport, presence: &mut PresenceState, now_ms: u64) {
        for ev in tx.events() {
            match ev {
                MeshEvent::Connected(p) if p != self.me && !self.is_full() => {
                    if self.peers.insert(p) {
                        presence.join(p, format!("peer {p}"), peer_color(p));
                    }
                }
                MeshEvent::Connected(_) => {} // self, or mesh full → ignore
                MeshEvent::Disconnected(p) => {
                    if self.peers.remove(&p) {
                        presence.leave(p);
                    }
                }
            }
        }

        for (from, data) in tx.poll() {
            if from == self.me || !self.peers.contains(&from) {
                continue; // ignore self-echo and unknown senders
            }
            if let Ok(pose) = serde_json::from_slice::<MeshPose>(&data) {
                presence.pose(
                    from,
                    PoseSample {
                        t_ms: now_ms,
                        pos: pose.p.into(),
                        rot: glam::Quat::from_xyzw(pose.r[0], pose.r[1], pose.r[2], pose.r[3]),
                        vel: pose.v.into(),
                        anim: pose.a,
                    },
                );
            }
        }
    }
}

/// Perfect-negotiation tiebreak: the **lower** peer id creates the offer. This
/// avoids "glare" — both sides offering at once — without any central coordinator,
/// so two peers introduced by the rendezvous deterministically agree who initiates.
pub fn should_offer(me: PeerId, other: PeerId) -> bool {
    me < other
}

/// A stable colour per peer id (until avatar descriptors carry real looks).
fn peer_color(id: PeerId) -> [f32; 4] {
    let h = id.wrapping_mul(2654435761);
    [
        0.4 + ((h & 0xFF) as f32 / 255.0) * 0.5,
        0.4 + (((h >> 8) & 0xFF) as f32 / 255.0) * 0.5,
        0.4 + (((h >> 16) & 0xFF) as f32 / 255.0) * 0.5,
        1.0,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// A two-party in-memory transport: what one side broadcasts lands in the
    /// other's inbox. Proves the mesh moves poses with no network.
    struct Loopback {
        outbox: Rc<RefCell<Vec<(PeerId, Vec<u8>)>>>, // the peer's inbox
        inbox: Rc<RefCell<Vec<(PeerId, Vec<u8>)>>>,
        me: PeerId,
        pending_events: Vec<MeshEvent>,
    }
    impl MeshTransport for Loopback {
        fn broadcast(&mut self, data: &[u8]) {
            self.outbox.borrow_mut().push((self.me, data.to_vec()));
        }
        fn poll(&mut self) -> Vec<(PeerId, Vec<u8>)> {
            std::mem::take(&mut *self.inbox.borrow_mut())
        }
        fn events(&mut self) -> Vec<MeshEvent> {
            std::mem::take(&mut self.pending_events)
        }
    }

    fn linked() -> (Loopback, Loopback) {
        let a_inbox = Rc::new(RefCell::new(Vec::new()));
        let b_inbox = Rc::new(RefCell::new(Vec::new()));
        let a = Loopback {
            outbox: b_inbox.clone(),
            inbox: a_inbox.clone(),
            me: 1,
            pending_events: vec![MeshEvent::Connected(2)],
        };
        let b = Loopback {
            outbox: a_inbox,
            inbox: b_inbox,
            me: 2,
            pending_events: vec![MeshEvent::Connected(1)],
        };
        (a, b)
    }

    #[test]
    fn two_peers_exchange_poses_with_no_server() {
        let (mut ta, mut tb) = linked();
        let mut a = MeshCoordinator::new(1);
        let mut b = MeshCoordinator::new(2);
        let mut pa = PresenceState::default();
        let mut pb = PresenceState::default();

        // First pump applies the Connected events → each sees the other.
        a.pump(&mut ta, &mut pa, 0);
        b.pump(&mut tb, &mut pb, 0);
        assert_eq!(a.peer_count(), 1);
        assert_eq!(b.peer_count(), 1);

        // A broadcasts a pose; B pumps and should now track A.
        a.send_pose(
            &mut ta,
            [5.0, 0.0, 3.0],
            [0.0, 0.0, 0.0, 1.0],
            [1.0, 0.0, 0.0],
            2,
            100,
        );
        b.pump(&mut tb, &mut pb, 50);
        assert_eq!(pb.count(), 1, "B tracks A after receiving a pose");
    }

    #[test]
    fn a_disconnect_drops_the_peer() {
        let (mut ta, _tb) = linked();
        let mut a = MeshCoordinator::new(1);
        let mut pa = PresenceState::default();
        a.pump(&mut ta, &mut pa, 0);
        assert_eq!(a.peer_count(), 1);

        ta.pending_events = vec![MeshEvent::Disconnected(2)];
        a.pump(&mut ta, &mut pa, 10);
        assert_eq!(a.peer_count(), 0, "peer removed on disconnect");
    }

    #[test]
    fn mesh_refuses_peers_past_capacity() {
        let mut c = MeshCoordinator::new(0);
        let mut p = PresenceState::default();
        let mut tx = Loopback {
            outbox: Rc::new(RefCell::new(Vec::new())),
            inbox: Rc::new(RefCell::new(Vec::new())),
            me: 0,
            pending_events: (1..=(MAX_MESH_PEERS as u32 + 5))
                .map(MeshEvent::Connected)
                .collect(),
        };
        c.pump(&mut tx, &mut p, 0);
        assert_eq!(c.peer_count(), MAX_MESH_PEERS, "capped at MAX_MESH_PEERS");
    }

    #[test]
    fn signaling_round_trips() {
        let s = Signal::Offer {
            from: 1,
            to: 2,
            sdp: "v=0…".into(),
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains(r#""t":"offer""#));
        assert_eq!(serde_json::from_str::<Signal>(&json).unwrap(), s);
    }

    #[test]
    fn welcome_assigns_the_peer_id() {
        // Wire shape: {"t":"welcome","id":N} — additive, sent before `peers`.
        let s: Signal = serde_json::from_str(r#"{"t":"welcome","id":7}"#).unwrap();
        assert_eq!(s, Signal::Welcome { id: 7 });

        // The coordinator adopts it, replacing the provisional id.
        let mut c = MeshCoordinator::new(999);
        c.set_id(7);
        assert_eq!(c.id(), 7);
    }

    #[test]
    fn exactly_one_side_offers() {
        // For any distinct pair, exactly one peer initiates — no glare.
        assert!(should_offer(1, 2));
        assert!(!should_offer(2, 1));
        assert_ne!(should_offer(3, 8), should_offer(8, 3));
    }
}
