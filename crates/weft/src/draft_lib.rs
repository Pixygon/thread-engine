//! # weft-draft — level design as a verified program
//!
//! In weaving, the **draft** is the pattern a cloth is woven from. This is
//! that, for places: a library of *figures* — halls, colonnades, thresholds,
//! stairs, groves — that compose into a [plan](infinite_manifest::plan): the
//! ground, the lights, the ways in and out, and a **bill of requirements**
//! saying what the place needs without naming a single file.
//!
//! Two properties fall out of being Weft, and both matter more than they
//! sound:
//!
//! - **The same brief rebuilds the same place, forever.** No hidden clock, no
//!   platform float drift; a seed makes siblings on purpose, never by accident.
//! - **Taste is written down.** Door widths, ceiling ratios, lamp spacing,
//!   the clearance a veil needs from a spawn — these are constants with names
//!   ([`infinite_manifest::plan::metric`]), used by every figure, so a layout
//!   is critiqued by reading it rather than by walking it.
//!
//! What this library deliberately cannot do is *shop*. Finding or
//! commissioning the models a plan asks for is an effect, and effects live at
//! the edge (`thread level`), never inside a verified program.

use std::collections::{BTreeMap, BTreeSet};

use crate::pack::Package;
use crate::{hash_def, Def, PrimOp, Term, Ty, WeftHash, FIX_SCALE, FIX_TAU};

// --- the same small comfort layer the modeling library uses ----------------

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
fn p1(op: PrimOp, a: Term) -> Term {
    Term::Prim(op, vec![a])
}
fn p2(op: PrimOp, a: Term, b: Term) -> Term {
    Term::Prim(op, vec![a, b])
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
fn get(t: Term, f: &str) -> Term {
    Term::Get(Box::new(t), f.to_string())
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
fn vec3(x: Term, y: Term, z: Term) -> Term {
    list(vec![x, y, z])
}
fn vec3_ty() -> Ty {
    Ty::List(Box::new(Ty::Fix))
}
/// An empty list of a *known* element type (an empty `ListNew` has none).
fn none_of(elem: Term) -> Term {
    Term::Map {
        cap: 0,
        list: Box::new(list(vec![elem])),
        body: Box::new(var(0)),
    }
}

const CAP: u32 = 512;
const REPEAT_CAP: u32 = 128;

// --- the plan ABI, mirroring infinite_manifest::plan ------------------------

fn need_ty() -> Ty {
    rec_ty(vec![
        ("at", vec3_ty()),
        ("commission", Ty::Bool),
        ("d", Ty::Fix),
        ("h", Ty::Fix),
        ("kind", Ty::Text),
        ("must", Ty::Bool),
        ("name", Ty::Text),
        ("solid", Ty::Bool),
        ("style", Ty::Text),
        ("tags", Ty::List(Box::new(Ty::Text))),
        ("tol", Ty::Fix),
        ("w", Ty::Fix),
        ("yaw", Ty::Fix),
    ])
}
fn veil_ty() -> Ty {
    rec_ty(vec![
        ("at", vec3_ty()),
        ("label", Ty::Text),
        ("to", Ty::Text),
        ("yaw", Ty::Fix),
    ])
}
fn spawn_ty() -> Ty {
    rec_ty(vec![
        ("at", vec3_ty()),
        ("name", Ty::Text),
        ("yaw", Ty::Fix),
    ])
}
fn light_ty() -> Ty {
    rec_ty(vec![
        ("at", vec3_ty()),
        ("fixture", Ty::Bool),
        ("intensity", Ty::Fix),
        ("range", Ty::Fix),
        ("warm", Ty::Fix),
    ])
}
fn sign_ty() -> Ty {
    rec_ty(vec![
        ("at", vec3_ty()),
        ("h", Ty::Fix),
        ("text", Ty::Text),
        ("w", Ty::Fix),
        ("yaw", Ty::Fix),
    ])
}
fn build_ty() -> Ty {
    rec_ty(vec![
        ("at", vec3_ty()),
        ("d", Ty::Fix),
        ("h", Ty::Fix),
        ("material", Ty::Text),
        ("name", Ty::Text),
        ("r", Ty::Fix),
        ("shape", Ty::Text),
        ("solid", Ty::Bool),
        ("w", Ty::Fix),
        ("yaw", Ty::Fix),
    ])
}
fn palette_ty() -> Ty {
    rec_ty(vec![
        ("metal", Ty::Text),
        ("sky", Ty::Text),
        ("stone", Ty::Text),
        ("style", Ty::Text),
        ("wood", Ty::Text),
    ])
}
fn plan_ty() -> Ty {
    rec_ty(vec![
        ("description", Ty::Text),
        ("lights", Ty::List(Box::new(light_ty()))),
        ("name", Ty::Text),
        ("needs", Ty::List(Box::new(need_ty()))),
        ("palette", palette_ty()),
        ("builds", Ty::List(Box::new(build_ty()))),
        ("signs", Ty::List(Box::new(sign_ty()))),
        ("spawns", Ty::List(Box::new(spawn_ty()))),
        ("veils", Ty::List(Box::new(veil_ty()))),
    ])
}

fn need(kind: &str, over: Vec<(&str, Term)>) -> Term {
    let mut f: BTreeMap<String, Term> = BTreeMap::new();
    f.insert("kind".into(), txt(kind));
    f.insert("at".into(), vec3(fx(0.0), fx(0.0), fx(0.0)));
    f.insert("yaw".into(), fx(0.0));
    for k in ["w", "h", "d"] {
        f.insert(k.into(), fx(0.0));
    }
    f.insert("tol".into(), fx(0.25));
    f.insert("style".into(), txt(""));
    f.insert("tags".into(), none_of(txt("")));
    f.insert("commission".into(), Term::Bool(true));
    f.insert("must".into(), Term::Bool(true));
    f.insert("solid".into(), Term::Bool(true));
    f.insert("name".into(), txt(""));
    for (k, v) in over {
        f.insert(k.into(), v);
    }
    Term::Rec(f)
}

fn copy_need(elem: u32, over: Vec<(&str, Term)>) -> Term {
    let mut f: BTreeMap<String, Term> = BTreeMap::new();
    for k in [
        "at",
        "commission",
        "d",
        "h",
        "kind",
        "must",
        "name",
        "solid",
        "style",
        "tags",
        "tol",
        "w",
        "yaw",
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
        Term::Call(self.by_name[name].1, args)
    }
    fn idx(&self, name: &str) -> usize {
        self.by_name[name].0
    }
}

/// Build the `weft-draft` package.
pub fn package() -> Package {
    let mut b = Builder::new();

    // --- one of a thing, somewhere ----------------------------------------
    b.add(
        "need",
        def(
            vec![
                Ty::Text,
                Ty::Fix,
                Ty::Fix,
                Ty::Fix,
                Ty::Fix,
                Ty::Fix,
                Ty::Fix,
                Ty::Fix,
            ],
            Ty::List(Box::new(need_ty())),
            // need(kind, x, y, z, yaw, w, h, d)
            list(vec![need(
                "",
                vec![
                    ("kind", var(7)),
                    ("at", vec3(var(6), var(5), var(4))),
                    ("yaw", var(3)),
                    ("w", var(2)),
                    ("h", var(1)),
                    ("d", var(0)),
                ],
            )]),
        ),
    );
    b.add(
        "join",
        def(
            vec![Ty::List(Box::new(need_ty())), Ty::List(Box::new(need_ty()))],
            Ty::List(Box::new(need_ty())),
            p2(PrimOp::ListCat, var(1), var(0)),
        ),
    );
    b.add(
        "styled",
        def(
            vec![Ty::List(Box::new(need_ty())), Ty::Text],
            Ty::List(Box::new(need_ty())),
            Term::Map {
                cap: CAP,
                list: Box::new(var(1)),
                body: Box::new(copy_need(0, vec![("style", var(1))])),
            },
        ),
    );
    b.add(
        "loose",
        def(
            vec![Ty::List(Box::new(need_ty())), Ty::Fix],
            Ty::List(Box::new(need_ty())),
            Term::Map {
                cap: CAP,
                list: Box::new(var(1)),
                body: Box::new(copy_need(0, vec![("tol", var(1))])),
            },
        ),
    );
    b.add(
        "scatterable",
        def(
            vec![Ty::List(Box::new(need_ty()))],
            Ty::List(Box::new(need_ty())),
            Term::Map {
                cap: CAP,
                list: Box::new(var(0)),
                body: Box::new(copy_need(
                    0,
                    vec![
                        ("must", Term::Bool(false)),
                        ("commission", Term::Bool(false)),
                    ],
                )),
            },
        ),
    );

    // --- slots: repetition that cannot collide -----------------------------
    // ring_of(needs, n, radius): n copies around a circle, each turned to
    // face the centre. Slots, not free coordinates — overlap is impossible
    // by construction, which removes a whole class of layout fault.
    b.add(
        "ring_of",
        def(
            vec![Ty::List(Box::new(need_ty())), Ty::Int, Ty::Fix],
            Ty::List(Box::new(need_ty())),
            Term::Fold {
                cap: REPEAT_CAP,
                list: Box::new(Term::Iota {
                    cap: REPEAT_CAP,
                    count: Box::new(var(1)),
                }),
                init: Box::new(none_of(need("column", vec![]))),
                // [i=0, acc=1, radius=2, n=3, needs=4]
                //
                // Slots sit HALF A STEP off the axes. A ring that starts at 0°
                // puts a column dead in front of the entrance and another in
                // front of the far threshold — the two places a visitor is
                // guaranteed to be looking. Offsetting keeps both axes clear,
                // which is why colonnades in real buildings have a gap where
                // the door is.
                body: Box::new(Term::Let(
                    // az in degrees
                    Box::new(mul(
                        div(
                            add(p1(PrimOp::FixOfInt, var(0)), fx(0.5)),
                            p1(PrimOp::FixOfInt, var(3)),
                        ),
                        fx(360.0),
                    )),
                    // [az=0, i=1, acc=2, radius=3, n=4, needs=5]
                    Box::new(p2(
                        PrimOp::ListCat,
                        var(2),
                        Term::Map {
                            cap: CAP,
                            list: Box::new(var(5)),
                            // [elem=0, az=1, i=2, acc=3, radius=4, n=5, needs=6]
                            body: Box::new(copy_need(
                                0,
                                vec![
                                    (
                                        "at",
                                        // The engine's azimuth convention
                                        // (infinite_manifest::arch): 0 faces
                                        // −z, away from arrivals; a point at
                                        // az sits at (sin az, −cos az)·r.
                                        vec3(
                                            mul(
                                                var(4),
                                                p1(
                                                    PrimOp::FSin,
                                                    mul(div(var(1), fx(360.0)), Term::Fix(FIX_TAU)),
                                                ),
                                            ),
                                            // Ground level: models stand on
                                            // their own base (facts.origin).
                                            fx(0.0),
                                            sub(
                                                fx(0.0),
                                                mul(
                                                    var(4),
                                                    p1(
                                                        PrimOp::FCos,
                                                        mul(
                                                            div(var(1), fx(360.0)),
                                                            Term::Fix(FIX_TAU),
                                                        ),
                                                    ),
                                                ),
                                            ),
                                        ),
                                    ),
                                    // Face the centre: ψ = −az, the same law
                                    // arch::Corridor::yaw_across derives.
                                    ("yaw", sub(fx(0.0), var(1))),
                                ],
                            )),
                        },
                    )),
                )),
            },
        ),
    );
    // row_of(needs, n, from_x, from_z, dx, dz, yaw)
    b.add(
        "row_of",
        def(
            vec![
                Ty::List(Box::new(need_ty())),
                Ty::Int,
                Ty::Fix,
                Ty::Fix,
                Ty::Fix,
                Ty::Fix,
                Ty::Fix,
            ],
            Ty::List(Box::new(need_ty())),
            Term::Fold {
                cap: REPEAT_CAP,
                list: Box::new(Term::Iota {
                    cap: REPEAT_CAP,
                    count: Box::new(var(5)),
                }),
                init: Box::new(none_of(need("column", vec![]))),
                // [i=0, acc=1, yaw=2, dz=3, dx=4, fz=5, fx=6, n=7, needs=8]
                body: Box::new(p2(
                    PrimOp::ListCat,
                    var(1),
                    Term::Map {
                        cap: CAP,
                        list: Box::new(var(8)),
                        // [elem=0, i=1, acc=2, yaw=3, dz=4, dx=5, fz=6, fx=7, n=8, needs=9]
                        body: Box::new(copy_need(
                            0,
                            vec![
                                (
                                    "at",
                                    vec3(
                                        add(var(7), mul(p1(PrimOp::FixOfInt, var(1)), var(5))),
                                        fx(0.0),
                                        add(var(6), mul(p1(PrimOp::FixOfInt, var(1)), var(4))),
                                    ),
                                ),
                                ("yaw", var(3)),
                            ],
                        )),
                    },
                )),
            },
        ),
    );

    // --- built stone: what the browser draws itself ------------------------
    //
    // A wall segment is a box. Asking a store for one — or worse,
    // commissioning it — would be absurd, so built geometry is its own list
    // and never goes shopping. This is also what keeps a drafted world light:
    // a hundred wall boxes are one prefab and one instanced draw.
    let build = |shape: &str, over: Vec<(&str, Term)>| {
        let mut f: BTreeMap<String, Term> = BTreeMap::new();
        f.insert("shape".into(), txt(shape));
        f.insert("at".into(), vec3(fx(0.0), fx(0.0), fx(0.0)));
        f.insert("yaw".into(), fx(0.0));
        for k in ["r", "w", "d"] {
            f.insert(k.into(), fx(0.0));
        }
        f.insert("h".into(), fx(0.1));
        f.insert("material".into(), txt("stone"));
        f.insert("solid".into(), Term::Bool(true));
        f.insert("name".into(), txt(""));
        for (k, v) in over {
            f.insert(k.into(), v);
        }
        Term::Rec(f)
    };
    let empty_builds = none_of(build("box", vec![]));

    b.add(
        "ground",
        def(
            vec![Ty::Fix],
            Ty::List(Box::new(build_ty())),
            list(vec![build(
                "disc",
                vec![
                    ("r", var(0)),
                    ("h", fx(0.12)),
                    ("at", vec3(fx(0.0), fx(0.02), fx(0.0))),
                    ("material", txt("floor")),
                    ("name", txt("floor")),
                ],
            )]),
        ),
    );
    b.add(
        "join_builds",
        def(
            vec![
                Ty::List(Box::new(build_ty())),
                Ty::List(Box::new(build_ty())),
            ],
            Ty::List(Box::new(build_ty())),
            p2(PrimOp::ListCat, var(1), var(0)),
        ),
    );

    // labelled(needs, prefix): the same courtesy for requirements. The binder
    // reports what it bound by name; "column" twelve times over two different
    // colonnades is a report nobody can read.
    b.add(
        "labelled",
        def(
            vec![Ty::List(Box::new(need_ty())), Ty::Text],
            Ty::List(Box::new(need_ty())),
            Term::Map {
                cap: CAP,
                list: Box::new(var(1)),
                // [want=0, prefix=1, needs=2]
                body: Box::new(rec(vec![
                    ("at", get(var(0), "at")),
                    ("commission", get(var(0), "commission")),
                    ("d", get(var(0), "d")),
                    ("h", get(var(0), "h")),
                    ("kind", get(var(0), "kind")),
                    ("must", get(var(0), "must")),
                    ("name", p2(PrimOp::Concat, var(1), get(var(0), "name"))),
                    ("solid", get(var(0), "solid")),
                    ("style", get(var(0), "style")),
                    ("tags", get(var(0), "tags")),
                    ("tol", get(var(0), "tol")),
                    ("w", get(var(0), "w")),
                    ("yaw", get(var(0), "yaw")),
                ])),
            },
        ),
    );

    // prefixed(builds, prefix): rename a run of built pieces. Two rings in one
    // plan are both made of "wall" — and a reader (or a linter, or the next
    // agent) then cannot tell the rotunda from the boundary. Naming is part of
    // the design, so it gets a combinator rather than a parameter on every
    // figure that emits geometry.
    b.add(
        "prefixed",
        def(
            vec![Ty::List(Box::new(build_ty())), Ty::Text],
            Ty::List(Box::new(build_ty())),
            Term::Map {
                cap: CAP,
                list: Box::new(var(1)),
                // [piece=0, prefix=1, builds=2]
                body: Box::new(rec(vec![
                    ("at", get(var(0), "at")),
                    ("d", get(var(0), "d")),
                    ("h", get(var(0), "h")),
                    ("material", get(var(0), "material")),
                    ("name", p2(PrimOp::Concat, var(1), get(var(0), "name"))),
                    ("r", get(var(0), "r")),
                    ("shape", get(var(0), "shape")),
                    ("solid", get(var(0), "solid")),
                    ("w", get(var(0), "w")),
                    ("yaw", get(var(0), "yaw")),
                ])),
            },
        ),
    );

    // wall_ring(r, n, h, thick, gate_az, gate_reach): a ring of wall segments
    // with openings. A gate is an azimuth; segments whose midpoint falls
    // within `reach` of one are simply not built — which is how a doorway
    // exists in a wall that was never a hole.
    // |a − b| folded into 0…180. There is no modulo in Weft, so the fold is
    // written out — and it must take the absolute value TWICE. A gate at
    // −125° against a wall at 262° differs by 387°, and `360 − 387` is
    // negative: without the second abs that reads as "zero degrees apart",
    // and the gate silently eats half the ring. (It did. The wall count in
    // the test is what caught it.)
    let ang_dist = |a: Term, bb: Term| {
        let abs = |t: Term| {
            Term::Let(
                Box::new(t),
                Box::new(Term::If(
                    Box::new(p2(PrimOp::FLt, var(0), fx(0.0))),
                    Box::new(sub(fx(0.0), var(0))),
                    Box::new(var(0)),
                )),
            )
        };
        Term::Let(
            Box::new(abs(sub(a, bb))),
            Box::new(Term::If(
                Box::new(p2(PrimOp::FLt, fx(180.0), var(0))),
                Box::new(abs(sub(fx(360.0), var(0)))),
                Box::new(var(0)),
            )),
        )
    };
    b.add(
        "wall_ring",
        def(
            vec![
                Ty::Fix,
                Ty::Int,
                Ty::Fix,
                Ty::Fix,
                Ty::List(Box::new(Ty::Fix)),
                Ty::Fix,
            ],
            Ty::List(Box::new(build_ty())),
            Term::Fold {
                cap: REPEAT_CAP,
                list: Box::new(Term::Iota {
                    cap: REPEAT_CAP,
                    count: Box::new(var(4)),
                }),
                init: Box::new(empty_builds.clone()),
                // [i=0, acc=1, reach=2, gates=3, thick=4, h=5, n=6, r=7]
                body: Box::new(Term::Let(
                    // az of this segment's midpoint
                    Box::new(mul(
                        div(
                            add(p1(PrimOp::FixOfInt, var(0)), fx(0.5)),
                            p1(PrimOp::FixOfInt, var(6)),
                        ),
                        fx(360.0),
                    )),
                    // [az=0, i=1, acc=2, reach=3, gates=4, thick=5, h=6, n=7, r=8]
                    Box::new(Term::Let(
                        // is this segment inside a gate?
                        Box::new(Term::Fold {
                            cap: 16,
                            list: Box::new(var(4)),
                            init: Box::new(Term::Bool(false)),
                            // [gate=0, open=1, az=2, i=3, acc=4, reach=5, …]
                            body: Box::new(p2(
                                PrimOp::Or,
                                var(1),
                                p2(PrimOp::FLe, ang_dist(var(2), var(0)), var(5)),
                            )),
                        }),
                        // [gated=0, az=1, i=2, acc=3, reach=4, gates=5, thick=6, h=7, n=8, r=9]
                        Box::new(Term::If(
                            Box::new(var(0)),
                            Box::new(var(3)),
                            Box::new(p2(
                                PrimOp::ListCat,
                                var(3),
                                list(vec![
                                    build(
                                        "box",
                                        vec![
                                            (
                                                "at",
                                                vec3(
                                                    mul(
                                                        var(9),
                                                        p1(
                                                            PrimOp::FSin,
                                                            mul(
                                                                div(var(1), fx(360.0)),
                                                                Term::Fix(FIX_TAU),
                                                            ),
                                                        ),
                                                    ),
                                                    div(var(7), fx(2.0)),
                                                    sub(
                                                        fx(0.0),
                                                        mul(
                                                            var(9),
                                                            p1(
                                                                PrimOp::FCos,
                                                                mul(
                                                                    div(var(1), fx(360.0)),
                                                                    Term::Fix(FIX_TAU),
                                                                ),
                                                            ),
                                                        ),
                                                    ),
                                                ),
                                            ),
                                            // tangent to the ring: ψ = −az
                                            ("yaw", sub(fx(0.0), var(1))),
                                            // chord + a little overlap so the ring reads sealed
                                            (
                                                "w",
                                                add(
                                                    mul(
                                                        mul(fx(2.0), var(9)),
                                                        p1(
                                                            PrimOp::FSin,
                                                            div(
                                                                Term::Fix(FIX_TAU / 2),
                                                                p1(PrimOp::FixOfInt, var(8)),
                                                            ),
                                                        ),
                                                    ),
                                                    fx(0.2),
                                                ),
                                            ),
                                            ("h", var(7)),
                                            ("d", var(6)),
                                            ("material", txt("stone")),
                                            ("name", txt("wall")),
                                        ],
                                    ),
                                    // The coping: a finished top course. Without
                                    // it a wall reads as a slab someone stood up.
                                    build(
                                        "box",
                                        vec![
                                            (
                                                "at",
                                                vec3(
                                                    mul(
                                                        var(9),
                                                        p1(
                                                            PrimOp::FSin,
                                                            mul(
                                                                div(var(1), fx(360.0)),
                                                                Term::Fix(FIX_TAU),
                                                            ),
                                                        ),
                                                    ),
                                                    add(var(7), fx(0.09)),
                                                    sub(
                                                        fx(0.0),
                                                        mul(
                                                            var(9),
                                                            p1(
                                                                PrimOp::FCos,
                                                                mul(
                                                                    div(var(1), fx(360.0)),
                                                                    Term::Fix(FIX_TAU),
                                                                ),
                                                            ),
                                                        ),
                                                    ),
                                                ),
                                            ),
                                            ("yaw", sub(fx(0.0), var(1))),
                                            (
                                                "w",
                                                add(
                                                    mul(
                                                        mul(fx(2.0), var(9)),
                                                        p1(
                                                            PrimOp::FSin,
                                                            div(
                                                                Term::Fix(FIX_TAU / 2),
                                                                p1(PrimOp::FixOfInt, var(8)),
                                                            ),
                                                        ),
                                                    ),
                                                    fx(0.3),
                                                ),
                                            ),
                                            ("h", fx(0.18)),
                                            ("d", add(var(6), fx(0.12))),
                                            ("solid", Term::Bool(false)),
                                            ("material", txt("stone")),
                                            ("name", txt("coping")),
                                        ],
                                    ),
                                ]),
                            )),
                        )),
                    )),
                )),
            },
        ),
    );

    // gated(needs, gates, reach): drop the ones standing in a doorway. A ring
    // of columns and a ring of walls have to agree about where the gates are,
    // or the wall opens politely and a column stands in the gap. (It did. You
    // walk up to a gallery entrance and there is a column in it.) A need's
    // azimuth is recoverable from its facing — a ring faces inward, ψ = −az —
    // so this needs no new information, only the same arithmetic the wall did.
    b.add(
        "gated",
        def(
            vec![
                Ty::List(Box::new(need_ty())),
                Ty::List(Box::new(Ty::Fix)),
                Ty::Fix,
            ],
            Ty::List(Box::new(need_ty())),
            Term::Fold {
                cap: CAP,
                list: Box::new(var(2)),
                init: Box::new(none_of(need("column", vec![]))),
                // [want=0, acc=1, reach=2, gates=3, needs=4]
                body: Box::new(Term::Let(
                    Box::new(sub(fx(0.0), get(var(0), "yaw"))),
                    // [az=0, want=1, acc=2, reach=3, gates=4, needs=5]
                    Box::new(Term::Let(
                        Box::new(Term::Fold {
                            cap: 16,
                            list: Box::new(var(4)),
                            init: Box::new(Term::Bool(false)),
                            // [gate=0, blocked=1, az=2, want=3, acc=4, reach=5, …]
                            body: Box::new(p2(
                                PrimOp::Or,
                                var(1),
                                p2(PrimOp::FLe, ang_dist(var(2), var(0)), var(5)),
                            )),
                        }),
                        // [blocked=0, az=1, want=2, acc=3, reach=4, …]
                        Box::new(Term::If(
                            Box::new(var(0)),
                            Box::new(var(3)),
                            Box::new(p2(PrimOp::ListCat, var(3), list(vec![var(2)]))),
                        )),
                    )),
                )),
            },
        ),
    );

    // enclosure(r, n, h, gate_az, reach): the grounds and the boundary wall.
    // A great building needs somewhere to *be* — the outer ring is what makes
    // the rotunda read as standing in a precinct rather than floating on a
    // disc, and it is the difference between a model and a place.
    b.add(
        "enclosure",
        def(
            vec![Ty::Fix, Ty::Int, Ty::Fix, Ty::Fix, Ty::Fix],
            Ty::List(Box::new(build_ty())),
            // [reach=0, gate_az=1, h=2, n=3, r=4]
            p2(
                PrimOp::ListCat,
                list(vec![build(
                    "disc",
                    vec![
                        // past the wall, so the precinct has an outside
                        ("r", mul(var(4), fx(1.18))),
                        ("h", fx(0.1)),
                        // resting exactly, not bedded: half its own thickness
                        // up, so conformance C7 has nothing to say about it
                        ("at", vec3(fx(0.0), fx(0.05), fx(0.0))),
                        ("material", txt("ground")),
                        ("name", txt("grounds")),
                    ],
                )]),
                b.call(
                    "prefixed",
                    vec![
                        b.call(
                            "wall_ring",
                            vec![var(4), var(3), var(2), fx(0.45), list(vec![var(1)]), var(0)],
                        ),
                        txt("outer "),
                    ],
                ),
            ),
        ),
    );

    // --- figures: the shapes places actually come in -----------------------
    // colonnade(n, radius, height): a ring of columns, sized to the ring so
    // the proportion holds at any scale.
    b.add(
        "colonnade",
        def(
            vec![Ty::Int, Ty::Fix, Ty::Fix],
            Ty::List(Box::new(need_ty())),
            b.call(
                "ring_of",
                vec![
                    list(vec![need(
                        "column",
                        vec![
                            ("h", var(0)),
                            ("w", mul(var(0), fx(0.17))),
                            ("d", mul(var(0), fx(0.17))),
                            ("tol", mul(var(0), fx(0.08))),
                            ("tags", list(vec![txt("load-bearing")])),
                            ("name", txt("column")),
                        ],
                    )]),
                    var(2),
                    var(1),
                ],
            ),
        ),
    );
    // jambs(gates, radius, height, spread): a pair of columns framing every
    // opening. Gating the ring keeps doorways walkable but leaves them
    // *unmarked* — a 45° hole in a colonnade reads as a missing piece rather
    // than an entrance. Two columns on the jambs turn the hole back into a
    // door, and they land where the ring's own rhythm was interrupted.
    {
        // No `Let` for the azimuth, deliberately: binding it would shift every
        // index in the `r` and `h` terms the caller passed in, and a silent
        // off-by-one there gave the first version 14-metre columns (it had
        // grabbed the *spread* argument). Three copies of a cheap term beat
        // one clever binder.
        let column_at = |az: Term, r: Term, h: Term| {
            let rad = |a: Term| mul(div(a, fx(360.0)), Term::Fix(FIX_TAU));
            need(
                "column",
                vec![
                    (
                        "at",
                        vec3(
                            mul(r.clone(), p1(PrimOp::FSin, rad(az.clone()))),
                            fx(0.0),
                            sub(fx(0.0), mul(r, p1(PrimOp::FCos, rad(az.clone())))),
                        ),
                    ),
                    ("yaw", sub(fx(0.0), az)),
                    ("h", h.clone()),
                    ("w", mul(h.clone(), fx(0.17))),
                    ("d", mul(h.clone(), fx(0.17))),
                    ("tol", mul(h, fx(0.08))),
                    ("tags", list(vec![txt("load-bearing")])),
                    ("name", txt("column")),
                ],
            )
        };
        b.add(
            "jambs",
            def(
                vec![Ty::List(Box::new(Ty::Fix)), Ty::Fix, Ty::Fix, Ty::Fix],
                Ty::List(Box::new(need_ty())),
                Term::Fold {
                    cap: 32,
                    list: Box::new(var(3)),
                    init: Box::new(none_of(need("column", vec![]))),
                    // [gate=0, acc=1, spread=2, height=3, radius=4, gates=5]
                    body: Box::new(p2(
                        PrimOp::ListCat,
                        var(1),
                        list(vec![
                            column_at(add(var(0), var(2)), var(4), var(3)),
                            column_at(sub(var(0), var(2)), var(4), var(3)),
                        ]),
                    )),
                },
            ),
        );
    }

    // lamp_ring(n, radius): light as part of the plan, not an afterthought.
    b.add(
        "lamp_ring",
        def(
            vec![Ty::Int, Ty::Fix],
            Ty::List(Box::new(light_ty())),
            Term::Fold {
                cap: REPEAT_CAP,
                list: Box::new(Term::Iota {
                    cap: REPEAT_CAP,
                    count: Box::new(var(1)),
                }),
                init: Box::new(none_of(rec(vec![
                    ("at", vec3(fx(0.0), fx(0.0), fx(0.0))),
                    ("fixture", Term::Bool(true)),
                    ("intensity", fx(1.0)),
                    ("range", fx(9.0)),
                    ("warm", fx(1.0)),
                ]))),
                // [i=0, acc=1, radius=2, n=3]
                // Half a step off the axes: a fixture standing in the
                // entrance sightline is the first thing a visitor sees, and
                // it should never be a lamp post. Offsetting by ½ a step puts
                // the lights between the cardinal directions, where they lift
                // the room instead of blocking it.
                body: Box::new(Term::Let(
                    Box::new(mul(
                        div(
                            add(p1(PrimOp::FixOfInt, var(0)), fx(0.5)),
                            p1(PrimOp::FixOfInt, var(3)),
                        ),
                        Term::Fix(FIX_TAU),
                    )),
                    // [theta=0, i=1, acc=2, radius=3, n=4]
                    Box::new(p2(
                        PrimOp::ListCat,
                        var(2),
                        list(vec![rec(vec![
                            (
                                "at",
                                vec3(
                                    mul(var(3), p1(PrimOp::FSin, var(0))),
                                    fx(2.4),
                                    sub(fx(0.0), mul(var(3), p1(PrimOp::FCos, var(0)))),
                                ),
                            ),
                            ("fixture", Term::Bool(true)),
                            ("intensity", fx(1.1)),
                            ("range", fx(LAMP_RANGE)),
                            ("warm", fx(1.0)),
                        ])]),
                    )),
                )),
            },
        ),
    );

    // --- the whole place ---------------------------------------------------
    // hall(name, radius, height, columns, style, stone, sky):
    // a round hall — floor, colonnade, lamps, a threshold you arrive at and
    // a veil opposite it, with the spawn kept clear of the doorway.
    b.add(
        "hall",
        def(
            vec![
                Ty::Text,
                Ty::Fix,
                Ty::Fix,
                Ty::Int,
                Ty::Text,
                Ty::Text,
                Ty::Text,
            ],
            plan_ty(),
            rec(vec![
                ("name", var(6)),
                ("description", txt("")),
                (
                    "palette",
                    rec(vec![
                        ("style", var(2)),
                        ("stone", var(1)),
                        ("wood", txt("wood")),
                        ("metal", txt("iron")),
                        ("sky", var(0)),
                    ]),
                ),
                ("builds", b.call("ground", vec![var(5)])),
                (
                    "needs",
                    b.call(
                        "join",
                        vec![
                            b.call(
                                "styled",
                                vec![
                                    b.call(
                                        "colonnade",
                                        vec![var(3), mul(var(5), fx(0.82)), var(4)],
                                    ),
                                    var(2),
                                ],
                            ),
                            // A threshold arch at the far side, framing the veil.
                            list(vec![need(
                                "arch",
                                vec![
                                    ("at", vec3(fx(0.0), fx(0.0), mul(var(5), fx(-0.92)))),
                                    ("yaw", fx(0.0)),
                                    ("w", fx(3.0)),
                                    ("h", mul(var(4), fx(0.8))),
                                    ("d", fx(0.6)),
                                    ("tol", fx(0.5)),
                                    ("style", var(2)),
                                    ("tags", list(vec![txt("threshold")])),
                                    ("name", txt("threshold")),
                                ],
                            )]),
                        ],
                    ),
                ),
                (
                    "veils",
                    list(vec![rec(vec![
                        ("at", vec3(fx(0.0), fx(1.4), mul(var(5), fx(-0.9)))),
                        ("yaw", fx(0.0)),
                        ("to", txt("")),
                        ("label", txt("Onward")),
                    ])]),
                ),
                (
                    "spawns",
                    list(vec![rec(vec![
                        ("at", vec3(fx(0.0), fx(0.0), mul(var(5), fx(0.72)))),
                        ("yaw", fx(180.0)),
                        ("name", txt("entry")),
                    ])]),
                ),
                (
                    "lights",
                    b.call("lamp_ring", vec![int(4), mul(var(5), fx(0.55))]),
                ),
                (
                    "signs",
                    none_of(rec(vec![
                        ("at", vec3(fx(0.0), fx(0.0), fx(0.0))),
                        ("h", fx(1.5)),
                        ("text", txt("")),
                        ("w", fx(2.0)),
                        ("yaw", fx(0.0)),
                    ])),
                ),
            ]),
        ),
    );

    // courtyard(name, size, style, stone, sky): a square yard — paved ground,
    // a colonnade down two sides, a well at the focus, scattered planting.
    b.add(
        "courtyard",
        def(
            vec![Ty::Text, Ty::Fix, Ty::Text, Ty::Text, Ty::Text],
            plan_ty(),
            rec(vec![
                ("name", var(4)),
                ("description", txt("")),
                (
                    "palette",
                    rec(vec![
                        ("style", var(2)),
                        ("stone", var(1)),
                        ("wood", txt("wood")),
                        ("metal", txt("iron")),
                        ("sky", var(0)),
                    ]),
                ),
                (
                    "builds",
                    list(vec![rec(vec![
                        ("shape", txt("slab")),
                        ("at", vec3(fx(0.0), fx(0.02), fx(0.0))),
                        ("yaw", fx(0.0)),
                        ("r", fx(0.0)),
                        ("w", var(3)),
                        ("d", var(3)),
                        ("h", fx(0.12)),
                        ("material", txt("stone")),
                        ("solid", Term::Bool(true)),
                        ("name", txt("paving")),
                    ])]),
                ),
                (
                    "needs",
                    b.call(
                        "join",
                        vec![
                            b.call(
                                "join",
                                vec![
                                    // two facing colonnades
                                    b.call(
                                        "row_of",
                                        vec![
                                            list(vec![need(
                                                "column",
                                                vec![
                                                    ("h", fx(4.2)),
                                                    ("w", fx(0.7)),
                                                    ("d", fx(0.7)),
                                                    ("tol", fx(0.4)),
                                                    ("style", var(2)),
                                                    ("name", txt("column")),
                                                ],
                                            )]),
                                            int(5),
                                            mul(var(3), fx(-0.36)),
                                            mul(var(3), fx(-0.36)),
                                            fx(0.0),
                                            mul(var(3), fx(0.18)),
                                            fx(90.0),
                                        ],
                                    ),
                                    b.call(
                                        "row_of",
                                        vec![
                                            list(vec![need(
                                                "column",
                                                vec![
                                                    ("h", fx(4.2)),
                                                    ("w", fx(0.7)),
                                                    ("d", fx(0.7)),
                                                    ("tol", fx(0.4)),
                                                    ("style", var(2)),
                                                    ("name", txt("column")),
                                                ],
                                            )]),
                                            int(5),
                                            mul(var(3), fx(0.36)),
                                            mul(var(3), fx(-0.36)),
                                            fx(0.0),
                                            mul(var(3), fx(0.18)),
                                            fx(270.0),
                                        ],
                                    ),
                                ],
                            ),
                            // the focus: something to walk toward
                            list(vec![need(
                                "vessel",
                                vec![
                                    ("at", vec3(fx(0.0), fx(0.0), fx(0.0))),
                                    ("w", fx(1.2)),
                                    ("h", fx(1.2)),
                                    ("d", fx(1.2)),
                                    ("tol", fx(0.6)),
                                    ("style", var(2)),
                                    ("name", txt("focus")),
                                ],
                            )]),
                        ],
                    ),
                ),
                (
                    "veils",
                    list(vec![rec(vec![
                        ("at", vec3(fx(0.0), fx(1.4), mul(var(3), fx(-0.46)))),
                        ("yaw", fx(0.0)),
                        ("to", txt("")),
                        ("label", txt("Onward")),
                    ])]),
                ),
                (
                    "spawns",
                    list(vec![rec(vec![
                        ("at", vec3(fx(0.0), fx(0.0), mul(var(3), fx(0.38)))),
                        ("yaw", fx(180.0)),
                        ("name", txt("entry")),
                    ])]),
                ),
                (
                    "lights",
                    b.call("lamp_ring", vec![int(4), mul(var(3), fx(0.3))]),
                ),
                (
                    "signs",
                    none_of(rec(vec![
                        ("at", vec3(fx(0.0), fx(0.0), fx(0.0))),
                        ("h", fx(1.5)),
                        ("text", txt("")),
                        ("w", fx(2.0)),
                        ("yaw", fx(0.0)),
                    ])),
                ),
            ]),
        ),
    );

    // wing(az, from_r, len, width, height): a gallery corridor radiating from
    // a rotunda — floor, two side walls, an end wall, and a lintel over the
    // gate. Every position and every turn comes from the corridor frame
    // (`arch::Corridor`'s law, in Weft): along = 90 − az, across = −az. Wings
    // were the museum's hardest geometry to get right by hand; here it is
    // written once.
    b.add(
        "wing",
        def(
            vec![Ty::Fix, Ty::Fix, Ty::Fix, Ty::Fix, Ty::Fix],
            Ty::List(Box::new(build_ty())),
            // [h=0, width=1, len=2, from_r=3, az=4]
            Term::Let(
                // sin az
                Box::new(p1(
                    PrimOp::FSin,
                    mul(div(var(4), fx(360.0)), Term::Fix(FIX_TAU)),
                )),
                // [s=0, h=1, width=2, len=3, from_r=4, az=5]
                Box::new(Term::Let(
                    // cos az
                    Box::new(p1(
                        PrimOp::FCos,
                        mul(div(var(5), fx(360.0)), Term::Fix(FIX_TAU)),
                    )),
                    // [c=0, s=1, h=2, width=3, len=4, from_r=5, az=6]
                    Box::new(Term::Let(
                        // mid: the corridor's centre, from_r + len/2 out
                        Box::new(add(var(5), div(var(4), fx(2.0)))),
                        // [mid=0, c=1, s=2, h=3, width=4, len=5, from_r=6, az=7]
                        Box::new(list(vec![
                            // floor: X-long down the corridor
                            build(
                                "box",
                                vec![
                                    (
                                        "at",
                                        vec3(
                                            mul(var(0), var(2)),
                                            fx(0.03),
                                            sub(fx(0.0), mul(var(0), var(1))),
                                        ),
                                    ),
                                    ("yaw", sub(fx(90.0), var(7))),
                                    ("w", var(5)),
                                    ("h", fx(0.12)),
                                    ("d", var(4)),
                                    ("material", txt("floor")),
                                    ("name", txt("wing floor")),
                                ],
                            ),
                            // side walls, offset across the corridor
                            build(
                                "box",
                                vec![
                                    (
                                        "at",
                                        vec3(
                                            add(
                                                mul(var(0), var(2)),
                                                mul(div(var(4), fx(2.0)), var(1)),
                                            ),
                                            div(var(3), fx(2.0)),
                                            add(
                                                sub(fx(0.0), mul(var(0), var(1))),
                                                mul(div(var(4), fx(2.0)), var(2)),
                                            ),
                                        ),
                                    ),
                                    ("yaw", sub(fx(90.0), var(7))),
                                    ("w", var(5)),
                                    ("h", var(3)),
                                    ("d", fx(0.4)),
                                    ("material", txt("stone")),
                                    ("name", txt("wing wall")),
                                ],
                            ),
                            build(
                                "box",
                                vec![
                                    (
                                        "at",
                                        vec3(
                                            sub(
                                                mul(var(0), var(2)),
                                                mul(div(var(4), fx(2.0)), var(1)),
                                            ),
                                            div(var(3), fx(2.0)),
                                            sub(
                                                sub(fx(0.0), mul(var(0), var(1))),
                                                mul(div(var(4), fx(2.0)), var(2)),
                                            ),
                                        ),
                                    ),
                                    ("yaw", sub(fx(90.0), var(7))),
                                    ("w", var(5)),
                                    ("h", var(3)),
                                    ("d", fx(0.4)),
                                    ("name", txt("wing wall")),
                                ],
                            ),
                            // end wall, closing the far end
                            build(
                                "box",
                                vec![
                                    (
                                        "at",
                                        vec3(
                                            mul(add(var(6), var(5)), var(2)),
                                            div(var(3), fx(2.0)),
                                            sub(fx(0.0), mul(add(var(6), var(5)), var(1))),
                                        ),
                                    ),
                                    ("yaw", sub(fx(0.0), var(7))),
                                    ("w", add(var(4), fx(0.8))),
                                    ("h", var(3)),
                                    ("d", fx(0.4)),
                                    ("material", txt("stone")),
                                    ("name", txt("wing end")),
                                ],
                            ),
                            // lintel over the gate, spanning the opening
                            build(
                                "box",
                                vec![
                                    (
                                        "at",
                                        vec3(
                                            mul(add(var(6), fx(0.3)), var(2)),
                                            add(var(3), fx(0.15)),
                                            sub(fx(0.0), mul(add(var(6), fx(0.3)), var(1))),
                                        ),
                                    ),
                                    ("yaw", sub(fx(0.0), var(7))),
                                    ("w", add(var(4), fx(1.2))),
                                    ("h", fx(0.32)),
                                    ("d", fx(0.5)),
                                    ("material", txt("accent")),
                                    ("solid", Term::Bool(false)),
                                    ("name", txt("lintel")),
                                ],
                            ),
                        ])),
                    )),
                )),
            ),
        ),
    );

    // wing_az(i): where the i-th wing points. Wings alternate either side of
    // the entrance and step outward — +55, −55, +125, −125 … — so a hall of
    // two wings and a hall of six are the same design at different sizes,
    // and neither ever puts a gallery where the door is.
    b.add(
        "wing_az",
        def(
            vec![Ty::Int],
            Ty::Fix,
            Term::Let(
                // pair index: how far out from the entrance axis
                Box::new(p2(PrimOp::Div, var(0), int(2))),
                // [pair=0, i=1]
                Box::new(Term::Let(
                    Box::new(add(fx(55.0), mul(p1(PrimOp::FixOfInt, var(0)), fx(70.0)))),
                    // [mag=0, pair=1, i=2]
                    Box::new(Term::If(
                        // even index → the +side, odd → the −side
                        Box::new(p2(
                            PrimOp::EqInt,
                            p2(PrimOp::Sub, var(2), p2(PrimOp::Mul, var(1), int(2))),
                            int(0),
                        )),
                        Box::new(var(0)),
                        Box::new(sub(fx(0.0), var(0))),
                    )),
                )),
            ),
        ),
    );

    // museum(name, radius, height, columns, wing_len, titles, style, stone, sky):
    //
    // The encyclopedia's own figure. A walled, colonnaded rotunda with a
    // pedestal at its focus, and **one gallery wing per title the brief
    // supplies** — each with its name over the gate, a reading board inside,
    // and a veil at the far end. The architecture follows the content: two
    // sections make a small hall, six make a great one, and neither is a
    // different program.
    // The wall's gates: the entrance, plus one mouth per wing — derived from
    // the titles, so a two-section museum is not a six-gate wall with four
    // gaps opening onto nothing.
    let gate_azimuths = |titles: Term| -> Term {
        p2(
            PrimOp::ListCat,
            list(vec![fx(180.0)]),
            Term::Fold {
                cap: 8,
                list: Box::new(Term::Iota {
                    cap: 8,
                    count: Box::new(p1(PrimOp::Len, titles)),
                }),
                init: Box::new(none_of(fx(0.0))),
                // [i=0, acc=1, …]
                body: Box::new(p2(
                    PrimOp::ListCat,
                    var(1),
                    list(vec![b.call("wing_az", vec![var(0)])]),
                )),
            },
        )
    };
    // A wing's contribution, folded per title: the corridor, its named gate,
    // its reading board, and the veil that ends it.
    b.add(
        "museum",
        def(
            vec![
                Ty::Text,
                Ty::Fix,
                Ty::Fix,
                Ty::Int,
                Ty::Fix,
                Ty::List(Box::new(Ty::Text)),
                Ty::Text,
                Ty::Text,
                Ty::Text,
            ],
            plan_ty(),
            // [sky=0, stone=1, style=2, titles=3, wing_len=4, columns=5,
            //  height=6, radius=7, name=8]
            rec(vec![
                ("name", var(8)),
                ("description", txt("")),
                (
                    "palette",
                    rec(vec![
                        ("style", var(2)),
                        ("stone", var(1)),
                        ("wood", txt("wood")),
                        ("metal", txt("iron")),
                        ("sky", var(0)),
                    ]),
                ),
                ("builds", {
                    // floor + gated wall ring, then a corridor per wing.
                    let base = b.call(
                        "join_builds",
                        vec![
                            b.call(
                                "join_builds",
                                vec![
                                    // the precinct: grounds and a boundary
                                    // wall, open on the entrance axis
                                    b.call(
                                        "enclosure",
                                        vec![
                                            // clear of the wings, with
                                            // room to walk round them
                                            add(add(var(7), var(4)), fx(8.0)),
                                            int(32),
                                            fx(2.2),
                                            fx(180.0),
                                            // A ring's segments are offset by
                                            // half a step, so a gate on an axis
                                            // falls BETWEEN two of them: the
                                            // reach has to clear that half-step
                                            // (5.6° at 32 segments) or the wall
                                            // closes over the way in. It did —
                                            // the precinct had no door, and you
                                            // only learn that by walking up to
                                            // it.
                                            fx(6.5),
                                        ],
                                    ),
                                    b.call("ground", vec![var(7)]),
                                ],
                            ),
                            b.call(
                                "wall_ring",
                                vec![
                                    var(7),
                                    var(5),
                                    // The wall comes up to about 70 % of
                                    // the order. Level with the columns it
                                    // hides them, and the room loses the
                                    // rhythm that makes it a room.
                                    mul(var(6), fx(0.7)),
                                    fx(0.5),
                                    // 13° of reach opens two 15° segments
                                    // per gate — a 30° mouth, which at
                                    // r = 16 is the 6.8 m a wing is wide.
                                    gate_azimuths(var(3)),
                                    fx(13.0),
                                ],
                            ),
                        ],
                    );
                    let with_plinth = b.call(
                        "join_builds",
                        vec![
                            base,
                            list(vec![build(
                                "cylinder",
                                vec![
                                    ("at", vec3(fx(0.0), fx(0.3), fx(0.0))),
                                    ("r", fx(1.6)),
                                    ("h", fx(0.6)),
                                    ("material", txt("floor")),
                                    ("name", txt("pedestal")),
                                ],
                            )]),
                        ],
                    );
                    // the walk in, so arrivals land on stone
                    let with_walk = b.call(
                        "join_builds",
                        vec![
                            with_plinth,
                            list(vec![build(
                                "box",
                                vec![
                                    ("at", vec3(fx(0.0), fx(0.04), add(var(7), fx(7.0)))),
                                    ("w", fx(4.0)),
                                    ("h", fx(0.12)),
                                    ("d", fx(14.0)),
                                    ("solid", Term::Bool(false)),
                                    ("name", txt("walkway")),
                                ],
                            )]),
                        ],
                    );
                    // one corridor per title
                    Term::Fold {
                        cap: 8,
                        list: Box::new(Term::Iota {
                            cap: 8,
                            count: Box::new(p1(PrimOp::Len, var(3))),
                        }),
                        init: Box::new(with_walk),
                        // [i=0, acc=1, sky=2, stone=3, style=4, titles=5,
                        //  wing_len=6, columns=7, height=8, radius=9, name=10]
                        body: Box::new(b.call(
                            "join_builds",
                            vec![
                                var(1),
                                b.call(
                                    "wing",
                                    vec![
                                        b.call("wing_az", vec![var(0)]),
                                        var(9),
                                        var(6),
                                        fx(6.8),
                                        // the same wall height as the
                                        // rotunda: a gallery taller than
                                        // the room it leaves is a mistake
                                        // you feel before you see
                                        mul(var(8), fx(0.7)),
                                    ],
                                ),
                            ],
                        )),
                    }
                }),
                (
                    "needs",
                    b.call(
                        "join",
                        vec![
                            b.call(
                                "gated",
                                vec![
                                    b.call(
                                        "styled",
                                        vec![
                                            b.call(
                                                "colonnade",
                                                // On the wall line, half a metre
                                                // proud of its face: engaged
                                                // columns, the way a colonnade
                                                // and a wall actually meet.
                                                vec![var(5), add(var(7), fx(0.45)), var(6)],
                                            ),
                                            var(2),
                                        ],
                                    ),
                                    gate_azimuths(var(3)),
                                    fx(13.0),
                                ],
                            ),
                            b.call(
                                "join",
                                vec![
                                    // every gate framed by its own pair
                                    b.call(
                                        "styled",
                                        vec![
                                            b.call(
                                                "jambs",
                                                vec![
                                                    gate_azimuths(var(3)),
                                                    add(var(7), fx(0.45)),
                                                    var(6),
                                                    fx(14.0),
                                                ],
                                            ),
                                            var(2),
                                        ],
                                    ),
                                    b.call(
                                        "join",
                                        vec![
                                            list(vec![need(
                                                "vessel",
                                                vec![
                                                    ("at", vec3(fx(0.0), fx(0.6), fx(0.0))),
                                                    ("w", fx(1.2)),
                                                    ("h", fx(1.6)),
                                                    ("d", fx(1.2)),
                                                    ("tol", fx(0.8)),
                                                    ("style", var(2)),
                                                    ("name", txt("subject")),
                                                    ("solid", Term::Bool(false)),
                                                ],
                                            )]),
                                            // a sparser outer colonnade: the precinct
                                            // wall gets a rhythm too, at a scale you
                                            // read from across the grounds
                                            b.call(
                                                "labelled",
                                                vec![
                                                    b.call(
                                                        "gated",
                                                        vec![
                                                            b.call(
                                                                "styled",
                                                                vec![
                                                                    b.call(
                                                                        "colonnade",
                                                                        vec![
                                                                            int(12),
                                                                            add(
                                                                                add(
                                                                                    add(
                                                                                        var(7),
                                                                                        var(4),
                                                                                    ),
                                                                                    fx(8.0),
                                                                                ),
                                                                                fx(0.45),
                                                                            ),
                                                                            mul(var(6), fx(0.9)),
                                                                        ],
                                                                    ),
                                                                    var(2),
                                                                ],
                                                            ),
                                                            list(vec![fx(180.0)]),
                                                            fx(9.0),
                                                        ],
                                                    ),
                                                    txt("outer "),
                                                ],
                                            ),
                                        ],
                                    ),
                                ],
                            ),
                        ],
                    ),
                ),
                (
                    "veils",
                    // one at the end of each wing, facing back down it
                    Term::Fold {
                        cap: 8,
                        list: Box::new(var(3)),
                        init: Box::new(none_of(rec(vec![
                            ("at", vec3(fx(0.0), fx(0.0), fx(0.0))),
                            ("label", txt("")),
                            ("to", txt("")),
                            ("yaw", fx(0.0)),
                        ]))),
                        // [title=0, acc=1, …, wing_len=6, …, radius=9]
                        body: Box::new(Term::Let(
                            Box::new(b.call("wing_az", vec![p1(PrimOp::Len, var(1))])),
                            // [az=0, i=1, acc=2, sky=3, stone=4, style=5,
                            //  titles=6, wing_len=7, columns=8, height=9, radius=10]
                            Box::new(Term::Let(
                                Box::new(add(var(10), sub(var(7), fx(1.2)))),
                                // [reach=0, az=1, i=2, acc=3, …]
                                Box::new(p2(
                                    PrimOp::ListCat,
                                    var(3),
                                    list(vec![rec(vec![
                                        (
                                            "at",
                                            vec3(
                                                mul(
                                                    var(0),
                                                    p1(
                                                        PrimOp::FSin,
                                                        mul(
                                                            div(var(1), fx(360.0)),
                                                            Term::Fix(FIX_TAU),
                                                        ),
                                                    ),
                                                ),
                                                fx(1.4),
                                                sub(
                                                    fx(0.0),
                                                    mul(
                                                        var(0),
                                                        p1(
                                                            PrimOp::FCos,
                                                            mul(
                                                                div(var(1), fx(360.0)),
                                                                Term::Fix(FIX_TAU),
                                                            ),
                                                        ),
                                                    ),
                                                ),
                                            ),
                                        ),
                                        // face back down the corridor
                                        ("yaw", add(fx(180.0), sub(fx(0.0), var(1)))),
                                        ("to", txt("")),
                                        // the door says where it goes
                                        ("label", var(2)),
                                    ])]),
                                )),
                            )),
                        )),
                    },
                ),
                (
                    "spawns",
                    list(vec![rec(vec![
                        ("at", vec3(fx(0.0), fx(0.0), mul(var(7), fx(0.62)))),
                        ("yaw", fx(180.0)),
                        ("name", txt("entry")),
                    ])]),
                ),
                (
                    "lights",
                    p2(
                        PrimOp::ListCat,
                        b.call("lamp_ring", vec![int(6), mul(var(7), fx(0.62))]),
                        // A corridor lit only from the room it leaves is a
                        // corridor nobody walks down.
                        Term::Fold {
                            cap: 8,
                            list: Box::new(Term::Iota {
                                cap: 8,
                                count: Box::new(p1(PrimOp::Len, var(3))),
                            }),
                            init: Box::new(none_of(rec(vec![
                                ("at", vec3(fx(0.0), fx(0.0), fx(0.0))),
                                ("fixture", Term::Bool(true)),
                                ("intensity", fx(1.0)),
                                ("range", fx(LAMP_RANGE)),
                                ("warm", fx(1.0)),
                            ]))),
                            // [i=0, acc=1, sky=2, stone=3, style=4, titles=5,
                            //  wing_len=6, columns=7, height=8, radius=9, name=10]
                            body: Box::new(Term::Let(
                                Box::new(mul(
                                    div(b.call("wing_az", vec![var(0)]), fx(360.0)),
                                    Term::Fix(FIX_TAU),
                                )),
                                // [theta=0, i=1, acc=2, …, wing_len=7, …, radius=10]
                                Box::new(Term::Let(
                                    Box::new(add(var(10), mul(var(7), fx(0.55)))),
                                    // [d=0, theta=1, i=2, acc=3, …]
                                    Box::new(p2(
                                        PrimOp::ListCat,
                                        var(3),
                                        list(vec![rec(vec![
                                            (
                                                "at",
                                                vec3(
                                                    mul(var(0), p1(PrimOp::FSin, var(1))),
                                                    fx(2.4),
                                                    sub(
                                                        fx(0.0),
                                                        mul(var(0), p1(PrimOp::FCos, var(1))),
                                                    ),
                                                ),
                                            ),
                                            ("fixture", Term::Bool(true)),
                                            ("intensity", fx(1.0)),
                                            ("range", fx(LAMP_RANGE)),
                                            ("warm", fx(1.0)),
                                        ])]),
                                    )),
                                )),
                            )),
                        },
                    ),
                ),
                (
                    "signs",
                    // Each wing's name over its gate, facing the rotunda —
                    // read on approach, which is the only moment it helps.
                    Term::Fold {
                        cap: 8,
                        list: Box::new(var(3)),
                        init: Box::new(none_of(rec(vec![
                            ("at", vec3(fx(0.0), fx(0.0), fx(0.0))),
                            ("h", fx(1.5)),
                            ("text", txt("")),
                            ("w", fx(2.0)),
                            ("yaw", fx(0.0)),
                        ]))),
                        // Fold with a counting accumulator would need a record;
                        // instead the index rides in the accumulator's length.
                        // [title=0, acc=1, sky=2, stone=3, style=4, titles=5,
                        //  wing_len=6, columns=7, height=8, radius=9, name=10]
                        body: Box::new(Term::Let(
                            Box::new(b.call("wing_az", vec![p1(PrimOp::Len, var(1))])),
                            // [az=0, title=1, acc=2, …, radius=10]
                            Box::new(p2(
                                PrimOp::ListCat,
                                var(2),
                                list(vec![rec(vec![
                                    (
                                        "at",
                                        vec3(
                                            mul(
                                                add(var(10), fx(0.5)),
                                                p1(
                                                    PrimOp::FSin,
                                                    mul(div(var(0), fx(360.0)), Term::Fix(FIX_TAU)),
                                                ),
                                            ),
                                            add(mul(var(9), fx(0.7)), fx(0.45)),
                                            sub(
                                                fx(0.0),
                                                mul(
                                                    add(var(10), fx(0.5)),
                                                    p1(
                                                        PrimOp::FCos,
                                                        mul(
                                                            div(var(0), fx(360.0)),
                                                            Term::Fix(FIX_TAU),
                                                        ),
                                                    ),
                                                ),
                                            ),
                                        ),
                                    ),
                                    // face the rotunda: ψ = −az
                                    ("yaw", sub(fx(0.0), var(0))),
                                    ("text", var(1)),
                                    ("w", fx(3.6)),
                                    ("h", fx(0.8)),
                                ])]),
                            )),
                        )),
                    },
                ),
            ]),
        ),
    );

    let names: Vec<String> = b.by_name.keys().cloned().collect();
    let exports: Vec<(&str, usize)> = names.iter().map(|n| (n.as_str(), b.idx(n))).collect();
    Package::build("weft-draft", b.defs.clone(), exports).expect("the drafting library builds")
}

/// Lamp reach, mirroring `infinite_manifest::plan::metric::LAMP_SPACING`'s
/// companion. The language crate does not depend on the manifest format, so
/// the number is written in both places deliberately — a change to either
/// shows up as a diff instead of drifting quietly.
const LAMP_RANGE: f32 = 9.0;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{eval_call, pack::link, Value};

    fn call(pkg: &Package, name: &str, args: Vec<Value>) -> Value {
        let hash = pkg
            .export(name)
            .unwrap_or_else(|| panic!("export '{name}'"));
        let entry = pkg.defs.get(&hash).cloned().expect("def");
        let module = link(&[pkg.clone()], vec![entry], 0).expect("links");
        eval_call(&module, module.entry, args, 60_000_000)
            .unwrap_or_else(|e| panic!("'{name}' failed: {e:?}"))
            .value
    }
    fn f(v: f32) -> Value {
        Value::Fix((v * FIX_SCALE as f32) as i64)
    }
    fn t(s: &str) -> Value {
        Value::Text(s.into())
    }
    /// The same seam chisel uses: Weft values cross to JSON, and the plan
    /// format reads them back.
    fn to_json(v: &Value) -> serde_json::Value {
        match v {
            Value::Int(i) => serde_json::Value::from(*i),
            Value::Fix(raw) => serde_json::Value::from(*raw as f64 / FIX_SCALE as f64),
            Value::Bool(b) => serde_json::Value::Bool(*b),
            Value::Text(s) => serde_json::Value::String(s.clone()),
            Value::List(xs) => serde_json::Value::Array(xs.iter().map(to_json).collect()),
            Value::Rec(fs) => {
                serde_json::Value::Object(fs.iter().map(|(k, x)| (k.clone(), to_json(x))).collect())
            }
            Value::Action { .. } => serde_json::Value::Null,
        }
    }

    #[test]
    fn the_drafting_library_verifies() {
        let pkg = package();
        pkg.verify()
            .expect("every figure type-checks and terminates");
        for must in [
            "need",
            "join",
            "ring_of",
            "row_of",
            "colonnade",
            "hall",
            "courtyard",
        ] {
            assert!(pkg.exports.contains_key(must), "exports {must}");
        }
    }

    #[test]
    fn a_hall_lays_itself_out_soundly() {
        let pkg = package();
        let v = call(
            &pkg,
            "hall",
            vec![
                t("Test Hall"),
                f(14.0),
                f(5.2),
                Value::Int(12),
                t("classical"),
                t("marble"),
                t("dusk"),
            ],
        );
        let json = to_json(&v);
        let plan: infinite_manifest::plan::Plan =
            serde_json::from_value(json).expect("a hall is a plan");
        assert_eq!(plan.name, "Test Hall");
        assert_eq!(plan.palette.style, "classical");
        // Twelve columns on the ring, plus the threshold arch.
        assert_eq!(plan.needs.iter().filter(|n| n.kind == "column").count(), 12);
        assert!(plan.needs.iter().any(|n| n.kind == "arch"));
        // Columns sit on the ring, sized to it, and face the centre.
        for n in plan.needs.iter().filter(|n| n.kind == "column") {
            let r = (n.at[0] * n.at[0] + n.at[2] * n.at[2]).sqrt();
            assert!((r - 14.0 * 0.82).abs() < 0.2, "on the ring: {r}");
            assert!((n.h - 5.2).abs() < 0.01, "height {}", n.h);
            assert_eq!(n.style, "classical", "the palette reached the needs");
        }
        // And the layout passes its own checks — floor, light, clear spawn.
        assert!(plan.check().is_empty(), "sound hall: {:?}", plan.check());
    }

    #[test]
    fn a_museum_hall_drafts_the_architecture_the_generator_hand_wrote() {
        let pkg = package();
        let titles = ["Ascent", "Empire", "Exile", "Legacy"];
        let v = call(
            &pkg,
            "museum",
            vec![
                t("Napoleon"),
                f(16.0),
                f(3.8),
                Value::Int(24),
                f(18.0),
                Value::List(titles.iter().map(|s| t(s)).collect()),
                t("classical"),
                t("marble"),
                t("dusk"),
            ],
        );
        let plan: infinite_manifest::plan::Plan =
            serde_json::from_value(to_json(&v)).expect("a museum is a plan");
        // The rotunda: a floor, a walled ring with openings, four wings.
        assert!(plan.builds.iter().any(|b| b.shape == "disc"), "a floor");
        let walls = plan.builds.iter().filter(|b| b.name == "wall").count();
        assert!(
            (12..=20).contains(&walls),
            "five gates open two segments each, leaving most of the ring standing; got {walls}"
        );
        assert_eq!(
            plan.builds
                .iter()
                .filter(|b| b.name == "wing floor")
                .count(),
            4
        );
        assert_eq!(
            plan.builds.iter().filter(|b| b.name == "wing wall").count(),
            8
        );
        assert_eq!(plan.builds.iter().filter(|b| b.name == "lintel").count(), 4);
        // Wall segments stand ON the ring, tangent to it.
        for w in plan.builds.iter().filter(|b| b.name == "wall") {
            let r = (w.at[0] * w.at[0] + w.at[2] * w.at[2]).sqrt();
            assert!((r - 16.0).abs() < 0.05, "on the ring: {r}");
        }
        // The gates are real holes: nothing stands across the entrance (az
        // 180, +z) or across a wing mouth (az ±55, ±125).
        for gate in [180.0f32, 55.0, -55.0, 125.0, -125.0] {
            let rad = gate.to_radians();
            let (gx, gz) = (16.0 * rad.sin(), -16.0 * rad.cos());
            let nearest = plan
                .builds
                .iter()
                .filter(|b| b.name == "wall")
                .map(|b| ((b.at[0] - gx).powi(2) + (b.at[2] - gz).powi(2)).sqrt())
                .fold(f32::MAX, f32::min);
            assert!(
                nearest > 3.0,
                "gate at {gate}° is open (nearest wall {nearest:.1} m)"
            );
        }
        // A veil at the end of every wing, and the colonnade inside the walls.
        assert_eq!(plan.veils.len(), 4);
        // Not "24 columns" — *no column in a doorway*. A gated ring drops the
        // ones that would stand in the opening, so the count follows from the
        // gates, and pinning it would only pin the bug back in.
        let cols: Vec<_> = plan.needs.iter().filter(|n| n.name == "column").collect();
        assert!(
            (8..=24).contains(&cols.len()),
            "a colonnade remains: {}",
            cols.len()
        );
        for gate in [180.0f32, 55.0, -55.0, 125.0, -125.0] {
            let rad = gate.to_radians();
            let (gx, gz) = (16.45 * rad.sin(), -16.45 * rad.cos());
            let nearest = cols
                .iter()
                .map(|c| ((c.at[0] - gx).powi(2) + (c.at[2] - gz).powi(2)).sqrt())
                .fold(f32::MAX, f32::min);
            assert!(
                nearest > 3.0,
                "gate at {gate}° is walkable (nearest column {nearest:.1} m)"
            );
        }
        // The precinct: grounds, a boundary wall, and a colonnade at a scale
        // you read from across it.
        assert_eq!(
            plan.needs
                .iter()
                .filter(|n| n.name == "outer column")
                .count(),
            12
        );
        assert!(
            plan.builds.iter().any(|b| b.name == "grounds"),
            "somewhere to stand"
        );
        let outer = plan
            .builds
            .iter()
            .filter(|b| b.name == "outer wall")
            .count();
        assert!(
            (28..=32).contains(&outer),
            "a wall around the grounds; got {outer}"
        );
        for w in plan.builds.iter().filter(|b| b.name == "outer wall") {
            let r = (w.at[0] * w.at[0] + w.at[2] * w.at[2]).sqrt();
            // Weft's sine is Bhaskara's, ~0.2 % — good enough to build with,
            // and the tolerance says so out loud rather than pretending.
            // radius + wing + 8 m of walking room = 42, the same figure the
            // hand-written hall arrived at — derived here instead of chosen.
            assert!((r - 42.0).abs() < 0.15, "on the outer ring: {r}");
        }
        assert!(
            plan.needs.iter().any(|n| n.name == "subject"),
            "something at the focus"
        );
        // The museum says its own words: one named gate per section, hung over
        // that section's mouth and turned to face the rotunda you read it from.
        assert_eq!(plan.signs.len(), 4, "a name over every gate");
        for (i, title) in titles.iter().enumerate() {
            let sign = plan.signs.iter().find(|s| s.text == *title).expect(title);
            let az = if i % 2 == 0 {
                55.0 + 70.0 * (i / 2) as f32
            } else {
                -(55.0 + 70.0 * (i / 2) as f32)
            };
            let rad: f32 = az.to_radians();
            let (gx, gz) = (16.5 * rad.sin(), -16.5 * rad.cos());
            assert!(
                (sign.at[0] - gx).abs() < 0.05 && (sign.at[2] - gz).abs() < 0.05,
                "'{title}' hangs over its own gate, not someone else's: {:?}",
                sign.at
            );
            assert!(
                (sign.yaw - -az).abs() < 0.05,
                "read from inside the rotunda"
            );
        }
        // Copings cap the wall, and the walk in is stone under the arrival.
        assert!(
            plan.builds.iter().any(|b| b.name == "walkway"),
            "arrivals land on stone"
        );
        assert_eq!(
            plan.builds.iter().filter(|b| b.name == "coping").count(),
            walls,
            "every standing segment is capped"
        );
        assert!(plan.check().is_empty(), "sound museum: {:?}", plan.check());
    }

    #[test]
    fn a_courtyard_lays_itself_out_soundly() {
        let pkg = package();
        let v = call(
            &pkg,
            "courtyard",
            vec![t("Yard"), f(24.0), t("rustic"), t("sandstone"), t("day")],
        );
        let plan: infinite_manifest::plan::Plan =
            serde_json::from_value(to_json(&v)).expect("a courtyard is a plan");
        assert_eq!(plan.needs.iter().filter(|n| n.kind == "column").count(), 10);
        assert!(
            plan.needs.iter().any(|n| n.name == "focus"),
            "something to walk toward"
        );
        assert!(plan.check().is_empty(), "sound yard: {:?}", plan.check());
    }
}
