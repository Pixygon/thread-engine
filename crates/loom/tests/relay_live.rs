//! Live identity-over-the-wire proof: two real `PresenceClient`s joined through
//! an in-process `thread-relay`, one carrying a Passport — the other sees them
//! by *name*, not as "traveler N". This is the end-to-end certification of the
//! presence-wire v0.1.1 identity extension (Occupant claims + `join` broadcast)
//! plus the loom client's descriptor plumbing.
//!
//! Ignored by default: it opens sockets. Run manually:
//!
//! ```text
//! cargo test -p loom --test relay_live -- --ignored --nocapture
//! ```

use glam::{Quat, Vec3};
use thread_engine::traveler::PresenceState;
use thread_engine::presence::PresenceClient;

/// Start a reference relay on an ephemeral localhost port; returns the port.
fn start_relay() -> u16 {
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
            thread_relay::serve_listener(listener, 0.0, 15).await;
        });
    });
    rx.recv().expect("relay port")
}

/// An unsigned JWT-shaped Passport (the reference relay reads claims unverified).
fn passport(sub: &str, name: &str) -> String {
    fn b64url(data: &[u8]) -> String {
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
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
    let payload = format!(r#"{{"sub":"{sub}","name":"{name}"}}"#);
    format!(
        "{}.{}.sig",
        b64url(br#"{"alg":"none"}"#),
        b64url(payload.as_bytes())
    )
}

#[test]
#[ignore = "live WebSocket over localhost — run manually"]
fn identity_crosses_a_live_relay() {
    let port = start_relay();
    let relay = format!("ws://127.0.0.1:{port}");

    // A carries a Passport; B walks anonymous.
    let a = PresenceClient::connect(
        relay.clone(),
        "plaza".into(),
        passport("did:pixygon:abc", "Anders"),
        None,
    );
    let b = PresenceClient::connect(relay, "plaza".into(), String::new(), None);

    let mut pa = PresenceState::default();
    let mut pb = PresenceState::default();

    // Pump like a frame loop until B tracks A (or timeout). Poses make A
    // visible; the join broadcast/welcome carry the identity.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut now_ms: u64 = 0;
    let mut b_named = false;
    while std::time::Instant::now() < deadline {
        let mut hellos = a.drain(&mut pa, now_ms);
        hellos.extend(b.drain(&mut pb, now_ms));
        a.send_pose(Vec3::new(1.0, 0.0, 0.0), Quat::IDENTITY, Vec3::ZERO, 0);
        b.send_pose(Vec3::new(2.0, 0.0, 0.0), Quat::IDENTITY, Vec3::ZERO, 0);
        // B heard A's identity (via welcome occupants or the join broadcast).
        b_named = b_named
            || hellos.iter().any(|h| {
                h.sub.as_deref() == Some("did:pixygon:abc") && h.name.as_deref() == Some("Anders")
            });
        if b_named && pa.count() == 1 && pb.count() == 1 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
        now_ms += 50;
    }

    assert_eq!(pa.count(), 1, "A tracks B's pose");
    assert_eq!(pb.count(), 1, "B tracks A's pose");
    assert!(b_named, "B learned A's Passport identity over the wire");
}
