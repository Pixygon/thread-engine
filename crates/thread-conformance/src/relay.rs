//! Relay conformance — does a presence relay speak [presence-wire-v0.1]?
//!
//! [presence-wire-v0.1]: ../../../docs/spec/presence-wire-v0.1.md
//!
//! The substance is the **pure validators** — [`check_welcome`] and [`check_pose`]
//! turn a wire message into conformance clauses with no network, so the spec's
//! rules are pinned by unit tests today. [`probe`] is the thin live driver: it
//! connects to a relay, joins, and runs those validators on the real messages it
//! sees — ready the moment a relay is deployed.

use serde_json::Value;

use crate::{Clause, Severity};

/// Extract an array of exactly `n` finite floats at `key`, if present and well-formed.
fn floats(v: &Value, key: &str, n: usize) -> Option<Vec<f64>> {
    let arr = v.get(key)?.as_array()?;
    if arr.len() != n {
        return None;
    }
    let out: Vec<f64> = arr
        .iter()
        .filter_map(Value::as_f64)
        .filter(|f| f.is_finite())
        .collect();
    (out.len() == n).then_some(out)
}

/// Validate the `welcome` a relay sends in reply to `observe` (§2). An observer
/// watches without entering, so its welcome differs from a joiner's in exactly
/// two ways — it says so, and it is given the non-occupant id 0 — while still
/// carrying the roster **as it stands**, so a watcher arriving mid-session sees
/// who is already there rather than only who arrives next.
pub fn check_observer_welcome(msg: &Value) -> Vec<Clause> {
    let mut clauses = check_welcome(msg);
    clauses.push(Clause {
        name: "observe welcome is marked observer",
        severity: Severity::Error,
        pass: msg.get("observer").and_then(Value::as_bool) == Some(true),
        notes: match msg.get("observer") {
            Some(v) => vec![format!("observer = {v}")],
            None => vec!["no `observer` field — a watcher cannot tell it is one".into()],
        },
    });
    clauses.push(Clause {
        name: "observer is not an occupant (id 0)",
        severity: Severity::Error,
        pass: msg.get("id").and_then(Value::as_u64) == Some(0),
        notes: vec![],
    });
    clauses
}

/// Validate a `welcome` message (relay→client, in reply to `join`). Per §2 a
/// conformant welcome assigns an occupant `id`, lists `occupants`, and declares a
/// `tick_hz`.
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
            name: "welcome assigns an occupant id",
            severity: Severity::Error,
            pass: msg.get("id").and_then(Value::as_u64).is_some(),
            notes: vec![],
        },
        Clause {
            name: "welcome lists occupants",
            severity: Severity::Error,
            pass: msg.get("occupants").map(Value::is_array).unwrap_or(false),
            notes: vec![],
        },
        Clause {
            name: "welcome declares tick_hz",
            severity: Severity::Warn,
            pass: msg
                .get("tick_hz")
                .and_then(Value::as_u64)
                .is_some_and(|h| h > 0),
            notes: vec![],
        },
    ]
}

/// Validate a `pose` message (the hot path, §3–§4). `ts` (server stamp) and `v`
/// (velocity) are **mandatory** — they're exactly what a client needs to
/// interpolate without trusting peer clocks — so their absence is an Error.
pub fn check_pose(msg: &Value) -> Vec<Clause> {
    let is_pose = msg.get("t").and_then(Value::as_str) == Some("pose");
    let quat = floats(msg, "r", 4);
    let has_orientation = quat.is_some() || msg.get("y").and_then(Value::as_f64).is_some();
    // Unit-quaternion check only applies when a full quaternion was sent.
    let unit_quat = match &quat {
        Some(r) => {
            let len2 = r.iter().map(|x| x * x).sum::<f64>();
            (len2 - 1.0).abs() < 0.05
        }
        None => true, // yaw-only or absent → not applicable, don't penalize
    };

    vec![
        Clause {
            name: "pose tagged t=pose",
            severity: Severity::Error,
            pass: is_pose,
            notes: if is_pose {
                vec![]
            } else {
                vec![format!("t = {:?}", msg.get("t"))]
            },
        },
        Clause {
            name: "pose carries an occupant id",
            severity: Severity::Error,
            pass: msg.get("id").and_then(Value::as_u64).is_some(),
            notes: vec![],
        },
        Clause {
            name: "pose is server-stamped (ts)",
            severity: Severity::Error,
            pass: msg.get("ts").and_then(Value::as_u64).is_some(),
            notes: vec!["ts is mandatory: the single clock source for interpolation".into()],
        },
        Clause {
            name: "pose has a position [f32;3]",
            severity: Severity::Error,
            pass: floats(msg, "p", 3).is_some(),
            notes: vec![],
        },
        Clause {
            name: "pose has a velocity [f32;3]",
            severity: Severity::Error,
            pass: floats(msg, "v", 3).is_some(),
            notes: vec!["v is mandatory: enables gap extrapolation".into()],
        },
        Clause {
            name: "pose has an orientation (r xyzw or yaw y)",
            severity: Severity::Error,
            pass: has_orientation,
            notes: vec![],
        },
        Clause {
            name: "orientation is a unit quaternion",
            severity: Severity::Warn,
            pass: unit_quat,
            notes: vec![],
        },
        Clause {
            name: "pose carries an animation state (a)",
            severity: Severity::Warn,
            pass: msg.get("a").and_then(Value::as_u64).is_some(),
            notes: vec![],
        },
    ]
}

/// The result of a live relay probe.
pub struct RelayOutcome {
    /// Whether the WebSocket connected at all.
    pub connected: bool,
    pub clauses: Vec<Clause>,
    /// Free-form observations (occupant count, whether a pose was seen, …).
    pub notes: Vec<String>,
}

/// Connect to `url` (`wss://<relay>/thread/<worldId>`), `join` with `passport`, and
/// run the wire validators on what the relay sends back. Best-effort: with a single
/// probe client the relay may not fan a pose back (area-of-interest / no peers), so
/// a missing pose is a note, not a failure — the pinned validators carry the spec.
pub async fn probe(url: &str, passport: &str, timeout_ms: u64) -> RelayOutcome {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let mut clauses = Vec::new();
    let mut notes = Vec::new();

    let ws = match tokio_tungstenite::connect_async(url).await {
        Ok((ws, _)) => ws,
        Err(e) => {
            clauses.push(Clause {
                name: "relay reachable (WebSocket)",
                severity: Severity::Error,
                pass: false,
                notes: vec![e.to_string()],
            });
            return RelayOutcome {
                connected: false,
                clauses,
                notes,
            };
        }
    };
    clauses.push(Clause {
        name: "relay reachable (WebSocket)",
        severity: Severity::Error,
        pass: true,
        notes: vec![],
    });

    let (mut write, mut read) = ws.split();
    let join = serde_json::json!({ "t": "join", "passport": passport, "spawn": null });
    let _ = write.send(Message::Text(join.to_string())).await;

    // Also emit one pose so a relay that echoes gives us something to validate.
    let pose = serde_json::json!({
        "t": "pose", "id": 0, "ts": 0, "p": [0.0, 0.0, 0.0], "r": [0.0, 0.0, 0.0, 1.0], "v": [0.0, 0.0, 0.0], "a": 0
    });
    let _ = write.send(Message::Text(pose.to_string())).await;

    let deadline = tokio::time::Duration::from_millis(timeout_ms);
    let mut saw_welcome = false;
    let mut saw_pose = false;

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
                    if let Some(occ) = v.get("occupants").and_then(Value::as_array) {
                        notes.push(format!("relay reports {} occupant(s)", occ.len()));
                    }
                }
                Some("pose") if !saw_pose => {
                    saw_pose = true;
                    clauses.extend(check_pose(&v));
                }
                _ => {}
            }
            if saw_welcome && saw_pose {
                break;
            }
        }
    })
    .await;

    if !saw_welcome {
        clauses.push(Clause {
            name: "relay replies welcome on join",
            severity: Severity::Error,
            pass: false,
            notes: vec!["no welcome within timeout (Passport rejected? wrong path?)".into()],
        });
    }
    if !saw_pose {
        notes.push("no pose fan-out observed (expected with a lone probe client)".into());
    }

    // A second connection, watching rather than entering. Done on its own
    // socket because that is how a status bar or a directory actually reads a
    // world — and because the interesting property is one a joined client
    // cannot test: that the watcher is *not* in the room it can see.
    match tokio_tungstenite::connect_async(url).await {
        Ok((ws2, _)) => {
            let (mut w2, mut r2) = ws2.split();
            let _ = w2
                .send(Message::Text(serde_json::json!({ "t": "observe" }).to_string()))
                .await;
            let mut welcomed = false;
            let mut poses_leaked = 0usize;
            let _ = tokio::time::timeout(deadline, async {
                while let Some(Ok(msg)) = r2.next().await {
                    let Ok(text) = msg.into_text() else { continue };
                    let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
                    match v.get("t").and_then(Value::as_str) {
                        Some("welcome") if !welcomed => {
                            welcomed = true;
                            clauses.extend(check_observer_welcome(&v));
                        }
                        Some("pose") => poses_leaked += 1,
                        _ => {}
                    }
                }
            })
            .await;
            if welcomed {
                clauses.push(Clause {
                    name: "observers are sent no poses",
                    severity: Severity::Error,
                    pass: poses_leaked == 0,
                    notes: if poses_leaked == 0 {
                        vec![]
                    } else {
                        vec![format!("{poses_leaked} pose frame(s) reached a watcher")]
                    },
                });
            } else {
                notes.push(
                    "relay did not answer `observe` — watching without entering is unsupported \
                     (presence-wire §2)"
                        .into(),
                );
            }
        }
        Err(e) => notes.push(format!("could not open a second socket to watch: {e}")),
    }

    RelayOutcome {
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

/// The observe verb's own shape. Written from an outside implementer's
    /// probe — a third language, against the live relay — which found the
    /// roster populated on the first frame; that property is the difference
    /// between a status bar that knows who is in a room and one that only
    /// learns about the next arrival.
    #[test]
    fn a_spec_observer_welcome_passes_and_a_joiners_does_not() {
        let w = json!({
            "t": "welcome", "id": 0, "observer": true,
            "occupants": [{ "id": 1 }], "tick_hz": 15
        });
        let clauses = check_observer_welcome(&w);
        assert!(clauses_pass(&clauses), "{clauses:?}");
        assert_eq!(
            w["occupants"].as_array().unwrap().len(),
            1,
            "the roster stands as it is, not as it will be"
        );

        // A relay that answers `observe` with an ordinary welcome has given the
        // watcher a body: it cannot tell it is observing, and it holds an
        // occupant id somebody else's roster will disagree about.
        let joiner = json!({ "t": "welcome", "id": 3, "occupants": [], "tick_hz": 15 });
        let clauses = check_observer_welcome(&joiner);
        assert!(!clauses_pass(&clauses));
        assert!(clauses.iter().any(|c| c.name == "observe welcome is marked observer" && !c.pass));
        assert!(clauses.iter().any(|c| c.name == "observer is not an occupant (id 0)" && !c.pass));
    }

    #[test]
    fn a_spec_welcome_passes() {
        let w = json!({ "t": "welcome", "id": 5, "occupants": [{ "id": 7 }, { "id": 9 }], "tick_hz": 15 });
        let clauses = check_welcome(&w);
        assert!(clauses_pass(&clauses), "{clauses:?}");
        assert!(clauses.iter().all(|c| c.pass));
    }

    #[test]
    fn a_welcome_without_an_id_fails() {
        let w = json!({ "t": "welcome", "occupants": [] });
        let clauses = check_welcome(&w);
        assert!(!clauses_pass(&clauses));
        let id = clauses
            .iter()
            .find(|c| c.name == "welcome assigns an occupant id")
            .unwrap();
        assert!(!id.pass);
        assert_eq!(id.severity, Severity::Error);
    }

    #[test]
    fn a_full_spec_pose_passes() {
        let p = json!({ "t": "pose", "id": 7, "ts": 123456, "p": [3.0, 0.0, -1.0], "r": [0.0, 0.0, 0.0, 1.0], "v": [1.0, 0.0, 0.0], "a": 2 });
        let clauses = check_pose(&p);
        assert!(clauses_pass(&clauses));
        assert!(clauses.iter().all(|c| c.pass), "{clauses:?}");
    }

    #[test]
    fn a_pose_missing_ts_or_velocity_is_an_error() {
        let no_ts = json!({ "t": "pose", "id": 7, "p": [0.0, 0.0, 0.0], "r": [0.0, 0.0, 0.0, 1.0], "v": [0.0, 0.0, 0.0], "a": 0 });
        assert!(!clauses_pass(&check_pose(&no_ts)), "ts is mandatory");

        let no_v = json!({ "t": "pose", "id": 7, "ts": 1, "p": [0.0, 0.0, 0.0], "r": [0.0, 0.0, 0.0, 1.0], "a": 0 });
        assert!(!clauses_pass(&check_pose(&no_v)), "v is mandatory");
    }

    #[test]
    fn yaw_only_orientation_is_accepted() {
        // §3 permits a yaw-only pose (`y`) instead of a full quaternion.
        let p = json!({ "t": "pose", "id": 7, "ts": 1, "p": [0.0, 0.0, 0.0], "y": 1.57, "v": [0.0, 0.0, 0.0], "a": 1 });
        let clauses = check_pose(&p);
        assert!(clauses_pass(&clauses), "{clauses:?}");
        let orient = clauses
            .iter()
            .find(|c| c.name.contains("orientation is a unit"))
            .unwrap();
        assert!(
            orient.pass,
            "unit-quat check must not penalize a yaw-only pose"
        );
    }

    #[test]
    fn a_non_unit_quaternion_warns_but_does_not_fail() {
        let p = json!({ "t": "pose", "id": 7, "ts": 1, "p": [0.0, 0.0, 0.0], "r": [1.0, 1.0, 0.0, 0.0], "v": [0.0, 0.0, 0.0], "a": 0 });
        let clauses = check_pose(&p);
        assert!(
            clauses_pass(&clauses),
            "non-unit quat is a warning, not a failure"
        );
        let unit = clauses
            .iter()
            .find(|c| c.name.contains("unit quaternion"))
            .unwrap();
        assert!(!unit.pass);
        assert_eq!(unit.severity, Severity::Warn);
    }
}
