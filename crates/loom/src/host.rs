//! The embedded room host — a browser hosting a place itself.
//!
//! The presence topology's P2P tier says *participants* host each other; the
//! first (and, for a home, only) willing host is a running browser. This module
//! is that capability: one listener that serves BOTH halves of a place —
//!
//! - **`GET`** (plain HTTP) → the world's manifest, CORS-open, at any path —
//!   so `thread://<ip>:<port>` resolves via the standard `.well-known` fetch
//!   (ported hosts resolve over plain HTTP: the invite-address convention).
//! - **WebSocket upgrade** → a presence room, driven by the reference relay's
//!   [`thread_relay::Hub`] — the exact wire every relay speaks.
//!
//! The host's lifetime IS the rule: an owner-tied room (a traveler's home)
//! lives while the owner's browser runs it. [`HomeHost::stop`] (or drop) tears
//! down the runtime, every guest socket closes, and guests' browsers walk them
//! home (`presence.owner_required` in the served manifest tells them to).

use std::net::UdpSocket;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use thread_relay::{Body, ConnId, Hub, Inbound, Outbound};

/// Ports tried in order for the invite listener — a small well-known range so
/// invite addresses stay guessable-short, but two browsers on one machine
/// (or a stuck socket) don't collide.
const PORT_RANGE: std::ops::Range<u16> = 4200..4210;

/// A running embedded host: manifest server + presence room in one listener.
pub struct HomeHost {
    rt: Option<tokio::runtime::Runtime>,
    port: u16,
    occupants: Arc<AtomicUsize>,
}

impl HomeHost {
    /// Serve a place and its presence room. Binds the first free port in the
    /// invite range on all interfaces, then asks `manifest_for` to build the
    /// served manifest for that port (it needs the port to name the room URL
    /// guests should join). Returns `None` if no port binds.
    pub fn start(manifest_for: impl FnOnce(u16) -> String) -> Option<HomeHost> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .ok()?;
        let listener = rt.block_on(async {
            for port in PORT_RANGE {
                if let Ok(l) = tokio::net::TcpListener::bind(("0.0.0.0", port)).await {
                    return Some(l);
                }
            }
            None
        })?;
        let port = listener.local_addr().ok()?.port();
        let manifest_text = manifest_for(port);
        let occupants = Arc::new(AtomicUsize::new(0));
        rt.spawn(accept_loop(listener, manifest_text, occupants.clone()));
        tracing::info!("host: serving this place on port {port}");
        Some(HomeHost {
            rt: Some(rt),
            port,
            occupants,
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Guests currently in the room (not counting the owner's own connection
    /// is the caller's business — the hub counts every occupant).
    pub fn occupant_count(&self) -> usize {
        self.occupants.load(Ordering::Relaxed)
    }

    /// The invite address to hand a friend: `thread://<lan-ip>:<port>`.
    pub fn invite_locator(&self) -> String {
        format!(
            "thread://{}:{}",
            lan_ip().unwrap_or_else(|| "127.0.0.1".into()),
            self.port
        )
    }

    /// The room URL the owner's own browser joins (loopback, full path).
    pub fn local_room_url(&self) -> String {
        format!("ws://127.0.0.1:{}/thread/home", self.port)
    }

    /// Close the door: every guest socket drops with the runtime.
    pub fn stop(&mut self) {
        if let Some(rt) = self.rt.take() {
            rt.shutdown_background();
            tracing::info!("host: closed (port {})", self.port);
        }
    }
}

impl Drop for HomeHost {
    fn drop(&mut self) {
        self.stop();
    }
}

/// This machine's LAN address, discovered by routing (no packet is sent).
pub fn lan_ip() -> Option<String> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:9").ok()?; // TEST-NET; only picks the egress interface
    Some(s.local_addr().ok()?.ip().to_string())
}

async fn accept_loop(
    listener: tokio::net::TcpListener,
    manifest_text: String,
    occupants: Arc<AtomicUsize>,
) {
    type Senders =
        Arc<Mutex<std::collections::HashMap<ConnId, tokio::sync::mpsc::UnboundedSender<Body>>>>;
    let hub = Arc::new(Mutex::new(Hub::new(64.0, 15)));
    let senders: Senders = Arc::new(Mutex::new(Default::default()));
    let next_conn = Arc::new(AtomicU64::new(1));
    let manifest = Arc::new(manifest_text);

    fn dispatch(hub: &Mutex<Hub>, senders: &Senders, ev: Inbound) {
        let outs: Vec<Outbound> = hub.lock().unwrap().handle(ev);
        let s = senders.lock().unwrap();
        for o in outs {
            if let Some(tx) = s.get(&o.conn) {
                let _ = tx.send(o.body);
            }
        }
    }

    loop {
        let Ok((stream, _)) = listener.accept().await else {
            break;
        };
        let hub = hub.clone();
        let senders = senders.clone();
        let manifest = manifest.clone();
        let conn = next_conn.fetch_add(1, Ordering::Relaxed);
        let occupants = occupants.clone();
        tokio::spawn(async move {
            // One peek tells the two apart: an upgrade carries its header.
            let mut probe = [0u8; 1024];
            let Ok(n) = stream.peek(&mut probe).await else {
                return;
            };
            let head = String::from_utf8_lossy(&probe[..n]).to_ascii_lowercase();
            if !head.contains("upgrade: websocket") {
                serve_manifest(stream, &manifest).await;
                return;
            }

            use futures_util::{SinkExt, StreamExt};
            use tokio_tungstenite::tungstenite::Message;
            let Ok(ws) = tokio_tungstenite::accept_async(stream).await else {
                return;
            };
            let (mut write, mut read) = ws.split();
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Body>();
            senders.lock().unwrap().insert(conn, tx);
            // Every room on this host is the one room — the place being served.
            dispatch(&hub, &senders, Inbound::Connect(conn, "home".to_string()));
            occupants.fetch_add(1, Ordering::Relaxed);

            loop {
                tokio::select! {
                    out = rx.recv() => match out {
                        Some(Body::Text(t)) => {
                            if write.send(Message::Text(t.into())).await.is_err() { break; }
                        }
                        Some(Body::Binary(b)) => {
                            if write.send(Message::Binary(b.into())).await.is_err() { break; }
                        }
                        None => break,
                    },
                    msg = read.next() => match msg {
                        Some(Ok(Message::Text(t))) => {
                            dispatch(&hub, &senders, Inbound::Text(conn, t.to_string()))
                        }
                        Some(Ok(Message::Binary(b))) => {
                            dispatch(&hub, &senders, Inbound::Binary(conn, b.to_vec()))
                        }
                        Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                        Some(Ok(_)) => {} // ping/pong — tungstenite answers
                    },
                }
            }
            occupants.fetch_sub(1, Ordering::Relaxed);
            senders.lock().unwrap().remove(&conn);
            dispatch(&hub, &senders, Inbound::Disconnect(conn));
        });
    }
}

/// Answer a plain-HTTP request with the manifest (any GET path — the resolver
/// asks for `/.well-known/thread/world.json`, curl asks for `/`; both get the
/// world). CORS-open like every Thread manifest host.
async fn serve_manifest(mut stream: tokio::net::TcpStream, manifest: &str) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    // Drain the request head so the peer sees a clean exchange.
    let mut buf = [0u8; 2048];
    let _ = stream.read(&mut buf).await;
    let body = manifest.as_bytes();
    let head = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\naccess-control-allow-origin: *\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes()).await;
    let _ = stream.write_all(body).await;
    let _ = stream.shutdown().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// End-to-end over loopback: manifest over plain HTTP, then two "browsers"
    /// join the room, see each other's hello, and get dropped when the host
    /// stops — the whole owner-tied contract in one test.
    #[test]
    fn serves_manifest_and_room_then_kicks_on_stop() {
        let manifest = r#"{ "thread": "thread/0.1", "world": { "id": "home", "title": "T" } }"#;
        let mut host = HomeHost::start(|_port| manifest.to_string()).expect("a free invite port");
        let port = host.port();

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            // 1. Plain HTTP GET → the manifest.
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .unwrap();
            s.write_all(b"GET /.well-known/thread/world.json HTTP/1.1\r\nhost: x\r\n\r\n")
                .await
                .unwrap();
            let mut resp = String::new();
            s.read_to_string(&mut resp).await.unwrap();
            assert!(resp.starts_with("HTTP/1.1 200"), "manifest served: {resp}");
            assert!(resp.contains(r#""id": "home""#));

            // 2. Two guests join the room and are introduced.
            use futures_util::{SinkExt, StreamExt};
            use tokio_tungstenite::tungstenite::Message;
            let url = format!("ws://127.0.0.1:{port}/thread/home");
            let (mut a, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
            a.send(Message::Text(r#"{"t":"join"}"#.into()))
                .await
                .unwrap();
            let welcome = a.next().await.unwrap().unwrap();
            assert!(welcome.to_text().unwrap().contains("welcome"));

            let (mut b, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
            b.send(Message::Text(r#"{"t":"join"}"#.into()))
                .await
                .unwrap();
            let _welcome_b = b.next().await.unwrap().unwrap();
            let join_seen_by_a = a.next().await.unwrap().unwrap();
            assert!(
                join_seen_by_a.to_text().unwrap().contains("join"),
                "a sees b arrive"
            );

            // 3. The owner leaves — the host stops — both sockets die.
            host.stop();
            let end = tokio::time::timeout(std::time::Duration::from_secs(3), async {
                loop {
                    match a.next().await {
                        Some(Ok(m)) if m.is_close() => break,
                        None | Some(Err(_)) => break,
                        _ => {}
                    }
                }
            })
            .await;
            assert!(end.is_ok(), "guest socket closed when the host stopped");
        });
    }
}
