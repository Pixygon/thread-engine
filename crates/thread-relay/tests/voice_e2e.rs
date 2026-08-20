//! End-to-end over real sockets: two clients on a live relay — one speaks, the
//! other hears the id-prefixed binary frame, with JSON presence riding beside.

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

type Ws =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Read frames until `pred` matches (or panic after a timeout).
async fn read_until<T>(ws: &mut Ws, mut pred: impl FnMut(Message) -> Option<T>) -> T {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let msg = ws.next().await.expect("socket open").expect("frame ok");
            if let Some(v) = pred(msg) {
                return v;
            }
        }
    })
    .await
    .expect("frame within timeout")
}

#[tokio::test]
async fn a_voice_frame_crosses_a_live_relay_with_the_speakers_id() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(thread_relay::serve_listener(listener, 0.0, 15));

    let url = format!("ws://{addr}/thread/plaza");
    let (mut alice, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    let (mut bob, _) = tokio_tungstenite::connect_async(&url).await.unwrap();

    // Both join; Alice's welcome carries her authoritative occupant id.
    alice
        .send(Message::Text(r#"{"t":"join","passport":"alice"}"#.into()))
        .await
        .unwrap();
    let alice_id = read_until(&mut alice, |m| {
        let t = m.into_text().ok()?;
        let v: serde_json::Value = serde_json::from_str(&t).ok()?;
        (v["t"] == "welcome").then(|| v["id"].as_u64().unwrap() as u32)
    })
    .await;
    bob.send(Message::Text(r#"{"t":"join","passport":"bob"}"#.into()))
        .await
        .unwrap();
    read_until(&mut bob, |m| {
        let t = m.into_text().ok()?;
        let v: serde_json::Value = serde_json::from_str(&t).ok()?;
        (v["t"] == "welcome").then_some(())
    })
    .await;

    // Alice speaks one 20 ms PCM16 frame; Bob hears it, prefixed with her id.
    let pcm: Vec<u8> = (0..640u32).map(|i| (i % 251) as u8).collect();
    alice
        .send(Message::Binary(pcm.clone().into()))
        .await
        .unwrap();
    let heard = read_until(&mut bob, |m| match m {
        Message::Binary(b) => Some(b.to_vec()),
        _ => None,
    })
    .await;
    assert_eq!(&heard[..4], &alice_id.to_le_bytes(), "sender id prefixed");
    assert_eq!(&heard[4..], &pcm[..], "payload byte-identical");
}
