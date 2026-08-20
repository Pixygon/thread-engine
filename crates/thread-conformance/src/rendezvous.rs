//! Rendezvous conformance — does a P2P rendezvous speak [presence-topology-v0.1] §3.2?
//!
//! [presence-topology-v0.1]: https://github.com/Pixygon/thread-spec/blob/main/specs/presence-topology-v0.1.md
//!
//! Same shape as the relay checker: **pure validators** ([`check_welcome`],
//! [`check_peers`]) pin the wire rules with no network, and [`probe`] is the thin
//! live driver — it announces to a room and validates the introduction sequence the
//! rendezvous sends back (`welcome` **before** `peers`; the ordering contract
//! clients rely on to adopt their assigned id before negotiating).

use serde_json::Value;

use crate::{Clause, Severity};

/// Validate a rendezvous `welcome` message (rv→client, in reply to `announce`).
/// Distinct from a relay's welcome: just `{t:"welcome", id:<u32>}` — no occupants,
/// no tick_hz (the rendezvous never sees a pose, so it has no rate to suggest).
pub fn check_welcome(msg: &Value) -> Vec<Clause> {
    let is_welcome = msg.get("t").and_then(Value::as_str) == Some("welcome");
    vec![
        Clause {
            name: "welcome tagged t=welcome",
            severity: Severity::Error,
            pass: is_welcome,
            notes: if is_welcome {
                vec![]
            } else {
                vec![format!("t = {:?}", msg.get("t"))]
            },
        },
        Clause {
            name: "welcome assigns a peer id",
            severity: Severity::Error,
            pass: msg.get("id").and_then(Value::as_u64).is_some(),
            notes: vec!["clients MUST adopt the assigned id before negotiating".into()],
        },
    ]
}

/// Validate a `peers` message (rv→client: who's present, or a join delta).
pub fn check_peers(msg: &Value) -> Vec<Clause> {
    let is_peers = msg.get("t").and_then(Value::as_str) == Some("peers");
    let all_ids = msg
        .get("peers")
        .and_then(Value::as_array)
        .map(|a| a.iter().all(|p| p.as_u64().is_some()))
        .unwrap_or(false);
    vec![
        Clause {
            name: "peers tagged t=peers",
            severity: Severity::Error,
            pass: is_peers,
            notes: if is_peers {
                vec![]
            } else {
                vec![format!("t = {:?}", msg.get("t"))]
            },
        },
        Clause {
            name: "peers is an array of peer ids",
            severity: Severity::Error,
            pass: all_ids,
            notes: vec![],
        },
    ]
}

/// The result of a live rendezvous probe.
pub struct RendezvousOutcome {
    /// Whether the WebSocket connected at all.
    pub connected: bool,
    pub clauses: Vec<Clause>,
    /// Free-form observations (peer count, whether welcome was assigned, …).
    pub notes: Vec<String>,
}

/// Connect to `url` (`wss://<host>/rtc/<key>`), `announce`, and validate the
/// introduction the rendezvous sends back. Per §3.2 `welcome` is OPTIONAL (a
/// minimal rendezvous may let clients keep provisional ids) but `peers` is not;
/// and when `welcome` IS sent, it must arrive **before** `peers`.
pub async fn probe(url: &str, timeout_ms: u64) -> RendezvousOutcome {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let mut clauses = Vec::new();
    let mut notes = Vec::new();

    let ws = match tokio_tungstenite::connect_async(url).await {
        Ok((ws, _)) => ws,
        Err(e) => {
            clauses.push(Clause {
                name: "rendezvous reachable (WebSocket)",
                severity: Severity::Error,
                pass: false,
                notes: vec![e.to_string()],
            });
            return RendezvousOutcome {
                connected: false,
                clauses,
                notes,
            };
        }
    };
    clauses.push(Clause {
        name: "rendezvous reachable (WebSocket)",
        severity: Severity::Error,
        pass: true,
        notes: vec![],
    });

    let (mut write, mut read) = ws.split();
    let announce =
        serde_json::json!({ "t": "announce", "peer": 4242, "world": "conformance-probe" });
    let _ = write.send(Message::Text(announce.to_string())).await;

    let deadline = tokio::time::Duration::from_millis(timeout_ms);
    let mut saw_welcome = false;
    let mut saw_peers = false;

    let _ = tokio::time::timeout(deadline, async {
        while let Some(Ok(msg)) = read.next().await {
            let Ok(text) = msg.into_text() else { continue };
            let Ok(v) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            match v.get("t").and_then(Value::as_str) {
                Some("welcome") if !saw_welcome => {
                    saw_welcome = true;
                    clauses.extend(check_welcome(&v));
                    // The ordering contract: welcome must precede peers.
                    clauses.push(Clause {
                        name: "welcome arrives before peers",
                        severity: Severity::Error,
                        pass: !saw_peers,
                        notes: vec![],
                    });
                }
                Some("peers") if !saw_peers => {
                    saw_peers = true;
                    clauses.extend(check_peers(&v));
                    if let Some(p) = v.get("peers").and_then(Value::as_array) {
                        notes.push(format!("room reports {} peer(s) present", p.len()));
                    }
                }
                _ => {}
            }
            if saw_peers {
                break; // peers is the last message of the introduction
            }
        }
    })
    .await;

    if !saw_peers {
        clauses.push(Clause {
            name: "rendezvous replies peers on announce",
            severity: Severity::Error,
            pass: false,
            notes: vec!["no peers within timeout (wrong path? not a rendezvous?)".into()],
        });
    }
    if !saw_welcome {
        notes.push(
            "no welcome{id} — minimal rendezvous, clients keep provisional ids (allowed)".into(),
        );
    }

    RendezvousOutcome {
        connected: true,
        clauses,
        notes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clauses_pass;
    use serde_json::json;

    #[test]
    fn a_spec_welcome_passes() {
        let w = json!({ "t": "welcome", "id": 3 });
        let clauses = check_welcome(&w);
        assert!(clauses_pass(&clauses), "{clauses:?}");
        assert!(clauses.iter().all(|c| c.pass));
    }

    #[test]
    fn a_welcome_without_an_id_fails() {
        let clauses = check_welcome(&json!({ "t": "welcome" }));
        assert!(!clauses_pass(&clauses));
    }

    #[test]
    fn spec_peers_messages_pass_including_the_empty_room_and_the_join_delta() {
        for p in [
            json!({ "t": "peers", "peers": [] }),
            json!({ "t": "peers", "peers": [1, 2] }),
            json!({ "t": "peers", "peers": [7] }),
        ] {
            let clauses = check_peers(&p);
            assert!(clauses_pass(&clauses), "{p} → {clauses:?}");
        }
    }

    #[test]
    fn malformed_peers_fail() {
        // Not an array of ids.
        assert!(!clauses_pass(&check_peers(
            &json!({ "t": "peers", "peers": ["a"] })
        )));
        assert!(!clauses_pass(&check_peers(&json!({ "t": "peers" }))));
    }
}
