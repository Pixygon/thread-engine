//! WebRTC data-channel transport for the Tier-1 P2P mesh (`p2p` feature).
//!
//! The real binding behind [`crate::mesh::MeshTransport`]: it connects to a
//! stateless rendezvous (WebSocket) to exchange [`Signal`] messages, opens a direct
//! WebRTC data channel to each peer, and shuttles pose bytes over it — so a small
//! world is shared with no relay. Feature-gated because webrtc-rs is a heavy
//! dependency; the lean browser omits it entirely.
//!
//! Like the relay client, it bridges the async WebRTC stack to the synchronous
//! [`MeshTransport`] poll API via shared queues + a background task. The mesh logic
//! it feeds ([`crate::mesh::MeshCoordinator`]) is transport-agnostic and unit-tested
//! against an in-memory transport; this module is the plumbing that makes it live.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use tokio::sync::Mutex as AsyncMutex;
use tokio_tungstenite::tungstenite::Message;
use webrtc::api::media_engine::MediaEngine;
use webrtc::api::{APIBuilder, API};
use webrtc::data_channel::data_channel_message::DataChannelMessage;
use webrtc::data_channel::RTCDataChannel;
use webrtc::ice_transport::ice_candidate::{RTCIceCandidate, RTCIceCandidateInit};
use webrtc::ice_transport::ice_server::RTCIceServer;
use webrtc::peer_connection::configuration::RTCConfiguration;
use webrtc::peer_connection::sdp::session_description::RTCSessionDescription;
use webrtc::peer_connection::RTCPeerConnection;

use crate::mesh::{should_offer, MeshEvent, MeshTransport, PeerId, Signal};

type Inbox = Arc<Mutex<Vec<(PeerId, Vec<u8>)>>>;
type Events = Arc<Mutex<Vec<MeshEvent>>>;
type Peers = Arc<AsyncMutex<HashMap<PeerId, Peer>>>;

/// A [`MeshTransport`] over real WebRTC data channels.
pub struct WebRtcTransport {
    inbox: Inbox,
    events: Events,
    broadcast_tx: UnboundedSender<Vec<u8>>,
    assigned: Arc<AtomicU32>,
}

impl WebRtcTransport {
    /// Join the mesh for a world via `rendezvous_url` (`wss://…/rtc/<key>`) as
    /// occupant `me` (a *provisional* self-picked id — a rendezvous that assigns
    /// ids overrides it with `welcome{id}`; see [`WebRtcTransport::id`]).
    /// Non-blocking — the WebRTC stack runs on a background thread.
    pub fn connect(rendezvous_url: String, world: String, me: PeerId) -> Self {
        let inbox: Inbox = Arc::new(Mutex::new(Vec::new()));
        let events: Events = Arc::new(Mutex::new(Vec::new()));
        let (broadcast_tx, broadcast_rx) = unbounded_channel::<Vec<u8>>();
        let assigned = Arc::new(AtomicU32::new(me));
        let (inbox2, events2, assigned2) = (inbox.clone(), events.clone(), assigned.clone());
        std::thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    tracing::warn!("mesh: runtime init failed: {e}");
                    return;
                }
            };
            rt.block_on(run(
                rendezvous_url,
                world,
                me,
                inbox2,
                events2,
                assigned2,
                broadcast_rx,
            ));
        });
        Self {
            inbox,
            events,
            broadcast_tx,
            assigned,
        }
    }

    /// The effective peer id: the provisional one until the rendezvous sends
    /// `welcome{id}`, then the server-assigned one. Feed it to
    /// [`crate::mesh::MeshCoordinator::set_id`] when pumping.
    pub fn id(&self) -> PeerId {
        self.assigned.load(Ordering::Relaxed)
    }
}

impl MeshTransport for WebRtcTransport {
    fn broadcast(&mut self, data: &[u8]) {
        let _ = self.broadcast_tx.send(data.to_vec());
    }
    fn poll(&mut self) -> Vec<(PeerId, Vec<u8>)> {
        std::mem::take(&mut *self.inbox.lock().unwrap())
    }
    fn events(&mut self) -> Vec<MeshEvent> {
        std::mem::take(&mut *self.events.lock().unwrap())
    }
}

struct Peer {
    pc: Arc<RTCPeerConnection>,
    dc: Option<Arc<RTCDataChannel>>,
}

fn build_api() -> API {
    let mut m = MediaEngine::default();
    let _ = m.register_default_codecs();
    APIBuilder::new().with_media_engine(m).build()
}

fn default_config() -> RTCConfiguration {
    RTCConfiguration {
        ice_servers: vec![RTCIceServer {
            urls: vec!["stun:stun.l.google.com:19302".to_owned()],
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[allow(clippy::too_many_arguments)]
async fn run(
    url: String,
    world: String,
    mut me: PeerId,
    inbox: Inbox,
    events: Events,
    assigned: Arc<AtomicU32>,
    mut broadcast_rx: UnboundedReceiver<Vec<u8>>,
) {
    let ws = match tokio_tungstenite::connect_async(&url).await {
        Ok((ws, _)) => ws,
        Err(e) => {
            tracing::warn!("mesh: rendezvous {url} unreachable: {e} (staying solo)");
            return;
        }
    };
    let (mut ws_write, mut ws_read) = ws.split();
    let (sig_tx, mut sig_rx) = unbounded_channel::<Signal>();

    // Announce our arrival; the rendezvous replies with the peers already present.
    let _ = sig_tx.send(Signal::Announce { peer: me, world });

    let api = Arc::new(build_api());
    let peers: Peers = Arc::new(AsyncMutex::new(HashMap::new()));

    loop {
        tokio::select! {
            Some(sig) = sig_rx.recv() => {
                if let Ok(txt) = serde_json::to_string(&sig) {
                    if ws_write.send(Message::Text(txt)).await.is_err() { break; }
                }
            }
            incoming = ws_read.next() => match incoming {
                Some(Ok(Message::Text(txt))) => {
                    if let Ok(sig) = serde_json::from_str::<Signal>(&txt) {
                        // A server-assigned id replaces the provisional one. The
                        // rendezvous sends `welcome` before `peers`, so it lands
                        // before any peer connection is negotiated.
                        if let Signal::Welcome { id } = sig {
                            me = id;
                            assigned.store(id, Ordering::Relaxed);
                        } else {
                            handle_signal(sig, me, &api, &peers, &sig_tx, &inbox, &events).await;
                        }
                    }
                }
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                _ => {}
            },
            Some(data) = broadcast_rx.recv() => {
                let bytes = Bytes::from(data);
                let map = peers.lock().await;
                for peer in map.values() {
                    if let Some(dc) = &peer.dc {
                        let _ = dc.send(&bytes).await;
                    }
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn handle_signal(
    sig: Signal,
    me: PeerId,
    api: &Arc<API>,
    peers: &Peers,
    sig_tx: &UnboundedSender<Signal>,
    inbox: &Inbox,
    events: &Events,
) {
    match sig {
        // The rendezvous told us who's present — offer to those we should initiate to.
        Signal::Peers { peers: present } => {
            for other in present {
                if other == me || !should_offer(me, other) {
                    continue;
                }
                let pc = new_peer(api, me, other, sig_tx, inbox, events, peers).await;
                // Offerer creates the data channel.
                let dc = pc.create_data_channel("poses", None).await.ok();
                if let Some(dc) = &dc {
                    wire_dc(dc, other, inbox.clone(), events.clone());
                }
                peers
                    .lock()
                    .await
                    .insert(other, Peer { pc: pc.clone(), dc });
                if let Ok(offer) = pc.create_offer(None).await {
                    let sdp = offer.sdp.clone();
                    if pc.set_local_description(offer).await.is_ok() {
                        let _ = sig_tx.send(Signal::Offer {
                            from: me,
                            to: other,
                            sdp,
                        });
                    }
                }
            }
        }
        Signal::Offer { from, to, sdp } if to == me => {
            let pc = new_peer(api, me, from, sig_tx, inbox, events, peers).await;
            peers.lock().await.insert(
                from,
                Peer {
                    pc: pc.clone(),
                    dc: None,
                },
            );
            if let Ok(desc) = RTCSessionDescription::offer(sdp) {
                if pc.set_remote_description(desc).await.is_ok() {
                    if let Ok(answer) = pc.create_answer(None).await {
                        let sdp = answer.sdp.clone();
                        if pc.set_local_description(answer).await.is_ok() {
                            let _ = sig_tx.send(Signal::Answer {
                                from: me,
                                to: from,
                                sdp,
                            });
                        }
                    }
                }
            }
        }
        Signal::Answer { from, to, sdp } if to == me => {
            if let Some(peer) = peers.lock().await.get(&from) {
                if let Ok(desc) = RTCSessionDescription::answer(sdp) {
                    let _ = peer.pc.set_remote_description(desc).await;
                }
            }
        }
        Signal::Candidate {
            from,
            to,
            candidate,
        } if to == me => {
            if let Some(peer) = peers.lock().await.get(&from) {
                let _ = peer
                    .pc
                    .add_ice_candidate(RTCIceCandidateInit {
                        candidate,
                        ..Default::default()
                    })
                    .await;
            }
        }
        Signal::Leave { peer } => {
            if let Some(p) = peers.lock().await.remove(&peer) {
                let _ = p.pc.close().await;
                events.lock().unwrap().push(MeshEvent::Disconnected(peer));
            }
        }
        _ => {}
    }
}

/// Create a peer connection to `other`, wiring ICE-candidate forwarding and (for the
/// answering side) inbound data channels.
#[allow(clippy::too_many_arguments)]
async fn new_peer(
    api: &Arc<API>,
    me: PeerId,
    other: PeerId,
    sig_tx: &UnboundedSender<Signal>,
    inbox: &Inbox,
    events: &Events,
    peers: &Peers,
) -> Arc<RTCPeerConnection> {
    let pc = match api.new_peer_connection(default_config()).await {
        Ok(pc) => Arc::new(pc),
        Err(e) => {
            tracing::warn!("mesh: peer connection failed: {e}");
            // A closed, inert connection; the handshake for `other` simply won't complete.
            Arc::new(
                api.new_peer_connection(RTCConfiguration::default())
                    .await
                    .expect("pc"),
            )
        }
    };

    // Forward locally-gathered ICE candidates to the peer via the rendezvous.
    let sig = sig_tx.clone();
    pc.on_ice_candidate(Box::new(move |c: Option<RTCIceCandidate>| {
        let sig = sig.clone();
        Box::pin(async move {
            if let Some(c) = c {
                if let Ok(init) = c.to_json() {
                    let _ = sig.send(Signal::Candidate {
                        from: me,
                        to: other,
                        candidate: init.candidate,
                    });
                }
            }
        })
    }));

    // The answering side receives its data channel here.
    let inbox2 = inbox.clone();
    let events2 = events.clone();
    let peers2 = peers.clone();
    pc.on_data_channel(Box::new(move |dc: Arc<RTCDataChannel>| {
        let inbox = inbox2.clone();
        let events = events2.clone();
        let peers = peers2.clone();
        Box::pin(async move {
            wire_dc(&dc, other, inbox, events);
            if let Some(peer) = peers.lock().await.get_mut(&other) {
                peer.dc = Some(dc);
            }
        })
    }));

    pc
}

/// Wire a data channel's open/message/close to the shared queues the coordinator drains.
fn wire_dc(dc: &Arc<RTCDataChannel>, peer: PeerId, inbox: Inbox, events: Events) {
    let ev = events.clone();
    dc.on_open(Box::new(move || {
        ev.lock().unwrap().push(MeshEvent::Connected(peer));
        Box::pin(async {})
    }));
    dc.on_message(Box::new(move |msg: DataChannelMessage| {
        inbox.lock().unwrap().push((peer, msg.data.to_vec()));
        Box::pin(async {})
    }));
    dc.on_close(Box::new(move || {
        events.lock().unwrap().push(MeshEvent::Disconnected(peer));
        Box::pin(async {})
    }));
}
