//! # weft-model — the Thread's modeling library
//!
//! The Weft-equivalent of a scene library, and the answer to "how does an
//! agent make a *model*?": not by pasting vertices, and not by hand-placing
//! primitives — by **calling functions that compute geometry**.
//!
//! ```text
//! stairs(12, 0.18, 0.28, 1.2)          twelve treads, computed by a fold
//! column(5.2, 0.44)                    a turned profile, from two numbers
//! at(ring_of(baluster, 12, 2.0), 0,1,0)   twelve of a thing, around a circle
//! model1("balustrade", …, marble())    …wearing a full PBR material
//! ```
//!
//! Everything here returns plain Weft values — a flat list of carving steps
//! (`Node`) and the materials its parts wear — which
//! [`chisel`](../../chisel) meshes and bakes. That indirection is the point:
//! the model is a **verified, content-addressed, deterministic program**, so
//! it can be parameterised, cached by hash, exchanged between agents, and
//! replayed byte-identically on any machine. Three.js gives an agent a scene
//! API and hopes; this gives it a *proof*.
//!
//! **PBR is not an afterthought.** Every material constructor returns the
//! full set — base color ramp, normal height, metallic and roughness bands,
//! ambient occlusion — because a model that arrives without them is a model
//! someone still has to finish.

use std::collections::{BTreeMap, BTreeSet};

use crate::pack::Package;
use crate::{hash_def, Def, PrimOp, Term, Ty, WeftHash, FIX_SCALE, FIX_TAU};

// ---------------------------------------------------------------------------
// Term-building helpers (the generator's own comfort — not part of the ABI)
// ---------------------------------------------------------------------------

fn fx(v: f32) -> Term {
    Term::Fix((v as f64 * FIX_SCALE as f64).round() as i64)
}
fn int(v: i64) -> Term {
    Term::Int(v)
}
fn txt(s: &str) -> Term {
    Term::Text(s.to_string())
}
fn var(i: u32) -> Term {
    Term::Var(i)
}
fn p2(op: PrimOp, a: Term, b: Term) -> Term {
    Term::Prim(op, vec![a, b])
}
fn p1(op: PrimOp, a: Term) -> Term {
    Term::Prim(op, vec![a])
}
fn add(a: Term, b: Term) -> Term {
    p2(PrimOp::FAdd, a, b)
}
fn sub(a: Term, b: Term) -> Term {
    p2(PrimOp::FSub, a, b)
}
fn mul(a: Term, b: Term) -> Term {
    p2(PrimOp::FMul, a, b)
}
fn div(a: Term, b: Term) -> Term {
    p2(PrimOp::FDiv, a, b)
}
fn get(t: Term, field: &str) -> Term {
    Term::Get(Box::new(t), field.to_string())
}
fn rec(fields: Vec<(&str, Term)>) -> Term {
    Term::Rec(
        fields
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    )
}
fn list(items: Vec<Term>) -> Term {
    Term::ListNew(items)
}
fn rec_ty(fields: Vec<(&str, Ty)>) -> Ty {
    Ty::Record(
        fields
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    )
}
fn fix_list_ty() -> Ty {
    Ty::List(Box::new(Ty::Fix))
}
/// A typed-but-empty `List(Fix)`: an empty `ListNew` has no element type, so
/// a zero-cap map over a one-element list is how you say "none, of Fix".
fn no_profile() -> Term {
    Term::Map {
        cap: 0,
        list: Box::new(list(vec![fx(0.0)])),
        body: Box::new(var(0)),
    }
}

/// Repetition caps. Static, because the fuel bound must be: a model with
/// more than this many steps is a scene, and scenes are the manifest's job.
const CAP: u32 = 512;
const REPEAT_CAP: u32 = 128;

// ---------------------------------------------------------------------------
// The ABI types (these must mirror `infinite_manifest::model`)
// ---------------------------------------------------------------------------

fn node_ty() -> Ty {
    rec_ty(vec![
        ("axis", Ty::Text),
        ("d", Ty::Fix),
        ("h", Ty::Fix),
        ("k", Ty::Fix),
        ("mode", Ty::Text),
        ("part", Ty::Int),
        ("prim", Ty::Text),
        ("profile", fix_list_ty()),
        ("r", Ty::Fix),
        ("r2", Ty::Fix),
        ("rot", Ty::Fix),
        ("round", Ty::Fix),
        ("w", Ty::Fix),
        ("x", Ty::Fix),
        ("y", Ty::Fix),
        ("z", Ty::Fix),
    ])
}
fn nodes_ty() -> Ty {
    Ty::List(Box::new(node_ty()))
}

/// The bakeable recipe — the *inner* one, without a layer of its own.
fn recipe_base_ty() -> Ty {
    rec_ty(vec![
        ("ao", Ty::Fix),
        ("colors", Ty::List(Box::new(fix_list_ty()))),
        ("height", Ty::Fix),
        ("kind", Ty::Text),
        ("metallic", fix_list_ty()),
        ("octaves", Ty::Int),
        ("roughness", fix_list_ty()),
        ("scale", Ty::Fix),
        ("seed", Ty::Int),
        ("size", Ty::Int),
        ("triplanar", Ty::Fix),
    ])
}
/// The outer recipe: the same, plus an optional second layer blended over it
/// (`mix: 0` means the layer is declared but unused, and costs nothing).
fn recipe_ty() -> Ty {
    let mut f = match recipe_base_ty() {
        Ty::Record(m) => m,
        _ => unreachable!(),
    };
    f.insert("over".into(), recipe_base_ty());
    f.insert("mix".into(), Ty::Fix);
    f.insert("mask_scale".into(), Ty::Fix);
    f.insert("mask_seed".into(), Ty::Int);
    Ty::Record(f)
}
fn material_ty() -> Ty {
    rec_ty(vec![
        ("color", fix_list_ty()),
        ("emissive", Ty::Fix),
        ("name", Ty::Text),
        ("resolution", Ty::Int),
        ("texture", recipe_ty()),
        ("uv", Ty::Text),
        ("uv_scale", Ty::Fix),
    ])
}
fn model_ty() -> Ty {
    rec_ty(vec![
        ("materials", Ty::List(Box::new(material_ty()))),
        ("name", Ty::Text),
        ("nodes", nodes_ty()),
    ])
}

/// Build one carving step, overriding the defaults that matter.
fn node(prim: &str, over: Vec<(&str, Term)>) -> Term {
    let mut f: BTreeMap<String, Term> = BTreeMap::new();
    f.insert("prim".into(), txt(prim));
    f.insert("mode".into(), txt("add"));
    f.insert("part".into(), int(0));
    f.insert("axis".into(), txt("y"));
    for k in ["x", "y", "z", "rot", "r2", "round"] {
        f.insert(k.into(), fx(0.0));
    }
    f.insert("r".into(), fx(0.5));
    for k in ["h", "w", "d"] {
        f.insert(k.into(), fx(1.0));
    }
    f.insert("k".into(), fx(0.25));
    f.insert("profile".into(), no_profile());
    for (k, v) in over {
        f.insert(k.into(), v);
    }
    Term::Rec(f)
}

/// Copy the node bound at `elem` (a de Bruijn index), changing some fields —
/// how every transform is written.
fn copy_node(elem: u32, over: Vec<(&str, Term)>) -> Term {
    let mut f: BTreeMap<String, Term> = BTreeMap::new();
    for k in [
        "axis", "d", "h", "k", "mode", "part", "prim", "profile", "r", "r2", "rot", "round", "w",
        "x", "y", "z",
    ] {
        f.insert(k.into(), get(var(elem), k));
    }
    for (k, v) in over {
        f.insert(k.into(), v);
    }
    Term::Rec(f)
}

fn def(params: Vec<Ty>, ret: Ty, body: Term) -> Def {
    Def {
        params,
        ret,
        effects: BTreeSet::new(),
        body,
        pre: None,
        post: None,
    }
}

/// A def under construction, so later defs can `Call` earlier ones by hash.
struct Builder {
    defs: Vec<Def>,
    by_name: BTreeMap<String, (usize, WeftHash)>,
}

impl Builder {
    fn new() -> Self {
        Builder {
            defs: Vec::new(),
            by_name: BTreeMap::new(),
        }
    }
    fn add(&mut self, name: &str, d: Def) -> WeftHash {
        let h = hash_def(&d);
        self.defs.push(d);
        self.by_name
            .insert(name.to_string(), (self.defs.len() - 1, h));
        h
    }
    fn call(&self, name: &str, args: Vec<Term>) -> Term {
        let (_, h) = self.by_name[name];
        Term::Call(h, args)
    }
    fn idx(&self, name: &str) -> usize {
        self.by_name[name].0
    }
}

// ---------------------------------------------------------------------------
// The library
// ---------------------------------------------------------------------------

/// Build the `weft-model` package: constructors, transforms, repeaters,
/// parametric parts, and PBR materials.
pub fn package() -> Package {
    let mut b = Builder::new();

    // --- primitives: each returns a one-step list, ready to compose --------
    b.add(
        "sphere",
        def(
            vec![Ty::Fix],
            nodes_ty(),
            list(vec![node("sphere", vec![("r", var(0))])]),
        ),
    );
    b.add(
        "cube",
        def(
            vec![Ty::Fix, Ty::Fix, Ty::Fix],
            nodes_ty(),
            list(vec![node(
                "box",
                vec![("w", var(2)), ("h", var(1)), ("d", var(0))],
            )]),
        ),
    );
    b.add(
        "cylinder",
        def(
            vec![Ty::Fix, Ty::Fix],
            nodes_ty(),
            list(vec![node("cylinder", vec![("r", var(1)), ("h", var(0))])]),
        ),
    );
    b.add(
        "capsule",
        def(
            vec![Ty::Fix, Ty::Fix],
            nodes_ty(),
            list(vec![node("capsule", vec![("r", var(1)), ("h", var(0))])]),
        ),
    );
    b.add(
        "cone",
        def(
            vec![Ty::Fix, Ty::Fix, Ty::Fix],
            nodes_ty(),
            list(vec![node(
                "cone",
                vec![("r", var(2)), ("r2", var(1)), ("h", var(0))],
            )]),
        ),
    );
    b.add(
        "torus",
        def(
            vec![Ty::Fix, Ty::Fix],
            nodes_ty(),
            list(vec![node("torus", vec![("r", var(1)), ("r2", var(0))])]),
        ),
    );
    b.add(
        "lathe",
        def(
            vec![fix_list_ty()],
            nodes_ty(),
            list(vec![node("lathe", vec![("profile", var(0))])]),
        ),
    );

    // --- transforms: map over the steps ------------------------------------
    // at(nodes, dx, dy, dz): inside the map body every outer var shifts by 1.
    b.add(
        "at",
        def(
            vec![nodes_ty(), Ty::Fix, Ty::Fix, Ty::Fix],
            nodes_ty(),
            Term::Map {
                cap: CAP,
                list: Box::new(var(3)),
                body: Box::new(copy_node(
                    0,
                    vec![
                        ("x", add(get(var(0), "x"), var(3))),
                        ("y", add(get(var(0), "y"), var(2))),
                        ("z", add(get(var(0), "z"), var(1))),
                    ],
                )),
            },
        ),
    );
    // spin(nodes, deg): turn the whole group about Y — positions AND the
    // steps' own rotation, so a spun part stays itself.
    // Scope inside the map body: [element=0, deg=1, nodes=2]; each `let`
    // pushes one more binder, so the angle is re-indexed per depth. (Writing
    // the scope out beats guessing: de Bruijn mistakes type-check as the
    // wrong thing, not as an error.)
    b.add(
        "spin",
        def(
            vec![nodes_ty(), Ty::Fix],
            nodes_ty(),
            Term::Map {
                cap: CAP,
                list: Box::new(var(1)),
                body: Box::new(Term::Let(
                    // c = cos θ, computed at [element=0, deg=1, nodes=2]
                    Box::new(p1(
                        PrimOp::FCos,
                        mul(div(var(1), fx(360.0)), Term::Fix(FIX_TAU)),
                    )),
                    Box::new(Term::Let(
                        // s = sin θ, computed at [c=0, element=1, deg=2, nodes=3]
                        Box::new(p1(
                            PrimOp::FSin,
                            mul(div(var(2), fx(360.0)), Term::Fix(FIX_TAU)),
                        )),
                        // body at [s=0, c=1, element=2, deg=3, nodes=4]
                        Box::new(copy_node(
                            2,
                            vec![
                                (
                                    "x",
                                    add(
                                        mul(get(var(2), "x"), var(1)),
                                        mul(get(var(2), "z"), var(0)),
                                    ),
                                ),
                                (
                                    "z",
                                    sub(
                                        mul(get(var(2), "z"), var(1)),
                                        mul(get(var(2), "x"), var(0)),
                                    ),
                                ),
                                ("rot", add(get(var(2), "rot"), var(3))),
                            ],
                        )),
                    )),
                )),
            },
        ),
    );
    // mode(nodes, how, k): how these steps combine — "add" | "blend" | "cut"
    // | "intersect". Carving is subtraction; this is the verb for it.
    b.add(
        "mode",
        def(
            vec![nodes_ty(), Ty::Text, Ty::Fix],
            nodes_ty(),
            Term::Map {
                cap: CAP,
                list: Box::new(var(2)),
                body: Box::new(copy_node(0, vec![("mode", var(2)), ("k", var(1))])),
            },
        ),
    );
    b.add(
        "cut",
        def(
            vec![nodes_ty()],
            nodes_ty(),
            b.call("mode", vec![var(0), txt("cut"), fx(0.25)]),
        ),
    );
    b.add(
        "blend",
        def(
            vec![nodes_ty(), Ty::Fix],
            nodes_ty(),
            b.call("mode", vec![var(1), txt("blend"), var(0)]),
        ),
    );
    // part(nodes, i): which material group these steps belong to.
    b.add(
        "part",
        def(
            vec![nodes_ty(), Ty::Int],
            nodes_ty(),
            Term::Map {
                cap: CAP,
                list: Box::new(var(1)),
                body: Box::new(copy_node(0, vec![("part", var(1))])),
            },
        ),
    );
    b.add(
        "round",
        def(
            vec![nodes_ty(), Ty::Fix],
            nodes_ty(),
            Term::Map {
                cap: CAP,
                list: Box::new(var(1)),
                body: Box::new(copy_node(0, vec![("round", var(1))])),
            },
        ),
    );
    b.add(
        "lay",
        def(
            vec![nodes_ty(), Ty::Text],
            nodes_ty(),
            Term::Map {
                cap: CAP,
                list: Box::new(var(1)),
                body: Box::new(copy_node(0, vec![("axis", var(1))])),
            },
        ),
    );
    // join(a, b): the composition primitive — this is what ListCat is for.
    b.add(
        "join",
        def(
            vec![nodes_ty(), nodes_ty()],
            nodes_ty(),
            p2(PrimOp::ListCat, var(1), var(0)),
        ),
    );

    // --- repeaters: where code beats data ---------------------------------
    // ring_of(nodes, n, radius): n copies around a circle, each turned to
    // face out. A fold, because each round appends to the last.
    b.add(
        "ring_of",
        def(
            vec![nodes_ty(), Ty::Int, Ty::Fix],
            nodes_ty(),
            Term::Fold {
                cap: REPEAT_CAP,
                list: Box::new(Term::Iota {
                    cap: REPEAT_CAP,
                    count: Box::new(var(1)),
                }),
                init: Box::new(Term::Map {
                    cap: 0,
                    list: Box::new(var(2)),
                    body: Box::new(var(0)),
                }),
                // acc = Var 1, i = Var 0 → outer nodes = Var 4, n = Var 3, r = Var 2
                body: Box::new(Term::Let(
                    // θ (degrees) = 360·i/n
                    Box::new(mul(
                        div(p1(PrimOp::FixOfInt, var(0)), p1(PrimOp::FixOfInt, var(3))),
                        fx(360.0),
                    )),
                    Box::new(p2(
                        PrimOp::ListCat,
                        var(2),
                        b.call(
                            "spin",
                            vec![
                                b.call(
                                    "at",
                                    vec![
                                        var(5),
                                        // x = r·sin θ, z = r·cos θ — the same
                                        // azimuth convention the browser's
                                        // architecture math uses.
                                        mul(
                                            var(3),
                                            p1(
                                                PrimOp::FSin,
                                                mul(div(var(0), fx(360.0)), Term::Fix(FIX_TAU)),
                                            ),
                                        ),
                                        fx(0.0),
                                        mul(
                                            var(3),
                                            p1(
                                                PrimOp::FCos,
                                                mul(div(var(0), fx(360.0)), Term::Fix(FIX_TAU)),
                                            ),
                                        ),
                                    ],
                                ),
                                var(0),
                            ],
                        ),
                    )),
                )),
            },
        ),
    );
    // row_of(nodes, n, dx, dy, dz): n copies stepped along a vector.
    b.add(
        "row_of",
        def(
            vec![nodes_ty(), Ty::Int, Ty::Fix, Ty::Fix, Ty::Fix],
            nodes_ty(),
            Term::Fold {
                cap: REPEAT_CAP,
                list: Box::new(Term::Iota {
                    cap: REPEAT_CAP,
                    count: Box::new(var(3)),
                }),
                init: Box::new(Term::Map {
                    cap: 0,
                    list: Box::new(var(4)),
                    body: Box::new(var(0)),
                }),
                // acc = Var 1, i = Var 0 → nodes = Var 6, n = Var 5, dx..dz = 4,3,2
                body: Box::new(p2(
                    PrimOp::ListCat,
                    var(1),
                    b.call(
                        "at",
                        vec![
                            var(6),
                            mul(p1(PrimOp::FixOfInt, var(0)), var(4)),
                            mul(p1(PrimOp::FixOfInt, var(0)), var(3)),
                            mul(p1(PrimOp::FixOfInt, var(0)), var(2)),
                        ],
                    ),
                )),
            },
        ),
    );

    // --- parametric parts --------------------------------------------------
    // column(h, r): plinth, torus base, entasis shaft, neck, flared capital —
    // a classical profile, computed from two numbers.
    b.add(
        "column",
        def(
            vec![Ty::Fix, Ty::Fix],
            nodes_ty(),
            b.call(
                "lathe",
                vec![list(vec![
                    fx(0.0),
                    fx(0.0),
                    mul(var(0), fx(1.05)),
                    fx(0.0),
                    mul(var(0), fx(1.05)),
                    mul(var(1), fx(0.035)),
                    mul(var(0), fx(0.82)),
                    mul(var(1), fx(0.06)),
                    mul(var(0), fx(0.72)),
                    mul(var(1), fx(0.10)),
                    // entasis: the shaft swells slightly, then tapers
                    mul(var(0), fx(0.66)),
                    mul(var(1), fx(0.45)),
                    mul(var(0), fx(0.60)),
                    mul(var(1), fx(0.88)),
                    mul(var(0), fx(0.70)),
                    mul(var(1), fx(0.93)),
                    mul(var(0), fx(1.02)),
                    mul(var(1), fx(0.97)),
                    mul(var(0), fx(1.10)),
                    var(1),
                    fx(0.0),
                    var(1),
                ])],
            ),
        ),
    );
    // stairs(n, rise, run, width): a fold — n treads, each stepped up and out.
    b.add(
        "stairs",
        def(
            vec![Ty::Int, Ty::Fix, Ty::Fix, Ty::Fix],
            nodes_ty(),
            Term::Fold {
                cap: REPEAT_CAP,
                list: Box::new(Term::Iota {
                    cap: REPEAT_CAP,
                    count: Box::new(var(3)),
                }),
                init: Box::new(Term::Map {
                    cap: 0,
                    list: Box::new(b.call("cube", vec![fx(1.0), fx(1.0), fx(1.0)])),
                    body: Box::new(var(0)),
                }),
                // acc = Var 1, i = Var 0 → n = Var 5, rise = 4, run = 3, width = 2
                // Each step is SOLID — a block from the ground to its tread,
                // not a floating slab. That is what a staircase looks like,
                // and it costs the same twelve triangles.
                body: Box::new(p2(
                    PrimOp::ListCat,
                    var(1),
                    b.call(
                        "at",
                        vec![
                            b.call(
                                "cube",
                                vec![
                                    var(2),
                                    mul(add(p1(PrimOp::FixOfInt, var(0)), fx(1.0)), var(4)),
                                    var(3),
                                ],
                            ),
                            fx(0.0),
                            // centre of a block rising from 0 to (i+1)·rise
                            mul(
                                mul(add(p1(PrimOp::FixOfInt, var(0)), fx(1.0)), var(4)),
                                fx(0.5),
                            ),
                            mul(add(p1(PrimOp::FixOfInt, var(0)), fx(0.5)), var(3)),
                        ],
                    ),
                )),
            },
        ),
    );
    // arch(w, h, thickness): a slab with a round-topped opening cut out.
    b.add(
        "arch",
        def(
            vec![Ty::Fix, Ty::Fix, Ty::Fix],
            nodes_ty(),
            b.call(
                "join",
                vec![
                    b.call(
                        "at",
                        vec![
                            b.call("cube", vec![var(2), var(1), var(0)]),
                            fx(0.0),
                            div(var(1), fx(2.0)),
                            fx(0.0),
                        ],
                    ),
                    b.call(
                        "cut",
                        vec![b.call(
                            "join",
                            vec![
                                // the round head
                                b.call(
                                    "at",
                                    vec![
                                        b.call(
                                            "lay",
                                            vec![
                                                b.call(
                                                    "cylinder",
                                                    vec![
                                                        mul(var(2), fx(0.33)),
                                                        mul(var(0), fx(2.0)),
                                                    ],
                                                ),
                                                txt("z"),
                                            ],
                                        ),
                                        fx(0.0),
                                        mul(var(1), fx(0.62)),
                                        fx(0.0),
                                    ],
                                ),
                                // the doorway below it
                                b.call(
                                    "at",
                                    vec![
                                        b.call(
                                            "cube",
                                            vec![
                                                mul(var(2), fx(0.66)),
                                                mul(var(1), fx(0.62)),
                                                mul(var(0), fx(2.0)),
                                            ],
                                        ),
                                        fx(0.0),
                                        mul(var(1), fx(0.31)),
                                        fx(0.0),
                                    ],
                                ),
                            ],
                        )],
                    ),
                ],
            ),
        ),
    );
    // vase(h, belly, neck): a turned vessel — the classic lathe demo, and a
    // fair test of whether a language can *describe* a curve.
    b.add(
        "vase",
        def(
            vec![Ty::Fix, Ty::Fix, Ty::Fix],
            nodes_ty(),
            b.call(
                "lathe",
                vec![list(vec![
                    fx(0.0),
                    fx(0.0),
                    mul(var(1), fx(0.55)),
                    fx(0.0),
                    mul(var(1), fx(0.62)),
                    mul(var(2), fx(0.06)),
                    mul(var(1), fx(0.80)),
                    mul(var(2), fx(0.22)),
                    var(1),
                    mul(var(2), fx(0.45)),
                    mul(var(1), fx(0.86)),
                    mul(var(2), fx(0.68)),
                    mul(var(0), fx(1.05)),
                    mul(var(2), fx(0.86)),
                    var(0),
                    mul(var(2), fx(0.95)),
                    mul(var(0), fx(1.25)),
                    var(2),
                    fx(0.0),
                    var(2),
                ])],
            ),
        ),
    );
    // bowl(r, wall): a sphere, flattened and hollowed.
    b.add(
        "bowl",
        def(
            vec![Ty::Fix, Ty::Fix],
            nodes_ty(),
            b.call(
                "join",
                vec![
                    b.call(
                        "join",
                        vec![
                            b.call(
                                "at",
                                vec![b.call("sphere", vec![var(1)]), fx(0.0), var(1), fx(0.0)],
                            ),
                            b.call(
                                "cut",
                                vec![b.call(
                                    "at",
                                    vec![
                                        b.call("sphere", vec![sub(var(1), var(0))]),
                                        fx(0.0),
                                        add(var(1), mul(var(0), fx(0.9))),
                                        fx(0.0),
                                    ],
                                )],
                            ),
                        ],
                    ),
                    // flat foot: cut everything below the base
                    b.call(
                        "cut",
                        vec![b.call(
                            "at",
                            vec![
                                b.call(
                                    "cube",
                                    vec![
                                        mul(var(1), fx(4.0)),
                                        mul(var(1), fx(2.0)),
                                        mul(var(1), fx(4.0)),
                                    ],
                                ),
                                fx(0.0),
                                mul(var(1), fx(-0.85)),
                                fx(0.0),
                            ],
                        )],
                    ),
                ],
            ),
        ),
    );
    // table(w, d, h): a top and four legs — the "does composition work?" part.
    b.add(
        "table",
        def(
            vec![Ty::Fix, Ty::Fix, Ty::Fix],
            nodes_ty(),
            b.call(
                "join",
                vec![
                    b.call(
                        "round",
                        vec![
                            b.call(
                                "at",
                                vec![
                                    b.call("cube", vec![var(2), mul(var(0), fx(0.08)), var(1)]),
                                    fx(0.0),
                                    var(0),
                                    fx(0.0),
                                ],
                            ),
                            fx(0.02),
                        ],
                    ),
                    b.call(
                        "row_of",
                        vec![
                            b.call(
                                "row_of",
                                vec![
                                    b.call(
                                        "at",
                                        vec![
                                            b.call(
                                                "cube",
                                                vec![
                                                    mul(var(2), fx(0.07)),
                                                    mul(var(0), fx(0.96)),
                                                    mul(var(2), fx(0.07)),
                                                ],
                                            ),
                                            mul(var(2), fx(-0.42)),
                                            mul(var(0), fx(0.48)),
                                            mul(var(1), fx(-0.40)),
                                        ],
                                    ),
                                    int(2),
                                    mul(var(2), fx(0.84)),
                                    fx(0.0),
                                    fx(0.0),
                                ],
                            ),
                            int(2),
                            fx(0.0),
                            fx(0.0),
                            mul(var(1), fx(0.80)),
                        ],
                    ),
                ],
            ),
        ),
    );
    // rock(r, seed): three spheres melted together and flattened — organic
    // form from a deterministic "random" (sin is a hash you can verify).
    let jitter = |seed: Term, salt: f32, amp: f32| {
        mul(
            p1(PrimOp::FSin, add(mul(seed, fx(12.9898)), fx(salt))),
            fx(amp),
        )
    };
    b.add(
        "rock",
        def(
            vec![Ty::Fix, Ty::Fix],
            nodes_ty(),
            b.call(
                "join",
                vec![
                    b.call(
                        "blend",
                        vec![
                            b.call(
                                "join",
                                vec![
                                    b.call(
                                        "at",
                                        vec![
                                            b.call("sphere", vec![var(1)]),
                                            fx(0.0),
                                            mul(var(1), fx(0.8)),
                                            fx(0.0),
                                        ],
                                    ),
                                    b.call(
                                        "join",
                                        vec![
                                            b.call(
                                                "at",
                                                vec![
                                                    b.call("sphere", vec![mul(var(1), fx(0.72))]),
                                                    mul(var(1), jitter(var(0), 1.7, 0.6)),
                                                    mul(var(1), fx(0.62)),
                                                    mul(var(1), jitter(var(0), 4.3, 0.6)),
                                                ],
                                            ),
                                            b.call(
                                                "at",
                                                vec![
                                                    b.call("sphere", vec![mul(var(1), fx(0.58))]),
                                                    mul(var(1), jitter(var(0), 8.1, -0.7)),
                                                    mul(var(1), fx(0.52)),
                                                    mul(var(1), jitter(var(0), 2.9, -0.5)),
                                                ],
                                            ),
                                        ],
                                    ),
                                ],
                            ),
                            mul(var(1), fx(0.42)),
                        ],
                    ),
                    // sit it flat on the ground
                    b.call(
                        "cut",
                        vec![b.call(
                            "at",
                            vec![
                                b.call(
                                    "cube",
                                    vec![
                                        mul(var(1), fx(6.0)),
                                        mul(var(1), fx(2.0)),
                                        mul(var(1), fx(6.0)),
                                    ],
                                ),
                                fx(0.0),
                                mul(var(1), fx(-1.0)),
                                fx(0.0),
                            ],
                        )],
                    ),
                ],
            ),
        ),
    );

    // --- materials: the full PBR set, tuned, by name -----------------------
    let recipe = |kind: &str,
                  scale: f32,
                  octaves: i64,
                  seed: i64,
                  colors: Vec<[f32; 3]>,
                  rough: [f32; 2],
                  metal: [f32; 2],
                  height: f32,
                  ao: f32,
                  triplanar: f32| {
        rec(vec![
            ("kind", txt(kind)),
            ("scale", fx(scale)),
            ("octaves", int(octaves)),
            ("seed", int(seed)),
            (
                "colors",
                list(
                    colors
                        .into_iter()
                        .map(|c| list(vec![fx(c[0]), fx(c[1]), fx(c[2])]))
                        .collect(),
                ),
            ),
            ("roughness", list(vec![fx(rough[0]), fx(rough[1])])),
            ("metallic", list(vec![fx(metal[0]), fx(metal[1])])),
            ("height", fx(height)),
            ("ao", fx(ao)),
            ("size", int(256)),
            ("triplanar", fx(triplanar)),
        ])
    };
    // A material wraps a recipe with the render-side knobs. `over` is a
    // declared-but-unused layer (mix 0) that `weathered` can switch on.
    let material = |name: &str, texture: Term, uv: &str| {
        let base = texture.clone();
        rec(vec![
            ("name", txt(name)),
            ("color", list(vec![fx(1.0), fx(1.0), fx(1.0), fx(1.0)])),
            ("emissive", fx(0.0)),
            ("resolution", int(48)),
            ("uv", txt(uv)),
            ("uv_scale", fx(0.5)),
            ("texture", {
                let mut f: BTreeMap<String, Term> = match base {
                    Term::Rec(m) => m,
                    _ => unreachable!(),
                };
                f.insert(
                    "over".into(),
                    match texture {
                        Term::Rec(m) => Term::Rec(m),
                        _ => unreachable!(),
                    },
                );
                f.insert("mix".into(), fx(0.0));
                f.insert("mask_scale".into(), fx(3.0));
                f.insert("mask_seed".into(), int(0));
                Term::Rec(f)
            }),
        ])
    };
    let moss_recipe = recipe(
        "voronoi",
        9.0,
        4,
        21,
        vec![[0.16, 0.28, 0.13], [0.30, 0.45, 0.20], [0.22, 0.36, 0.16]],
        [1.0, 0.9],
        [0.0, 0.0],
        0.5,
        0.4,
        0.5,
    );
    for (name, r, uv) in [
        (
            "marble",
            recipe(
                "veins",
                3.0,
                5,
                5,
                vec![[0.42, 0.43, 0.47], [0.86, 0.85, 0.83], [0.97, 0.96, 0.94]],
                [0.34, 0.20],
                [0.0, 0.0],
                0.10,
                0.20,
                0.5,
            ),
            "auto",
        ),
        (
            "granite",
            recipe(
                "voronoi",
                7.0,
                4,
                11,
                vec![[0.40, 0.40, 0.43], [0.56, 0.54, 0.52], [0.33, 0.33, 0.38]],
                [0.95, 0.78],
                [0.0, 0.05],
                0.40,
                0.45,
                0.5,
            ),
            "auto",
        ),
        (
            "sandstone",
            recipe(
                "fbm",
                5.0,
                5,
                3,
                vec![[0.72, 0.62, 0.45], [0.87, 0.79, 0.62]],
                [0.94, 0.82],
                [0.0, 0.0],
                0.35,
                0.35,
                0.5,
            ),
            "auto",
        ),
        (
            "wood",
            recipe(
                "wood",
                4.0,
                4,
                7,
                vec![[0.32, 0.20, 0.11], [0.55, 0.36, 0.20], [0.44, 0.28, 0.15]],
                [0.78, 0.55],
                [0.0, 0.0],
                0.30,
                0.25,
                0.0,
            ),
            "box",
        ),
        (
            "iron",
            recipe(
                "fbm",
                7.0,
                4,
                9,
                vec![[0.18, 0.18, 0.20], [0.34, 0.33, 0.35]],
                [0.55, 0.32],
                [0.9, 1.0],
                0.18,
                0.25,
                0.0,
            ),
            "auto",
        ),
        (
            "brass",
            recipe(
                "fbm",
                6.0,
                3,
                13,
                vec![[0.52, 0.40, 0.15], [0.85, 0.70, 0.32]],
                [0.42, 0.22],
                [1.0, 1.0],
                0.12,
                0.20,
                0.0,
            ),
            "auto",
        ),
        (
            "terracotta",
            recipe(
                "fbm",
                5.0,
                5,
                4,
                vec![[0.48, 0.26, 0.16], [0.70, 0.44, 0.27]],
                [0.80, 0.58],
                [0.0, 0.0],
                0.22,
                0.22,
                0.0,
            ),
            "auto",
        ),
        (
            "plaster",
            recipe(
                "fbm",
                3.0,
                4,
                17,
                vec![[0.80, 0.78, 0.74], [0.92, 0.90, 0.86]],
                [0.92, 0.85],
                [0.0, 0.0],
                0.10,
                0.18,
                0.5,
            ),
            "auto",
        ),
        ("moss", moss_recipe.clone(), "auto"),
    ] {
        b.add(name, def(vec![], material_ty(), material(name, r, uv)));
    }
    // tint(material, r, g, b): the same material, another colour.
    b.add(
        "tint",
        def(
            vec![material_ty(), Ty::Fix, Ty::Fix, Ty::Fix],
            material_ty(),
            rec(vec![
                ("name", get(var(3), "name")),
                ("color", list(vec![var(2), var(1), var(0), fx(1.0)])),
                ("emissive", get(var(3), "emissive")),
                ("resolution", get(var(3), "resolution")),
                ("uv", get(var(3), "uv")),
                ("uv_scale", get(var(3), "uv_scale")),
                ("texture", get(var(3), "texture")),
            ]),
        ),
    );
    // glow(material, strength): the same material, lit from within.
    b.add(
        "glow",
        def(
            vec![material_ty(), Ty::Fix],
            material_ty(),
            rec(vec![
                ("name", get(var(1), "name")),
                ("color", get(var(1), "color")),
                ("emissive", var(0)),
                ("resolution", get(var(1), "resolution")),
                ("uv", get(var(1), "uv")),
                ("uv_scale", get(var(1), "uv_scale")),
                ("texture", get(var(1), "texture")),
            ]),
        ),
    );
    // weathered(material, mix): moss creeping over it — layering, switched on.
    b.add(
        "weathered",
        def(
            vec![material_ty(), Ty::Fix],
            material_ty(),
            rec(vec![
                ("name", get(var(1), "name")),
                ("color", get(var(1), "color")),
                ("emissive", get(var(1), "emissive")),
                ("resolution", get(var(1), "resolution")),
                ("uv", get(var(1), "uv")),
                ("uv_scale", get(var(1), "uv_scale")),
                ("texture", {
                    let tex = get(var(1), "texture");
                    let mut f: BTreeMap<String, Term> = BTreeMap::new();
                    for k in [
                        "ao",
                        "colors",
                        "height",
                        "kind",
                        "metallic",
                        "octaves",
                        "roughness",
                        "scale",
                        "seed",
                        "size",
                        "triplanar",
                    ] {
                        f.insert(k.into(), get(tex.clone(), k));
                    }
                    f.insert("over".into(), moss_recipe.clone());
                    f.insert("mix".into(), var(0));
                    f.insert("mask_scale".into(), fx(2.5));
                    f.insert("mask_seed".into(), int(19));
                    Term::Rec(f)
                }),
            ]),
        ),
    );

    // --- assembly ----------------------------------------------------------
    b.add(
        "model",
        def(
            vec![Ty::Text, nodes_ty(), Ty::List(Box::new(material_ty()))],
            model_ty(),
            rec(vec![
                ("name", var(2)),
                ("nodes", var(1)),
                ("materials", var(0)),
            ]),
        ),
    );
    b.add(
        "model1",
        def(
            vec![Ty::Text, nodes_ty(), material_ty()],
            model_ty(),
            rec(vec![
                ("name", var(2)),
                ("nodes", var(1)),
                ("materials", list(vec![var(0)])),
            ]),
        ),
    );

    // --- the catalog: what this package can MAKE, and how to ask ----------
    //
    // A layout that cannot find a part in the store needs to commission one,
    // and to commission it must know which export makes a `column` and what
    // to pass it. Declaring that here turns any published package into a
    // *supplier*: the store stops being a pile of files and becomes a market
    // of parts and the programs that make them.
    //
    // `args` is a tiny expression per parameter, evaluated by the binder
    // against the need's own dimensions: `h`, `w`, `d` (metres), a literal
    // number, or either scaled — `w*0.5`. `material` names a palette slot
    // (`stone`, `wood`, `metal`) so a commission inherits the place's style.
    let catalog_entry_ty = rec_ty(vec![
        ("args", Ty::List(Box::new(Ty::Text))),
        ("export", Ty::Text),
        ("kind", Ty::Text),
        ("material", Ty::Text),
    ]);
    let entry = |kind: &str, export: &str, args: Vec<&str>, material: &str| {
        rec(vec![
            ("kind", txt(kind)),
            ("export", txt(export)),
            ("args", list(args.into_iter().map(txt).collect())),
            ("material", txt(material)),
        ])
    };
    b.add(
        "catalog",
        def(
            vec![],
            Ty::List(Box::new(catalog_entry_ty)),
            list(vec![
                entry("column", "column", vec!["h", "w*0.5"], "stone"),
                entry("arch", "arch", vec!["w", "h", "d"], "stone"),
                entry(
                    "stairs",
                    "stairs",
                    vec!["12", "h*0.083", "d*0.083", "w"],
                    "stone",
                ),
                entry("table", "table", vec!["w", "d", "h"], "wood"),
                entry("vessel", "vase", vec!["h", "w*0.34", "w*0.13"], "stone"),
                entry("bowl", "bowl", vec!["w*0.5", "w*0.08"], "stone"),
                entry("rock", "rock", vec!["w*0.5", "3"], "stone"),
                entry("boulder", "rock", vec!["w*0.5", "7"], "stone"),
            ]),
        ),
    );

    // --- a showpiece: everything at once, in one call ----------------------
    // amphora(h): a two-material vessel — terracotta body, brass collar.
    b.add(
        "amphora",
        def(
            vec![Ty::Fix],
            model_ty(),
            b.call(
                "model",
                vec![
                    txt("amphora"),
                    b.call(
                        "join",
                        vec![
                            // vase(height, belly radius, neck radius)
                            b.call(
                                "vase",
                                vec![var(0), mul(var(0), fx(0.34)), mul(var(0), fx(0.13))],
                            ),
                            b.call(
                                "part",
                                vec![
                                    b.call(
                                        "at",
                                        vec![
                                            b.call(
                                                "torus",
                                                vec![mul(var(0), fx(0.17)), mul(var(0), fx(0.028))],
                                            ),
                                            fx(0.0),
                                            mul(var(0), fx(0.88)),
                                            fx(0.0),
                                        ],
                                    ),
                                    int(1),
                                ],
                            ),
                        ],
                    ),
                    list(vec![b.call("terracotta", vec![]), b.call("brass", vec![])]),
                ],
            ),
        ),
    );

    let names: Vec<String> = b.by_name.keys().cloned().collect();
    let exports: Vec<(&str, usize)> = names.iter().map(|n| (n.as_str(), b.idx(n))).collect();
    Package::build("weft-model", b.defs.clone(), exports).expect("library builds")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{eval_call, pack::link, Value};

    fn call_export(pkg: &Package, name: &str, args: Vec<Value>) -> Value {
        let hash = pkg
            .export(name)
            .unwrap_or_else(|| panic!("export '{name}'"));
        let entry = pkg.defs.get(&hash).cloned().expect("def");
        let module = link(&[pkg.clone()], vec![entry], 0).expect("links");
        eval_call(&module, module.entry, args, 40_000_000)
            .unwrap_or_else(|e| panic!("'{name}' failed: {e:?}"))
            .value
    }

    fn nodes_len(v: &Value) -> usize {
        match v {
            Value::List(xs) => xs.len(),
            other => panic!("expected a node list, got {other:?}"),
        }
    }

    #[test]
    fn the_library_verifies_as_a_package() {
        let pkg = package();
        pkg.verify()
            .expect("every def type-checks, terminates, and hashes");
        assert!(
            pkg.exports.len() > 25,
            "a library worth the name: {}",
            pkg.exports.len()
        );
        for must in [
            "sphere", "at", "join", "stairs", "column", "marble", "model1", "amphora",
        ] {
            assert!(pkg.exports.contains_key(must), "exports {must}");
        }
    }

    #[test]
    fn primitives_and_transforms_compose() {
        let pkg = package();
        let one = call_export(&pkg, "sphere", vec![Value::Fix(FIX_SCALE)]);
        assert_eq!(nodes_len(&one), 1);

        // join concatenates; at moves; part regroups.
        let joined = call_export(
            &pkg,
            "join",
            vec![
                call_export(&pkg, "sphere", vec![Value::Fix(FIX_SCALE)]),
                call_export(&pkg, "cube", vec![Value::Fix(FIX_SCALE); 3]),
            ],
        );
        assert_eq!(nodes_len(&joined), 2);

        let moved = call_export(
            &pkg,
            "at",
            vec![
                call_export(&pkg, "sphere", vec![Value::Fix(FIX_SCALE)]),
                Value::Fix(2 * FIX_SCALE),
                Value::Fix(3 * FIX_SCALE),
                Value::Fix(4 * FIX_SCALE),
            ],
        );
        let Value::List(items) = &moved else { panic!() };
        let Value::Rec(f) = &items[0] else { panic!() };
        assert_eq!(f["x"], Value::Fix(2 * FIX_SCALE));
        assert_eq!(f["y"], Value::Fix(3 * FIX_SCALE));
        assert_eq!(f["z"], Value::Fix(4 * FIX_SCALE));

        // spin turns positions about Y: +x at 90° becomes… −z (the browser's
        // azimuth convention), and the step keeps its own rotation.
        let spun = call_export(
            &pkg,
            "spin",
            vec![
                call_export(
                    &pkg,
                    "at",
                    vec![
                        call_export(&pkg, "sphere", vec![Value::Fix(FIX_SCALE)]),
                        Value::Fix(FIX_SCALE),
                        Value::Fix(0),
                        Value::Fix(0),
                    ],
                ),
                Value::Fix(90 * FIX_SCALE),
            ],
        );
        let Value::List(items) = &spun else { panic!() };
        let Value::Rec(f) = &items[0] else { panic!() };
        let (x, z) = (
            match f["x"] {
                Value::Fix(v) => v as f64 / FIX_SCALE as f64,
                _ => panic!(),
            },
            match f["z"] {
                Value::Fix(v) => v as f64 / FIX_SCALE as f64,
                _ => panic!(),
            },
        );
        assert!(x.abs() < 0.02, "x ≈ 0 after a quarter turn, got {x}");
        assert!(
            (z + 1.0).abs() < 0.02,
            "z ≈ −1 after a quarter turn, got {z}"
        );
    }

    #[test]
    fn loops_are_what_make_it_a_language() {
        let pkg = package();
        // Twelve treads from one fold — the thing data can't do.
        let steps = call_export(
            &pkg,
            "stairs",
            vec![
                Value::Int(12),
                Value::Fix(180_000),
                Value::Fix(280_000),
                Value::Fix(1_200_000),
            ],
        );
        assert_eq!(nodes_len(&steps), 12);

        // A ring of eight, on the circle it was given.
        let ring = call_export(
            &pkg,
            "ring_of",
            vec![
                call_export(&pkg, "sphere", vec![Value::Fix(FIX_SCALE / 4)]),
                Value::Int(8),
                Value::Fix(3 * FIX_SCALE),
            ],
        );
        assert_eq!(nodes_len(&ring), 8);
        let Value::List(items) = &ring else { panic!() };
        for it in items {
            let Value::Rec(f) = it else { panic!() };
            let g = |k: &str| match f[k] {
                Value::Fix(v) => v as f64 / FIX_SCALE as f64,
                _ => panic!(),
            };
            let r = (g("x") * g("x") + g("z") * g("z")).sqrt();
            assert!((r - 3.0).abs() < 0.05, "on the circle: {r}");
        }

        // row_of steps along its vector.
        let row = call_export(
            &pkg,
            "row_of",
            vec![
                call_export(&pkg, "sphere", vec![Value::Fix(FIX_SCALE / 4)]),
                Value::Int(5),
                Value::Fix(FIX_SCALE),
                Value::Fix(0),
                Value::Fix(0),
            ],
        );
        assert_eq!(nodes_len(&row), 5);
    }

    #[test]
    fn every_part_and_material_evaluates() {
        let pkg = package();
        let f = |v: f32| Value::Fix((v * FIX_SCALE as f32) as i64);
        for (name, args) in [
            ("column", vec![f(5.2), f(0.44)]),
            ("vase", vec![f(0.42), f(0.34), f(1.2)]),
            ("bowl", vec![f(0.5), f(0.06)]),
            ("arch", vec![f(3.0), f(4.0), f(0.6)]),
            ("table", vec![f(1.6), f(0.9), f(0.75)]),
            ("rock", vec![f(0.8), f(3.0)]),
        ] {
            let v = call_export(&pkg, name, args);
            assert!(nodes_len(&v) >= 1, "{name} carved nothing");
        }
        for mat in [
            "marble",
            "granite",
            "sandstone",
            "wood",
            "iron",
            "brass",
            "terracotta",
            "plaster",
            "moss",
        ] {
            let v = call_export(&pkg, mat, vec![]);
            let Value::Rec(m) = &v else {
                panic!("{mat} is a material")
            };
            assert!(m.contains_key("texture"), "{mat} carries a PBR recipe");
            let Value::Rec(t) = &m["texture"] else {
                panic!()
            };
            assert!(t.contains_key("colors") && t.contains_key("roughness"));
        }
    }
}
