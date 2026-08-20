//! Tier 3 — the decade benchmark: **scaling with cores**.
//!
//! 10 000 independent behavior events through one verified module — the
//! serial reference interpreter vs. the same interpreter fanned across a
//! rayon pool at 1/2/4/8/16 threads. Weft's purity is what makes this both
//! trivially correct and bit-exact: eval shares an immutable module and owns
//! its arguments, so parallel results are asserted EQUAL to serial, always.
//! The curve — not the absolute — is the deliverable; a JS main loop cannot
//! produce this curve at any price.
//!
//! ```sh
//! cargo run -p weft --release --example bench_tier3
//! ```

use std::collections::BTreeSet;
use std::time::Instant;

use weft_lang::{Def, Module, PrimOp, Term, Ty, Value};

fn rec_ty(fields: Vec<(&str, Ty)>) -> Ty {
    Ty::Record(
        fields
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    )
}

/// A representative behavior body: fold fixed-point work over a bounded list
/// seeded from the event — the shape of a docent thinking, not a microloop.
fn workload_module() -> Module {
    let event_ty = rec_ty(vec![("seed", Ty::Int)]);
    // fold[64] over iota-like list built from seed via Map on a unit list is
    // clunky pre-Iota; the fold itself carries the work.
    let list = Term::Map {
        cap: 64,
        list: Box::new(Term::ListNew((0..64).map(Term::Int).collect())),
        body: Box::new(Term::Prim(
            PrimOp::Mul,
            vec![
                Term::Var(0),
                Term::Get(Box::new(Term::Var(1)), "seed".into()),
            ],
        )),
    };
    let body = Term::Fold {
        cap: 64,
        list: Box::new(list),
        init: Box::new(Term::Fix(0)),
        body: Box::new(Term::Prim(
            PrimOp::FAdd,
            vec![
                Term::Var(1),
                Term::Prim(
                    PrimOp::FMul,
                    vec![
                        Term::Prim(PrimOp::FixOfInt, vec![Term::Var(0)]),
                        Term::Fix(333),
                    ],
                ),
            ],
        )),
    };
    let def = Def {
        params: vec![event_ty],
        ret: Ty::Fix,
        effects: BTreeSet::new(),
        body,
        pre: None,
        post: None,
    };
    Module::build(vec![def], 0).expect("workload builds")
}

fn event(seed: i64) -> Value {
    Value::Rec([("seed".to_string(), Value::Int(seed))].into())
}

fn main() {
    const EVENTS: usize = 10_000;
    let module = workload_module();
    let certs = weft_lang::verify_module(&module).expect("verifies");
    let fuel = certs[&module.entry].fuel_bound + 64;

    // Serial reference.
    let t0 = Instant::now();
    let serial: Vec<Value> = (0..EVENTS)
        .map(|i| {
            weft_lang::eval_call(&module, module.entry, vec![event(i as i64)], fuel)
                .expect("total")
                .value
        })
        .collect();
    let serial_ms = t0.elapsed().as_secs_f64() * 1000.0;
    println!("serial          {serial_ms:8.2} ms   1.00×");

    for threads in [1usize, 2, 4, 8, 16] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        let t0 = Instant::now();
        let parallel: Vec<Value> = pool.install(|| {
            use rayon::prelude::*;
            (0..EVENTS)
                .into_par_iter()
                .map(|i| {
                    weft_lang::eval_call(&module, module.entry, vec![event(i as i64)], fuel)
                        .expect("total")
                        .value
                })
                .collect()
        });
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        // The §8 claim, enforced: parallel results are BIT-EXACT vs serial.
        assert_eq!(parallel, serial, "determinism broke at {threads} threads");
        println!(
            "{threads:2} thread(s)     {ms:8.2} ms   {:.2}×  (bit-exact ✓)",
            serial_ms / ms
        );
    }
}
