//! Generates **weft-form** — the first published Weft package, and the seed
//! of the Thread's three.js: the vocabulary for *making shapes of things*.
//! Rings, grids, interpolation — pure verified geometry over `Fix`, exported
//! as petnames over content hashes. Anyone can build on it; nobody can break
//! it (the hashes can't move).
//!
//! ```sh
//! cargo run -p weft --example gen_weft_form
//! ```

use std::collections::BTreeSet;

use weft_lang::pack::Package;
use weft_lang::{Def, PrimOp, Term, Ty, FIX_TAU};

fn rec_ty(fields: Vec<(&str, Ty)>) -> Ty {
    Ty::Record(
        fields
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    )
}
fn vec3_ty() -> Ty {
    rec_ty(vec![("x", Ty::Fix), ("y", Ty::Fix), ("z", Ty::Fix)])
}
fn p(op: PrimOp, args: Vec<Term>) -> Term {
    Term::Prim(op, args)
}

fn main() {
    use PrimOp::*;

    // lerp(a, b, t) = a + (b − a)·t          [a=Var2, b=Var1, t=Var0]
    let lerp = Def {
        params: vec![Ty::Fix, Ty::Fix, Ty::Fix],
        ret: Ty::Fix,
        effects: BTreeSet::new(),
        body: p(
            FAdd,
            vec![
                Term::Var(2),
                p(
                    FMul,
                    vec![p(FSub, vec![Term::Var(1), Term::Var(2)]), Term::Var(0)],
                ),
            ],
        ),
        pre: None,
        post: None,
    };

    // ring_point(i, n, radius) → {x: r·cos θ, y: 0, z: r·sin θ}, θ = τ·i/n
    // [i=Var2, n=Var1, radius=Var0]
    let theta = p(
        FMul,
        vec![
            p(
                FDiv,
                vec![
                    p(FixOfInt, vec![Term::Var(2)]),
                    p(FixOfInt, vec![Term::Var(1)]),
                ],
            ),
            Term::Fix(FIX_TAU),
        ],
    );
    let ring_point = Def {
        params: vec![Ty::Int, Ty::Int, Ty::Fix],
        ret: vec3_ty(),
        effects: BTreeSet::new(),
        body: Term::Let(
            Box::new(theta), // θ = Var 0 below; i/n/radius shift to 3/2/1
            Box::new(Term::Rec(
                [
                    (
                        "x".to_string(),
                        p(FMul, vec![Term::Var(1), p(FCos, vec![Term::Var(0)])]),
                    ),
                    ("y".to_string(), Term::Fix(0)),
                    (
                        "z".to_string(),
                        p(FMul, vec![Term::Var(1), p(FSin, vec![Term::Var(0)])]),
                    ),
                ]
                .into(),
            )),
        ),
        pre: None,
        post: None,
    };
    let ring_point_hash = weft_lang::hash_def(&ring_point);

    // ring(n, radius) → map[256] (iota[256] n) (i → ring_point(i, n, radius))
    // [n=Var1, radius=Var0]; inside the map body: i=Var0, radius=Var1, n=Var2.
    let ring = Def {
        params: vec![Ty::Int, Ty::Fix],
        ret: Ty::List(Box::new(vec3_ty())),
        effects: BTreeSet::new(),
        body: Term::Map {
            cap: 256,
            list: Box::new(Term::Iota {
                cap: 256,
                count: Box::new(Term::Var(1)),
            }),
            body: Box::new(Term::Call(
                ring_point_hash,
                vec![Term::Var(0), Term::Var(2), Term::Var(1)],
            )),
        },
        pre: None,
        post: None,
    };

    // grid_point(i, cols, spacing) → {x: col·s, y: 0, z: row·s}
    // [i=Var2, cols=Var1, spacing=Var0]
    let grid_point = Def {
        params: vec![Ty::Int, Ty::Int, Ty::Fix],
        ret: vec3_ty(),
        effects: BTreeSet::new(),
        body: Term::Let(
            Box::new(p(Div, vec![Term::Var(2), Term::Var(1)])), // row = Var0; shift i/cols/s → 3/2/1
            Box::new(Term::Rec(
                [
                    (
                        "x".to_string(),
                        p(
                            FMul,
                            vec![
                                Term::Var(1),
                                p(
                                    FixOfInt,
                                    vec![p(
                                        Sub,
                                        vec![
                                            Term::Var(3),
                                            p(Mul, vec![Term::Var(0), Term::Var(2)]),
                                        ],
                                    )],
                                ),
                            ],
                        ),
                    ),
                    ("y".to_string(), Term::Fix(0)),
                    (
                        "z".to_string(),
                        p(FMul, vec![Term::Var(1), p(FixOfInt, vec![Term::Var(0)])]),
                    ),
                ]
                .into(),
            )),
        ),
        pre: None,
        post: None,
    };
    let grid_point_hash = weft_lang::hash_def(&grid_point);

    // grid(count, cols, spacing) — [count=Var2, cols=Var1, spacing=Var0]
    let grid = Def {
        params: vec![Ty::Int, Ty::Int, Ty::Fix],
        ret: Ty::List(Box::new(vec3_ty())),
        effects: BTreeSet::new(),
        body: Term::Map {
            cap: 1024,
            list: Box::new(Term::Iota {
                cap: 1024,
                count: Box::new(Term::Var(2)),
            }),
            body: Box::new(Term::Call(
                grid_point_hash,
                vec![Term::Var(0), Term::Var(2), Term::Var(1)],
            )),
        },
        pre: None,
        post: None,
    };

    let pkg = Package::build(
        "weft-form",
        vec![lerp, ring_point, ring, grid_point, grid],
        vec![
            ("lerp", 0),
            ("ring-point", 1),
            ("ring", 2),
            ("grid-point", 3),
            ("grid", 4),
        ],
    )
    .expect("weft-form builds + verifies");

    // Functional proof before publishing: ring(4, 1.0) must trace the unit
    // circle — cos/sin at 0, π/2, π, 3π/2 within Bhaskara's ~0.2% envelope.
    {
        let ring_hash = pkg.export("ring").unwrap();
        let m = weft_lang::pack::link(&[pkg.clone()], vec![], usize::MAX)
            .err()
            .map(|_| ())
            .map(|_| weft_lang::Module {
                defs: pkg.defs.clone(),
                entry: ring_hash,
            })
            .unwrap_or(weft_lang::Module {
                defs: pkg.defs.clone(),
                entry: ring_hash,
            });
        let out = weft_lang::eval_call(
            &m,
            ring_hash,
            vec![weft_lang::Value::Int(4), weft_lang::Value::Fix(weft_lang::FIX_SCALE)],
            1_000_000,
        )
        .expect("ring evaluates");
        let weft_lang::Value::List(pts) = out.value else {
            panic!("ring returns a list")
        };
        assert_eq!(pts.len(), 4);
        let coord = |p: &weft_lang::Value, k: &str| -> i64 {
            let weft_lang::Value::Rec(r) = p else { panic!() };
            let weft_lang::Value::Fix(v) = r[k] else { panic!() };
            v
        };
        let close = |a: i64, b: i64| (a - b).abs() < 4_000; // 0.4% of unit
        assert!(close(coord(&pts[0], "x"), weft_lang::FIX_SCALE) && close(coord(&pts[0], "z"), 0));
        assert!(close(coord(&pts[1], "x"), 0) && close(coord(&pts[1], "z"), weft_lang::FIX_SCALE));
        assert!(close(coord(&pts[2], "x"), -weft_lang::FIX_SCALE) && close(coord(&pts[2], "z"), 0));
        assert!(close(coord(&pts[3], "x"), 0) && close(coord(&pts[3], "z"), -weft_lang::FIX_SCALE));
        println!("ring(4, 1.0) traces the unit circle ✓ (Bhaskara within 0.4%)");
    }

    std::fs::create_dir_all("packages/weft-form").expect("mkdir");
    let out = "packages/weft-form/weft-form.weftpack.json";
    std::fs::write(out, serde_json::to_string_pretty(&pkg).unwrap()).expect("write");
    println!(
        "wrote {out} — {} defs, exports: {}",
        pkg.defs.len(),
        pkg.exports.keys().cloned().collect::<Vec<_>>().join(", ")
    );
}
