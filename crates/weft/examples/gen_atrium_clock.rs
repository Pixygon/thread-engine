//! Generates the Atrium's living clock — a ticking Weft module that rewrites
//! the "alive for N seconds" plaque through the scene ABI (four beats to the
//! second) and, every ten seconds, **conjures a mote of light** into the hall
//! with the `spawn` effect: pseudo-random positions derived from the beat
//! count, pure and deterministic like everything woven. Run from the repo
//! root to (re)emit the committed module:
//!
//! ```sh
//! cargo run -p weft --example gen_atrium_clock
//! ```

use std::collections::BTreeSet;

use weft_lang::pack::Package;
use weft_lang::{Def, EffectKind, PrimOp, Term, Ty};

fn rec_ty(fields: Vec<(&str, Ty)>) -> Ty {
    Ty::Record(
        fields
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    )
}

fn int(i: i64) -> Term {
    Term::Int(i)
}
fn add(a: Term, b: Term) -> Term {
    Term::Prim(PrimOp::Add, vec![a, b])
}
fn sub(a: Term, b: Term) -> Term {
    Term::Prim(PrimOp::Sub, vec![a, b])
}
fn mul(a: Term, b: Term) -> Term {
    Term::Prim(PrimOp::Mul, vec![a, b])
}
fn div(a: Term, b: Term) -> Term {
    Term::Prim(PrimOp::Div, vec![a, b])
}
/// `x mod m` (for x ≥ 0), spelled with total ops: `x − (x/m)·m`.
fn modt(x: Term, m: i64) -> Term {
    sub(x.clone(), mul(div(x, int(m)), int(m)))
}

fn main() {
    let state_ty = rec_ty(vec![("beats", Ty::Int)]);
    let out_ty = rec_ty(vec![
        ("actions", Ty::List(Box::new(Ty::Action))),
        ("state", state_ty.clone()),
    ]);

    // Var(0) = beats' (the Let binder), Var(1) = state, Var(2) = event.
    let beats = || Term::Var(0);

    let clock_action = Term::Effect(
        EffectKind::SetState,
        [
            ("placement".to_string(), Term::Text("clock".into())),
            (
                "text".to_string(),
                Term::Prim(
                    PrimOp::Concat,
                    vec![
                        Term::Prim(
                            PrimOp::Concat,
                            vec![
                                Term::Text("The Atrium has been alive for ".into()),
                                Term::Prim(PrimOp::ToText, vec![div(beats(), int(4))]),
                            ],
                        ),
                        Term::Text(" seconds.\nWoven code, beating.".into()),
                    ],
                ),
            ),
        ]
        .into(),
    );

    // A mote: position + hue derived from the beat count — deterministic
    // "randomness", the only kind a total language owns. Positions are **Fix
    // metres** (exact fixed-point, weft-v0.1.1) and each mote bobs gently.
    let cm_to_m = |t: Term| {
        Term::Prim(
            PrimOp::FDiv,
            vec![
                Term::Prim(PrimOp::FixOfInt, vec![t]),
                Term::Prim(PrimOp::FixOfInt, vec![int(100)]),
            ],
        )
    };
    let mote_action = Term::Effect(
        EffectKind::Spawn,
        [
            ("builtin".to_string(), Term::Text("sphere".into())),
            (
                "x".to_string(),
                cm_to_m(sub(modt(mul(beats(), int(137)), 1800), int(900))),
            ),
            (
                "y".to_string(),
                cm_to_m(add(int(220), modt(mul(beats(), int(71)), 320))),
            ),
            (
                "z".to_string(),
                cm_to_m(sub(modt(mul(beats(), int(53)), 1800), int(900))),
            ),
            ("scale".to_string(), Term::Fix(140_000)), // 0.14 m
            (
                "r".to_string(),
                add(int(110), modt(mul(beats(), int(97)), 130)),
            ),
            (
                "g".to_string(),
                add(int(150), modt(mul(beats(), int(61)), 100)),
            ),
            ("b".to_string(), int(255)),
            ("animate".to_string(), Term::Text("bob".into())),
            ("speed".to_string(), Term::Fix(700_000)), // 0.7
            ("amp".to_string(), Term::Fix(180_000)),   // 0.18 m
        ]
        .into(),
    );

    // Every 200 beats (50 s): a RING of twelve motes around the beacon —
    // geometry from the **weft-form package**, consumed by hash. The first
    // Weft code built on a Weft package.
    let form: Package = serde_json::from_str(
        &std::fs::read_to_string("packages/weft-form/weft-form.weftpack.json")
            .expect("run gen_weft_form first"),
    )
    .expect("weft-form parses");
    form.verify().expect("weft-form verifies");
    let ring = form.export("ring").expect("weft-form exports ring");
    let ring_actions = Term::Map {
        cap: 16,
        list: Box::new(Term::Call(ring, vec![int(12), Term::Fix(6_000_000)])), // 12 points, r=6 m
        body: Box::new(Term::Effect(
            EffectKind::Spawn,
            [
                ("builtin".to_string(), Term::Text("sphere".into())),
                (
                    "x".to_string(),
                    Term::Get(Box::new(Term::Var(0)), "x".into()),
                ),
                (
                    "y".to_string(),
                    Term::Prim(
                        PrimOp::FAdd,
                        vec![
                            Term::Get(Box::new(Term::Var(0)), "y".into()),
                            Term::Fix(2_600_000),
                        ],
                    ),
                ),
                (
                    "z".to_string(),
                    Term::Get(Box::new(Term::Var(0)), "z".into()),
                ),
                ("scale".to_string(), Term::Fix(120_000)),
                ("r".to_string(), int(255)),
                ("g".to_string(), int(210)),
                ("b".to_string(), int(120)),
                ("animate".to_string(), Term::Text("bob".into())),
                ("speed".to_string(), Term::Fix(500_000)),
                ("amp".to_string(), Term::Fix(120_000)),
            ]
            .into(),
        )),
    };

    let body = Term::Let(
        Box::new(add(
            Term::Get(Box::new(Term::Var(1)), "beats".into()),
            int(1),
        )),
        Box::new(Term::Rec(
            [
                (
                    "actions".to_string(),
                    Term::If(
                        Box::new(Term::Prim(PrimOp::EqInt, vec![modt(beats(), 200), int(0)])),
                        Box::new(ring_actions),
                        Box::new(Term::If(
                            Box::new(Term::Prim(PrimOp::EqInt, vec![modt(beats(), 40), int(0)])),
                            Box::new(Term::ListNew(vec![clock_action.clone(), mote_action])),
                            Box::new(Term::ListNew(vec![clock_action])),
                        )),
                    ),
                ),
                (
                    "state".to_string(),
                    Term::Rec([("beats".to_string(), beats())].into()),
                ),
            ]
            .into(),
        )),
    );

    let def = Def {
        params: vec![state_ty, rec_ty(vec![])],
        ret: out_ty,
        effects: BTreeSet::from([EffectKind::SetState, EffectKind::Spawn]),
        body,
        pre: None,
        post: None,
    };
    // Link the behavior with weft-form — a set union of hashes — then publish
    // the result AS A PACKAGE: `weft-clock`, exporting "clock". The Atrium's
    // markup binds it with one attribute: weft-use="…#clock". The clock is
    // both a consumer of the ecosystem (it calls weft-form's ring by hash)
    // and a citizen of it (anyone can weft-use the clock).
    let module = weft_lang::pack::link(&[form], vec![def], 0).expect("clock links");
    weft_lang::verify_module(&module).expect("the clock must verify — nothing unverified ships");
    let pkg = weft_lang::pack::Package {
        name: "weft-clock".to_string(),
        exports: [("clock".to_string(), module.entry)].into(),
        defs: module.defs,
    };
    pkg.verify().expect("the clock package verifies");
    let json = serde_json::to_string_pretty(&pkg).unwrap();
    let out = "worlds/wiki.pixygon.io/weft-clock.weftpack.json";
    std::fs::write(out, &json).expect("write package");
    let _ = std::fs::remove_file("worlds/wiki.pixygon.io/clock.weft.json");
    println!(
        "wrote {out} ({} bytes, verified) — exports: clock",
        json.len()
    );
}
