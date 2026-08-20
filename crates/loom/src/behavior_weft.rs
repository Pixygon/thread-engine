//! Weft behaviors — the Thread's **native** code, running beside the WASM
//! floor (weft-v0.1 · behavior-abi-v0.1).
//!
//! A manifest behavior may name a `weft` asset instead of a `wasm` one: a
//! serialized [`weft::Module`]. It is **verified on load** — types, transitive
//! effect rows, contracts, static fuel bound — and evaluated per event on the
//! reference interpreter. Always available (no feature gate, no runtime dep):
//! the native path is lighter than the floor.
//!
//! The behavior contract mirrors the ABI's: the entry definition is
//! `(state, event) → { state, actions }`. The browser constructs the event by
//! **projecting the declared event type** — a behavior receives exactly the
//! fields its type asks for, defaulted where the browser has nothing to offer.
//! Total in, total out: a Weft behavior cannot crash a world, run away, or
//! perform an effect its row doesn't declare.

use std::collections::BTreeMap;

use weft::{EffectKind, Module, Ty, Value};

use crate::behavior::Action;

/// A loaded, verified Weft behavior: the module, its persistent state value,
/// and the fuel ceiling (static bound + contract headroom).
pub struct WeftBehavior {
    module: Module,
    state: Value,
    fuel: u64,
}

impl WeftBehavior {
    /// Parse + verify a serialized module (the `weft` asset's JSON form).
    /// Verification here is the trust boundary — nothing unverified evaluates.
    pub fn from_json(text: &str) -> Result<Self, String> {
        let module: Module = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let certs = weft::verify_module(&module).map_err(|e| e.to_string())?;
        let cert = certs
            .get(&module.entry)
            .ok_or("module has no entry certificate")?;
        let entry = module
            .defs
            .get(&module.entry)
            .ok_or("module has no entry def")?;
        if entry.params.len() != 2 {
            return Err("a weft behavior's entry must take (state, event)".into());
        }
        let state = default_value(&entry.params[0]);
        // Contracts evaluate inside the same budget; give them headroom.
        Ok(Self {
            fuel: cert.fuel_bound + 1024,
            module,
            state,
        })
    }

    /// Load a behavior from a **package** (`.weftpack.json`) by export petname
    /// — the `weft-use` path: verify the package, link the export's closure
    /// into a module, then the usual entry checks. One published export, one
    /// line of markup, a running behavior.
    pub fn from_package_json(text: &str, export: &str) -> Result<Self, String> {
        let pkg: weft::pack::Package = serde_json::from_str(text).map_err(|e| e.to_string())?;
        pkg.verify().map_err(|e| e.to_string())?;
        let hash = pkg
            .export(export)
            .ok_or_else(|| format!("package '{}' has no export '{export}'", pkg.name))?;
        let entry_def = pkg
            .defs
            .get(&hash)
            .cloned()
            .ok_or("export verified to exist")?;
        let module = weft::pack::link(&[pkg], vec![entry_def], 0).map_err(|e| e.to_string())?;
        Self::from_json(&serde_json::to_string(&module).map_err(|e| e.to_string())?)
    }

    /// The effects this behavior may request (its verified row) — for HUD /
    /// permission surfaces.
    pub fn effects(&self) -> Vec<EffectKind> {
        self.module.defs[&self.module.entry]
            .effects
            .iter()
            .copied()
            .collect()
    }

    /// Dispatch one interact event. State persists across calls; failures
    /// (fuel, contract) degrade to no actions — the world keeps rendering.
    pub fn on_interact(&mut self, name: &str, slug: Option<&str>) -> Vec<Action> {
        let entry = &self.module.defs[&self.module.entry];
        let event = project_event(
            &entry.params[1],
            &[
                ("name", Value::Text(name.to_string())),
                ("slug", Value::Text(slug.unwrap_or("").to_string())),
            ],
        );
        let out = match weft::eval_call(
            &self.module,
            self.module.entry,
            vec![self.state.clone(), event],
            self.fuel,
        ) {
            Ok(o) => o,
            Err(e) => {
                tracing::warn!("weft behavior: {e}");
                return Vec::new();
            }
        };
        let Value::Rec(mut rec) = out.value else {
            return Vec::new();
        };
        if let Some(st) = rec.remove("state") {
            self.state = st;
        }
        match rec.remove("actions") {
            Some(Value::List(actions)) => actions.into_iter().filter_map(to_action).collect(),
            _ => Vec::new(),
        }
    }

    /// Dispatch one tick. The event record offers `kind: "tick"` and `dt_ms`
    /// (the browser throttles cadence); a behavior's event type picks the
    /// fields it wants, same total projection as interact. This is what makes
    /// a world *behave* rather than only react — native code with a heartbeat,
    /// still fuel-metered every beat.
    pub fn on_tick(&mut self, dt_ms: i64) -> Vec<Action> {
        let entry = &self.module.defs[&self.module.entry];
        let event = project_event(
            &entry.params[1],
            &[
                ("kind", Value::Text("tick".into())),
                ("dt_ms", Value::Int(dt_ms)),
            ],
        );
        let out = match weft::eval_call(
            &self.module,
            self.module.entry,
            vec![self.state.clone(), event],
            self.fuel,
        ) {
            Ok(o) => o,
            Err(e) => {
                tracing::warn!("weft tick: {e}");
                return Vec::new();
            }
        };
        let Value::Rec(mut rec) = out.value else {
            return Vec::new();
        };
        if let Some(st) = rec.remove("state") {
            self.state = st;
        }
        match rec.remove("actions") {
            Some(Value::List(actions)) => actions.into_iter().filter_map(to_action).collect(),
            _ => Vec::new(),
        }
    }
}

/// The zero value of a type — behavior state starts here.
fn default_value(ty: &Ty) -> Value {
    match ty {
        Ty::Int => Value::Int(0),
        Ty::Fix => Value::Fix(0),
        Ty::Bool => Value::Bool(false),
        Ty::Text => Value::Text(String::new()),
        Ty::Action => Value::List(Vec::new()), // never used as state; safe filler
        Ty::List(_) => Value::List(Vec::new()),
        Ty::Record(fields) => Value::Rec(
            fields
                .iter()
                .map(|(k, t)| (k.clone(), default_value(t)))
                .collect(),
        ),
    }
}

/// Build the event record a behavior's declared type asks for: named fields
/// the browser can supply (type-matched), everything else defaulted. Total —
/// a behavior can never be handed a value outside its type.
fn project_event(ty: &Ty, available: &[(&str, Value)]) -> Value {
    let Ty::Record(fields) = ty else {
        return default_value(ty);
    };
    let mut out = BTreeMap::new();
    for (k, t) in fields {
        let supplied = available
            .iter()
            .find(|(name, v)| name == k && type_of(v) == *t)
            .map(|(_, v)| v.clone());
        out.insert(k.clone(), supplied.unwrap_or_else(|| default_value(t)));
    }
    Value::Rec(out)
}

fn type_of(v: &Value) -> Ty {
    match v {
        Value::Int(_) => Ty::Int,
        Value::Fix(_) => Ty::Fix,
        Value::Bool(_) => Ty::Bool,
        Value::Text(_) => Ty::Text,
        Value::Action { .. } => Ty::Action,
        Value::List(_) => Ty::List(Box::new(Ty::Action)),
        Value::Rec(fs) => Ty::Record(fs.iter().map(|(k, x)| (k.clone(), type_of(x))).collect()),
    }
}

/// Map a constructed Weft effect value onto the browser's [`Action`] set.
fn to_action(v: Value) -> Option<Action> {
    let Value::Action { kind, fields } = v else {
        return None;
    };
    let text = |k: &str| match fields.get(k) {
        Some(Value::Text(s)) => s.clone(),
        _ => String::new(),
    };
    let int = |k: &str| match fields.get(k) {
        Some(Value::Int(i)) => *i,
        _ => 0,
    };
    Some(match kind {
        EffectKind::Notify => Action::Notify {
            text: text("text"),
            level: None,
        },
        EffectKind::Navigate => Action::Navigate { to: text("to") },
        EffectKind::CodexOpen => Action::CodexOpen { slug: text("slug") },
        EffectKind::CommerceBuy => Action::CommerceBuy {
            item: text("item"),
            price_ref: serde_json::Value::from(int("price_ref")),
        },
        EffectKind::GiveItem => Action::GiveItem {
            item: int("item") as u16,
            count: int("count").max(1) as u32,
        },
        EffectKind::Despawn => Action::Despawn {
            position: [0.0, 0.0, 0.0],
        },
        // The scene ABI: `set_state` names a placement and patches its visible
        // state — every non-`placement` field rides along as the JSON patch
        // (a `text` field re-typesets a panel; more verbs join over time).
        EffectKind::SetState => {
            let placement = match fields.get("placement") {
                Some(Value::Text(s)) if !s.is_empty() => Some(s.clone()),
                _ => None,
            };
            let mut patch = serde_json::Map::new();
            for (k, v) in &fields {
                if k != "placement" {
                    patch.insert(k.clone(), value_to_json(v));
                }
            }
            Action::SetState {
                placement,
                patch: serde_json::Value::Object(patch),
            }
        }
        // Weft is integer-exact, so spatial fields cross in centimetres and
        // colors in 0–255 — the mapping to browser floats happens here, once.
        EffectKind::Spawn => {
            // Spatial fields: Fix metres first-class (`x`/`y`/`z`/`scale`),
            // integer centimetres kept for older modules.
            let fix = |k: &str| match fields.get(k) {
                Some(Value::Fix(raw)) => Some(*raw as f32 / weft::FIX_SCALE as f32),
                _ => None,
            };
            let axis = |m: &str, cm: &str| fix(m).unwrap_or_else(|| int(cm) as f32 / 100.0);
            Action::Spawn {
                builtin: {
                    let b = text("builtin");
                    if b.is_empty() {
                        "sphere".to_string()
                    } else {
                        b
                    }
                },
                position: [axis("x", "x_cm"), axis("y", "y_cm"), axis("z", "z_cm")],
                scale: fix("scale")
                    .unwrap_or_else(|| int("scale_cm").max(1) as f32 / 100.0)
                    .clamp(0.01, 50.0),
                color: [
                    (int("r").clamp(0, 255) as f32) / 255.0,
                    (int("g").clamp(0, 255) as f32) / 255.0,
                    (int("b").clamp(0, 255) as f32) / 255.0,
                ],
                name: text("name"),
                animate: text("animate"),
                anim_speed: fix("speed").unwrap_or(1.0),
                anim_amp: fix("amp").unwrap_or(0.25),
            }
        }
        // No browser-side mapping yet — dropped, never invented.
        EffectKind::PresenceEmit => return None,
    })
}

/// A Weft value as JSON — for the `set_state` patch (data crossing the seam).
fn value_to_json(v: &Value) -> serde_json::Value {
    match v {
        Value::Int(i) => serde_json::Value::from(*i),
        // Fixed-point crosses the seam as a plain number — millionths → f64
        // exactly (i64 magnitudes here are far under 2^53).
        Value::Fix(raw) => serde_json::Value::from(*raw as f64 / weft::FIX_SCALE as f64),
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Text(s) => serde_json::Value::String(s.clone()),
        Value::List(xs) => serde_json::Value::Array(xs.iter().map(value_to_json).collect()),
        Value::Rec(fs) => serde_json::Value::Object(
            fs.iter()
                .map(|(k, x)| (k.clone(), value_to_json(x)))
                .collect(),
        ),
        Value::Action { .. } => serde_json::Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use weft::{Def, PrimOp, Term};

    /// A ticking module: counts beats in state and rewrites a plaque each
    /// beat — the heartbeat path end to end (project tick event → eval →
    /// set_state action), state persisting across ticks.
    #[test]
    fn tick_dispatch_counts_and_rewrites() {
        use weft::EffectKind as EK;
        let state_ty = rec_ty(vec![("beats", Ty::Int)]);
        let out_ty = rec_ty(vec![
            ("actions", Ty::List(Box::new(Ty::Action))),
            ("state", state_ty.clone()),
        ]);
        let body = Term::Let(
            Box::new(Term::Prim(
                PrimOp::Add,
                vec![
                    Term::Get(Box::new(Term::Var(1)), "beats".into()),
                    Term::Int(1),
                ],
            )),
            Box::new(Term::Rec(
                [
                    (
                        "actions".to_string(),
                        Term::ListNew(vec![Term::Effect(
                            EK::SetState,
                            [
                                ("placement".to_string(), Term::Text("clock".into())),
                                (
                                    "text".to_string(),
                                    Term::Prim(
                                        PrimOp::Concat,
                                        vec![
                                            Term::Text("Beats: ".into()),
                                            Term::Prim(PrimOp::ToText, vec![Term::Var(0)]),
                                        ],
                                    ),
                                ),
                            ]
                            .into(),
                        )]),
                    ),
                    (
                        "state".to_string(),
                        Term::Rec([("beats".to_string(), Term::Var(0))].into()),
                    ),
                ]
                .into(),
            )),
        );
        let def = weft::Def {
            params: vec![state_ty, rec_ty(vec![])],
            ret: out_ty,
            effects: BTreeSet::from([EK::SetState]),
            body,
            pre: None,
            post: None,
        };
        let module = weft::Module::build(vec![def], 0).unwrap();
        let json = serde_json::to_string(&module).unwrap();
        let mut b = WeftBehavior::from_json(&json).expect("verifies");
        let a1 = b.on_tick(250);
        let a2 = b.on_tick(250);
        let patch_text = |a: &[Action]| match &a[0] {
            Action::SetState { placement, patch } => {
                assert_eq!(placement.as_deref(), Some("clock"));
                patch["text"].as_str().unwrap().to_string()
            }
            other => panic!("wrong action {other:?}"),
        };
        assert_eq!(patch_text(&a1), "Beats: 1");
        assert_eq!(patch_text(&a2), "Beats: 2", "state persists across ticks");
    }

    /// The scene ABI's wire shape: a Weft `set_state` effect becomes an
    /// `Action::SetState` whose non-`placement` fields ride as the JSON patch.
    #[test]
    fn set_state_effects_map_to_scene_patches() {
        let v = Value::Action {
            kind: EffectKind::SetState,
            fields: [
                ("placement".to_string(), Value::Text("visitors".into())),
                (
                    "text".to_string(),
                    Value::Text("Visitors this session\n3".into()),
                ),
                ("count".to_string(), Value::Int(3)),
            ]
            .into(),
        };
        match to_action(v).expect("maps") {
            Action::SetState { placement, patch } => {
                assert_eq!(placement.as_deref(), Some("visitors"));
                assert_eq!(patch["text"], "Visitors this session\n3");
                assert_eq!(patch["count"], 3);
                assert!(patch.get("placement").is_none());
            }
            other => panic!("wrong action: {other:?}"),
        }
    }

    fn rec_ty(fields: Vec<(&str, Ty)>) -> Ty {
        Ty::Record(
            fields
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        )
    }

    /// A greeter behavior in Weft: counts visits in state, notifies with the
    /// interacted object's name — proving verify→project→eval→Action end to end.
    fn greeter_json() -> String {
        let state_ty = rec_ty(vec![("visits", Ty::Int)]);
        let event_ty = rec_ty(vec![("name", Ty::Text)]);
        let out_ty = rec_ty(vec![
            ("actions", Ty::List(Box::new(Ty::Action))),
            ("state", state_ty.clone()),
        ]);
        // params: [state=Var1, event=Var0]
        let body = Term::Rec(
            [
                (
                    "actions".to_string(),
                    Term::ListNew(vec![Term::Effect(
                        EffectKind::Notify,
                        [(
                            "text".to_string(),
                            Term::Prim(
                                PrimOp::Concat,
                                vec![
                                    Term::Text("Welcome to ".into()),
                                    Term::Get(Box::new(Term::Var(0)), "name".into()),
                                ],
                            ),
                        )]
                        .into(),
                    )]),
                ),
                (
                    "state".to_string(),
                    Term::Rec(
                        [(
                            "visits".to_string(),
                            Term::Prim(
                                PrimOp::Add,
                                vec![
                                    Term::Get(Box::new(Term::Var(1)), "visits".into()),
                                    Term::Int(1),
                                ],
                            ),
                        )]
                        .into(),
                    ),
                ),
            ]
            .into(),
        );
        let def = Def {
            params: vec![state_ty, event_ty],
            ret: out_ty,
            effects: BTreeSet::from([EffectKind::Notify]),
            body,
            pre: None,
            post: None,
        };
        serde_json::to_string(&Module::build(vec![def], 0).unwrap()).unwrap()
    }

    #[test]
    fn a_weft_behavior_verifies_projects_and_acts() {
        let mut b = WeftBehavior::from_json(&greeter_json()).expect("verifies + loads");
        assert_eq!(b.effects(), vec![EffectKind::Notify]);
        let actions = b.on_interact("The Waystone", None);
        assert_eq!(
            actions,
            vec![Action::Notify {
                text: "Welcome to The Waystone".into(),
                level: None
            }]
        );
        // State persisted: the visit counter climbed (observable determinism —
        // interact twice, state advanced twice).
        b.on_interact("The Waystone", None);
        let Value::Rec(st) = &b.state else { panic!() };
        assert_eq!(st["visits"], Value::Int(2));
    }

    /// Tier-2 datapoint (run with `--ignored --nocapture`): events/second
    /// through the reference interpreter, greeter behavior, release profile
    /// recommended. `bench_tier2_weft_vs_wasmi` is the matched pair.
    #[test]
    #[ignore]
    fn bench_weft_events_per_second() {
        let mut b = WeftBehavior::from_json(&greeter_json()).unwrap();
        let n = 200_000u32;
        let t0 = std::time::Instant::now();
        let mut total_actions = 0usize;
        for _ in 0..n {
            total_actions += b.on_interact("The Waystone", None).len();
        }
        let secs = t0.elapsed().as_secs_f64();
        println!(
            "BENCH weft events/sec = {:.0} ({n} events in {secs:.3}s, {total_actions} actions)",
            n as f64 / secs
        );
        assert_eq!(total_actions, n as usize);
    }

    /// Tier 2, the matched pair: the SAME observable behavior — one interact →
    /// one notify action — through both production paths: Weft's verified
    /// interpreter vs. a WASM module on wasmi (the Behavior ABI floor, with
    /// its real JSON-over-linear-memory marshalling). Run:
    /// `cargo test --release -p loom --features behaviors bench_tier2 -- --ignored --nocapture`
    #[cfg(feature = "behaviors")]
    #[test]
    #[ignore]
    fn bench_tier2_weft_vs_wasmi() {
        use crate::behavior::{Actor, Behavior as _, InteractEvent};
        use crate::behavior_wasm::WasmBehavior;

        let n = 200_000u32;

        // -- Weft side: static notify (state {}, event {}) → one action.
        let notify_def = Def {
            params: vec![Ty::Record(BTreeMap::new()), Ty::Record(BTreeMap::new())],
            ret: Ty::Record(
                [
                    ("actions".to_string(), Ty::List(Box::new(Ty::Action))),
                    ("state".to_string(), Ty::Record(BTreeMap::new())),
                ]
                .into(),
            ),
            effects: BTreeSet::from([EffectKind::Notify]),
            body: Term::Rec(
                [
                    (
                        "actions".to_string(),
                        Term::ListNew(vec![Term::Effect(
                            EffectKind::Notify,
                            [("text".to_string(), Term::Text("The waystone hums.".into()))].into(),
                        )]),
                    ),
                    ("state".to_string(), Term::Rec(BTreeMap::new())),
                ]
                .into(),
            ),
            pre: None,
            post: None,
        };
        let module = Module::build(vec![notify_def], 0).unwrap();
        let mut weft_b = WeftBehavior::from_json(&serde_json::to_string(&module).unwrap()).unwrap();

        let t0 = std::time::Instant::now();
        let mut weft_actions = 0usize;
        for _ in 0..n {
            weft_actions += weft_b.on_interact("waystone", None).len();
        }
        let weft_secs = t0.elapsed().as_secs_f64();

        // -- wasmi side: the static-reply WAT module the navigator tests use —
        // returns the same one-notify ActionList through the real ABI.
        let reply = r#"{"actions":[{"action":"notify","text":"The waystone hums."}]}"#;
        // Fixed scratch allocator: one allocation per event (the host's event
        // buffer) at a constant offset — a bump allocator would exhaust the
        // single page after ~1k events and rig the benchmark.
        let wat = format!(
            r#"(module
              (memory (export "memory") 1)
              (data (i32.const 0) "{data}")
              (func (export "thread_alloc") (param i32) (result i32)
                i32.const 4096)
              (func (export "thread_on_interact") (param i32 i32) (result i64)
                i64.const {len}))"#,
            data = reply.replace('"', "\\\""),
            len = reply.len(),
        );
        let wasm = wat::parse_str(&wat).unwrap();
        let mut wasm_b = WasmBehavior::load(&wasm).unwrap();
        let event = InteractEvent {
            placement: "waystone".into(),
            actor: Actor { passport_sub: None },
            world: "meadow".into(),
            data: serde_json::Value::Null,
        };
        let t0 = std::time::Instant::now();
        let mut wasm_actions = 0usize;
        for _ in 0..n {
            wasm_actions += wasm_b.on_interact(&event).len();
        }
        let wasm_secs = t0.elapsed().as_secs_f64();

        assert_eq!(weft_actions, n as usize);
        assert_eq!(wasm_actions, n as usize);
        println!(
            "BENCH tier2 matched notify · weft {:.0} ev/s · wasmi {:.0} ev/s · ratio {:.2}x",
            n as f64 / weft_secs,
            n as f64 / wasm_secs,
            wasm_secs / weft_secs,
        );
    }

    #[test]
    fn a_tampered_module_is_refused_at_the_door() {
        // Body constructs Notify, but the declared row is empty — the exact
        // module a malicious host might serve. Verification refuses it before
        // a single step evaluates.
        let bad = {
            let mut m: Module = serde_json::from_str(&greeter_json()).unwrap();
            let mut defs: Vec<Def> = m.defs.values().cloned().collect();
            defs[0].effects.clear();
            m = Module::build(defs, 0).unwrap();
            serde_json::to_string(&m).unwrap()
        };
        assert!(WeftBehavior::from_json(&bad).is_err());
    }
}
