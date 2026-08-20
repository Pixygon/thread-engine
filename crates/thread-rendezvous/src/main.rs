//! `thread-rendezvous` — a self-hostable reference P2P rendezvous for the Thread.
//!
//! The serverless-presence introducer (presence-topology-v0.1 §3.2): peers meet
//! here, exchange WebRTC signaling, then talk *directly* — no pose ever transits
//! this service, so it stays tiny and stateless. Run one anywhere (a cheap VPS,
//! `docker run`, a Raspberry Pi); a world names its own in `presence.rendezvous`.
//!
//! ```text
//! thread-rendezvous                            # ws://0.0.0.0:4100
//! THREAD_RENDEZVOUS_ADDR=0.0.0.0:9100 thread-rendezvous
//! ```
//!
//! Speaks plain WebSocket (`ws://`); terminate TLS (`wss://`) at a reverse proxy
//! (Caddy/nginx). Clients connect to `…/rtc/<key>`; the `/rtc` path distinguishes
//! a rendezvous from a relay's `/thread/<key>` when both share a host.

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "thread_rendezvous=info".into()),
        )
        .init();

    let addr = std::env::var("THREAD_RENDEZVOUS_ADDR").unwrap_or_else(|_| "0.0.0.0:4100".into());
    let listener = tokio::net::TcpListener::bind(&addr).await.expect("bind");
    tracing::info!("thread-rendezvous listening on ws://{addr}  (rooms at /rtc/<key>)");
    thread_rendezvous::serve_listener(listener).await;
}
