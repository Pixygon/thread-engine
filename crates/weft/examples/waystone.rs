//! Generate the Meadow waystone's behavior — the first Weft module shipped in
//! a real world. Emits the serialized module to stdout:
//!
//! ```sh
//! cargo run -p weft --example waystone > worlds/meadow/waystone.weft.json
//! ```
//!
//! The behavior: count touches in state; each interact notifies with the
//! running count. Its whole capability is `notify` — the effect row IS the
//! permission, so this module *cannot* do anything else, on any host.

use std::collections::BTreeSet;

use weft_lang::{Def, EffectKind, Module, PrimOp, Term, Ty};

fn rec_ty(fields: Vec<(&str, Ty)>) -> Ty {
    Ty::Record(
        fields
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    )
}

fn main() {
    let state_ty = rec_ty(vec![("touches", Ty::Int)]);
    let event_ty = rec_ty(vec![]);
    let out_ty = rec_ty(vec![
        ("actions", Ty::List(Box::new(Ty::Action))),
        ("state", state_ty.clone()),
    ]);

    // params: [state = Var 1, event = Var 0]
    // let a(=Var0 after let) = state.touches + 1 in
    //   { actions: [notify "The waystone hums — <a> travelers have touched it."],
    //     state: {touches: a} }
    let body = Term::Let(
        Box::new(Term::Prim(
            PrimOp::Add,
            vec![
                Term::Get(Box::new(Term::Var(1)), "touches".into()),
                Term::Int(1),
            ],
        )),
        Box::new(Term::Rec(
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
                                    Term::Prim(
                                        PrimOp::Concat,
                                        vec![
                                            Term::Text("The waystone hums — ".into()),
                                            Term::Prim(PrimOp::ToText, vec![Term::Var(0)]),
                                        ],
                                    ),
                                    Term::Text(" travelers have touched it.".into()),
                                ],
                            ),
                        )]
                        .into(),
                    )]),
                ),
                (
                    "state".to_string(),
                    Term::Rec([("touches".to_string(), Term::Var(0))].into()),
                ),
            ]
            .into(),
        )),
    );

    let def = Def {
        params: vec![state_ty, event_ty],
        ret: out_ty,
        effects: BTreeSet::from([EffectKind::Notify]),
        body,
        pre: None,
        post: None,
    };
    let module = Module::build(vec![def], 0).expect("builds");
    weft_lang::verify_module(&module).expect("the shipped waystone must verify");
    eprintln!(
        "-- projection (audit view) --\n{}",
        weft_lang::project::module(&module)
    );
    println!("{}", serde_json::to_string_pretty(&module).unwrap());
}
