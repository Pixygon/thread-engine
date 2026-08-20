//! Live commerce proof: a real `CommerceClient` purchase against an in-process
//! HTTP endpoint speaking PixygonServer's exact `POST /v1/thread/purchase`
//! contract — bearer auth in, `{ worldId, itemStructuredId, priceRef }` body,
//! `{ grant, charge }` back. Certifies the browser side of the commerce wire.
//!
//! Ignored by default: it opens sockets. Run manually:
//!
//! ```text
//! cargo test -p loom --test commerce_live -- --ignored --nocapture
//! ```

use thread_engine::commerce::{offer_from, CommerceClient};

/// The response shape, verbatim from PixygonServer controllers/thread.js#purchase.
const RECEIPT: &str = r#"{
  "grant": { "itemStructuredId": "21010001", "worldId": "market", "userId": "u1",
             "granted": true, "persisted": false, "ref": "thread:u1:market:21010001" },
  "charge": { "amount": 25, "currency": "NOK", "recorded": true, "duplicate": false }
}"#;

/// A one-request HTTP endpoint on an ephemeral port. Reads the full request,
/// hands it back for assertions through the channel, answers with `RECEIPT`.
fn start_endpoint() -> (u16, std::sync::mpsc::Receiver<String>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        // Read headers, then exactly Content-Length body bytes.
        let mut buf = Vec::new();
        let mut chunk = [0u8; 1024];
        let (head_end, content_len) = loop {
            let n = stream.read(&mut chunk).expect("read");
            buf.extend_from_slice(&chunk[..n]);
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&buf[..pos]).to_string();
                let len = head
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(str::trim)
                            .map(str::to_string)
                    })
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(0);
                break (pos + 4, len);
            }
        };
        while buf.len() < head_end + content_len {
            let n = stream.read(&mut chunk).expect("read body");
            buf.extend_from_slice(&chunk[..n]);
        }
        tx.send(String::from_utf8_lossy(&buf).to_string())
            .expect("send request");
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            RECEIPT.len(),
            RECEIPT
        );
        stream.write_all(resp.as_bytes()).expect("write");
    });
    (port, rx)
}

#[test]
#[ignore = "live HTTP over localhost — run manually"]
fn a_purchase_settles_over_the_wire() {
    let (port, seen) = start_endpoint();

    // A Bazaar stall, exactly as the market world carries it.
    let data = serde_json::json!({
        "item": "21010001", "name": "Veilwalker Blade", "price": 250, "currency": "gold"
    });
    let offer = offer_from("21010001", &serde_json::Value::Null, &data, "market");

    let mut client = CommerceClient::new(format!("http://127.0.0.1:{port}"))
        .with_token(Some("test-passport-token".into()));
    assert!(client.buy(offer), "the purchase starts");

    // Pump like a frame loop until the receipt lands (or timeout).
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let settled = loop {
        if let Some(result) = client.poll() {
            break result;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "purchase never settled"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    };

    // The client sent the exact server contract…
    let request = seen.recv().expect("endpoint saw the request");
    assert!(
        request.starts_with("POST /thread/purchase HTTP/1.1"),
        "path+method: {request}"
    );
    assert!(
        request
            .to_ascii_lowercase()
            .contains("authorization: bearer test-passport-token"),
        "the Passport rides as the bearer"
    );
    let body: serde_json::Value =
        serde_json::from_str(request.split("\r\n\r\n").nth(1).expect("body")).expect("json body");
    assert_eq!(body["worldId"], "market");
    assert_eq!(body["itemStructuredId"], "21010001");
    assert_eq!(body["priceRef"], "21010001");

    // …and read back the exact server response.
    let (offer, result) = settled;
    assert_eq!(offer.name, "Veilwalker Blade");
    let receipt = result.expect("purchase succeeds");
    assert!(receipt.grant.granted);
    assert!(
        !receipt.grant.persisted,
        "server's grant persistence is still stubbed — surfaced honestly"
    );
    assert_eq!(receipt.grant.reference, "thread:u1:market:21010001");
    assert_eq!(receipt.charge.amount, 25.0);
    assert!(receipt.charge.recorded);
}
