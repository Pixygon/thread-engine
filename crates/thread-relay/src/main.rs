//! `thread-relay` — a self-hostable reference presence relay for the Thread.
//!
//! Run one anywhere (a cheap VPS, `docker run`, a Raspberry Pi). A world names its
//! own relay(s) in its manifest, so this is one relay among many — presence on the
//! Thread is federated, and survives any single operator disappearing.
//!
//! ```text
//! thread-relay                       # ws://0.0.0.0:4000, unlimited AoI
//! THREAD_RELAY_ADDR=0.0.0.0:9000 \
//! THREAD_RELAY_AOI=80 \              # metres; 0 = unlimited
//! THREAD_RELAY_TICK=15 thread-relay
//! ```
//!
//! Speaks plain WebSocket (`ws://`); terminate TLS (`wss://`) at a reverse proxy
//! (Caddy/nginx) — the standard, simplest way to keep the relay itself lean. Clients
//! connect to `…/thread/:worldId`; occupants are grouped by that world id.

use tokio::net::TcpListener;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "thread_relay=info".into()),
        )
        .init();

    let addr = std::env::var("THREAD_RELAY_ADDR").unwrap_or_else(|_| "0.0.0.0:4000".into());
    let aoi: f32 = std::env::var("THREAD_RELAY_AOI")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0);
    let tick: u32 = std::env::var("THREAD_RELAY_TICK")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(15);

    let listener = TcpListener::bind(&addr).await.expect("bind");
    tracing::info!(
        "thread-relay listening on ws://{addr}  (AoI {}, {tick} Hz)",
        if aoi <= 0.0 {
            "unlimited".into()
        } else {
            format!("{aoi} m")
        }
    );

    thread_relay::serve_listener(listener, aoi, tick).await;
}
