//! Live Tier-1 P2P proof: two real `WebRtcTransport`s introduced by an in-process
//! `thread-rendezvous`, exchanging poses over actual WebRTC data channels on
//! localhost — no relay, no mocks. This is the end-to-end certification of the
//! serverless presence tier (presence-topology-v0.1 §3).
//!
//! Ignored by default: it opens sockets and drives the WebRTC stack (~seconds).
//! Run manually:
//!
//! ```text
//! cargo test -p loom --features p2p --test p2p_live -- --ignored --nocapture
//! ```
#![cfg(feature = "p2p")]

use thread_engine::traveler::PresenceState;
use thread_engine::mesh_webrtc::WebRtcTransport;
use thread_engine::MeshCoordinator;

/// Start a reference rendezvous on an ephemeral localhost port; returns the port.
fn start_rendezvous() -> u16 {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("rt");
        rt.block_on(async move {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind");
            tx.send(listener.local_addr().expect("addr").port())
                .expect("send port");
            thread_rendezvous::serve_listener(listener).await;
        });
    });
    rx.recv().expect("rendezvous port")
}

#[test]
#[ignore = "live WebRTC over localhost — run manually"]
fn two_peers_exchange_poses_over_a_live_rendezvous() {
    let port = start_rendezvous();
    let url = format!("ws://127.0.0.1:{port}/rtc/live-test");

    // Provisional ids 100/200 — the rendezvous assigns 1/2 via welcome{id}.
    let mut ta = WebRtcTransport::connect(url.clone(), "live-test".into(), 100);
    let mut tb = WebRtcTransport::connect(url, "live-test".into(), 200);
    let mut ca = MeshCoordinator::new(100);
    let mut cb = MeshCoordinator::new(200);
    let mut pa = PresenceState::default();
    let mut pb = PresenceState::default();

    // Pump like a browser frame loop until each side tracks the other (or timeout).
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let mut now_ms: u64 = 0;
    while std::time::Instant::now() < deadline {
        // Adopt the server-assigned ids the moment welcome lands.
        ca.set_id(ta.id());
        cb.set_id(tb.id());
        ca.pump(&mut ta, &mut pa, now_ms);
        cb.pump(&mut tb, &mut pb, now_ms);
        ca.send_pose(
            &mut ta,
            [1.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
            [0.0; 3],
            0,
            now_ms as u32,
        );
        cb.send_pose(
            &mut tb,
            [2.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
            [0.0; 3],
            0,
            now_ms as u32,
        );
        if pa.count() == 1 && pb.count() == 1 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
        now_ms += 50;
    }

    assert_ne!(ca.id(), 100, "A adopted a server-assigned id");
    assert_ne!(cb.id(), 200, "B adopted a server-assigned id");
    assert_eq!(ca.peer_count(), 1, "A's data channel to B opened");
    assert_eq!(cb.peer_count(), 1, "B's data channel to A opened");
    assert_eq!(pa.count(), 1, "A tracks B's pose (received over WebRTC)");
    assert_eq!(pb.count(), 1, "B tracks A's pose (received over WebRTC)");
}
