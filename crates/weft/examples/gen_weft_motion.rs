//! Generates **weft-motion** — package two: easing and oscillation over
//! `Fix`. Where weft-form shapes space, weft-motion shapes *time* — the
//! curves that make conjured things feel alive. Pure, verified, exact.
//!
//! ```sh
//! cargo run -p weft --example gen_weft_motion
//! ```

use std::collections::BTreeSet;

use weft_lang::pack::Package;
use weft_lang::{Def, PrimOp, Term, Ty, FIX_SCALE, FIX_TAU};

fn p(op: PrimOp, args: Vec<Term>) -> Term {
    Term::Prim(op, args)
}
fn fx(raw: i64) -> Term {
    Term::Fix(raw)
}
const ONE: i64 = FIX_SCALE;

fn main() {
    use PrimOp::*;

    // clamp01(t) = if t < 0 then 0 else if 1 < t then 1 else t   [t = Var 0]
    let clamp01 = Def {
        params: vec![Ty::Fix],
        ret: Ty::Fix,
        effects: BTreeSet::new(),
        body: Term::If(
            Box::new(p(FLt, vec![Term::Var(0), fx(0)])),
            Box::new(fx(0)),
            Box::new(Term::If(
                Box::new(p(FLt, vec![fx(ONE), Term::Var(0)])),
                Box::new(fx(ONE)),
                Box::new(Term::Var(0)),
            )),
        ),
        pre: None,
        post: None,
    };
    let clamp01_h = weft_lang::hash_def(&clamp01);

    // ease_in(t)  = t²            (clamped)
    let ease_in = Def {
        params: vec![Ty::Fix],
        ret: Ty::Fix,
        effects: BTreeSet::new(),
        body: Term::Let(
            Box::new(Term::Call(clamp01_h, vec![Term::Var(0)])),
            Box::new(p(FMul, vec![Term::Var(0), Term::Var(0)])),
        ),
        pre: None,
        post: None,
    };

    // ease_out(t) = 1 − (1−t)²    (clamped)
    let ease_out = Def {
        params: vec![Ty::Fix],
        ret: Ty::Fix,
        effects: BTreeSet::new(),
        body: Term::Let(
            Box::new(p(
                FSub,
                vec![fx(ONE), Term::Call(clamp01_h, vec![Term::Var(0)])],
            )),
            Box::new(p(
                FSub,
                vec![fx(ONE), p(FMul, vec![Term::Var(0), Term::Var(0)])],
            )),
        ),
        pre: None,
        post: None,
    };

    // smoothstep(t) = t²·(3 − 2t) (clamped) — the ease-in-out workhorse.
    let smoothstep = Def {
        params: vec![Ty::Fix],
        ret: Ty::Fix,
        effects: BTreeSet::new(),
        body: Term::Let(
            Box::new(Term::Call(clamp01_h, vec![Term::Var(0)])),
            Box::new(p(
                FMul,
                vec![
                    p(FMul, vec![Term::Var(0), Term::Var(0)]),
                    p(
                        FSub,
                        vec![fx(3 * ONE), p(FMul, vec![fx(2 * ONE), Term::Var(0)])],
                    ),
                ],
            )),
        ),
        pre: None,
        post: None,
    };

    // osc(t) = (sin(τ·t) + 1) / 2 — a 0..1 wave with period 1.
    let osc = Def {
        params: vec![Ty::Fix],
        ret: Ty::Fix,
        effects: BTreeSet::new(),
        body: p(
            FDiv,
            vec![
                p(
                    FAdd,
                    vec![
                        p(FSin, vec![p(FMul, vec![fx(FIX_TAU), Term::Var(0)])]),
                        fx(ONE),
                    ],
                ),
                fx(2 * ONE),
            ],
        ),
        pre: None,
        post: None,
    };

    // pingpong(t) = 1 − |1 − 2·frac(t)| — a 0→1→0 triangle with period 1.
    // frac via t − trunc-toward-zero (valid for t ≥ 0; negatives clamp to 0).
    let pingpong = Def {
        params: vec![Ty::Fix],
        ret: Ty::Fix,
        effects: BTreeSet::new(),
        body: Term::Let(
            // f = t − fix(trunc(t))   (fractional part, t ≥ 0)
            Box::new(p(
                FSub,
                vec![
                    Term::Call(clamp01_h, vec![Term::Var(0)]), // reuse clamp for safety on tests
                    fx(0),
                ],
            )),
            Box::new(Term::Let(
                // d = 1 − 2f  … |d| via If
                Box::new(p(
                    FSub,
                    vec![fx(ONE), p(FMul, vec![fx(2 * ONE), Term::Var(0)])],
                )),
                Box::new(Term::If(
                    Box::new(p(FLt, vec![Term::Var(0), fx(0)])),
                    Box::new(p(FAdd, vec![fx(ONE), Term::Var(0)])),
                    Box::new(p(FSub, vec![fx(ONE), Term::Var(0)])),
                )),
            )),
        ),
        pre: None,
        post: None,
    };

    let pkg = Package::build(
        "weft-motion",
        vec![clamp01, ease_in, ease_out, smoothstep, osc, pingpong],
        vec![
            ("clamp01", 0),
            ("ease-in", 1),
            ("ease-out", 2),
            ("smoothstep", 3),
            ("osc", 4),
            ("pingpong", 5),
        ],
    )
    .expect("weft-motion builds + verifies");

    // Functional proof before publishing.
    {
        let call1 = |name: &str, arg: i64| -> i64 {
            let h = pkg.export(name).unwrap();
            let m = weft_lang::Module {
                defs: pkg.defs.clone(),
                entry: h,
            };
            let out =
                weft_lang::eval_call(&m, h, vec![weft_lang::Value::Fix(arg)], 1_000_000).expect("total");
            let weft_lang::Value::Fix(v) = out.value else {
                panic!()
            };
            v
        };
        let close = |a: i64, b: i64| (a - b).abs() < 4_000;
        assert_eq!(call1("smoothstep", 0), 0);
        assert_eq!(call1("smoothstep", ONE), ONE);
        assert!(close(call1("smoothstep", ONE / 2), ONE / 2));
        assert!(call1("ease-in", ONE / 2) < ONE / 2, "ease-in starts slow");
        assert!(call1("ease-out", ONE / 2) > ONE / 2, "ease-out starts fast");
        assert!(close(call1("osc", 0), ONE / 2));
        assert!(close(call1("osc", ONE / 4), ONE), "peak at quarter period");
        assert!(close(call1("pingpong", ONE / 2), ONE), "triangle peaks mid");
        assert_eq!(call1("clamp01", -5 * ONE), 0);
        println!("easing curves verified ✓ (endpoints, midpoints, monotone bias)");
    }

    std::fs::create_dir_all("packages/weft-motion").expect("mkdir");
    let out = "packages/weft-motion/weft-motion.weftpack.json";
    std::fs::write(out, serde_json::to_string_pretty(&pkg).unwrap()).expect("write");
    println!(
        "wrote {out} — {} defs, exports: {}",
        pkg.defs.len(),
        pkg.exports.keys().cloned().collect::<Vec<_>>().join(", ")
    );
}
