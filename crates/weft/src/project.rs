//! **Projection** — the human-readable rendering of a Weft module.
//!
//! Weft has no source text: the graph is the ground truth. But *audit is a
//! right* (spec §1.1) — commerce and Passports demand that a person can always
//! read what a behavior does. A projection is a faithful, derived view:
//! generated from the verified artifact on demand, never stored, never
//! written. Local binders get generated names (`a`, `b`, `c`… by depth) since
//! the code carries none — alpha-equivalence made visible.
//!
//! The projection is **deterministic**: same module, same text, always — so a
//! rendering can be cached, diffed, and cited by the module's hash.

use std::fmt::Write;

use crate::{Def, EffectKind, Module, PrimOp, Term, Ty};

/// Render a whole module: entry first, then every other definition, each
/// headed by its full hash.
pub fn module(m: &Module) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "weft module · entry {}", m.entry);
    let mut order: Vec<_> = m.defs.keys().collect();
    order.sort_by_key(|h| **h != m.entry); // entry first, then hash order
    for h in order {
        let _ = writeln!(out, "\n{}", def(&m.defs[h]));
        let _ = writeln!(out, "  = {}", h);
    }
    out
}

/// Render one definition: signature, effect row, contracts, body.
pub fn def(d: &Def) -> String {
    let mut out = String::new();
    let params: Vec<String> = d
        .params
        .iter()
        .enumerate()
        .map(|(i, t)| format!("{}: {}", name(i), ty(t)))
        .collect();
    let _ = write!(out, "def ({}) -> {}", params.join(", "), ty(&d.ret));
    if !d.effects.is_empty() {
        let effs: Vec<&str> = d.effects.iter().map(|e| effect_name(*e)).collect();
        let _ = write!(out, "\n  effects: {}", effs.join(", "));
    }
    let depth = d.params.len();
    if let Some(pre) = &d.pre {
        let _ = write!(out, "\n  requires {}", term(pre, depth));
    }
    if let Some(post) = &d.post {
        let _ = write!(out, "\n  ensures {}", term(post, depth + 1));
    }
    let _ = write!(out, "\n  {}", term(&d.body, depth));
    out
}

/// Binder names by depth: `a b c … z aa ab …` — stable and readable.
fn name(depth: usize) -> String {
    let mut n = depth;
    let mut s = String::new();
    loop {
        s.insert(0, (b'a' + (n % 26) as u8) as char);
        if n < 26 {
            break;
        }
        n = n / 26 - 1;
    }
    s
}

fn weft_fix_scale() -> i64 {
    crate::FIX_SCALE
}

fn ty(t: &Ty) -> String {
    match t {
        Ty::Int => "Int".into(),
        Ty::Fix => "Fix".into(),
        Ty::Bool => "Bool".into(),
        Ty::Text => "Text".into(),
        Ty::Action => "Action".into(),
        Ty::List(e) => format!("List {}", ty(e)),
        Ty::Record(fs) => {
            let fields: Vec<String> = fs.iter().map(|(k, v)| format!("{k}: {}", ty(v))).collect();
            format!("{{{}}}", fields.join(", "))
        }
    }
}

fn effect_name(e: EffectKind) -> &'static str {
    match e {
        EffectKind::Notify => "notify",
        EffectKind::Navigate => "navigate",
        EffectKind::CodexOpen => "codex.open",
        EffectKind::CommerceBuy => "commerce.buy",
        EffectKind::PresenceEmit => "presence.emit",
        EffectKind::SetState => "set_state",
        EffectKind::GiveItem => "give_item",
        EffectKind::Despawn => "despawn",
        EffectKind::Spawn => "spawn",
    }
}

fn prim_name(p: PrimOp) -> &'static str {
    match p {
        PrimOp::Add => "+",
        PrimOp::Sub => "-",
        PrimOp::Mul => "*",
        PrimOp::Div => "/",
        PrimOp::Lt => "<",
        PrimOp::Le => "<=",
        PrimOp::EqInt | PrimOp::EqText => "==",
        PrimOp::And => "and",
        PrimOp::Or => "or",
        PrimOp::Not => "not",
        PrimOp::Concat => "++",
        PrimOp::ToText => "text",
        PrimOp::FAdd => "+.",
        PrimOp::FSub => "-.",
        PrimOp::FMul => "*.",
        PrimOp::FDiv => "/.",
        PrimOp::FLt => "<.",
        PrimOp::FLe => "<=.",
        PrimOp::EqFix => "==.",
        PrimOp::FixOfInt => "fix",
        PrimOp::IntOfFix => "trunc",
        PrimOp::FixToText => "text",
        PrimOp::Len => "len",
        PrimOp::FSin => "sin",
        PrimOp::FCos => "cos",
        PrimOp::ListCat => "++list",
    }
}

/// Render a term with `depth` binders in scope (params + enclosing lets).
fn term(t: &Term, depth: usize) -> String {
    match t {
        Term::Int(v) => v.to_string(),
        Term::Fix(v) => {
            let s = weft_fix_scale();
            let sign = if *v < 0 { "-" } else { "" };
            let (w, f) = ((v / s).abs(), (v % s).abs());
            if f == 0 {
                format!("{sign}{w}.0")
            } else {
                format!("{sign}{w}.{}", format!("{f:06}").trim_end_matches('0'))
            }
        }
        Term::Bool(b) => b.to_string(),
        Term::Text(s) => format!("{s:?}"),
        // de Bruijn index i → the binder `depth - 1 - i` deep.
        Term::Var(i) => match (depth as u32).checked_sub(1 + i) {
            Some(k) => name(k as usize),
            None => format!("?var{i}"),
        },
        Term::Let(v, b) => {
            format!(
                "let {} = {} in\n  {}",
                name(depth),
                term(v, depth),
                term(b, depth + 1)
            )
        }
        Term::Map { cap, list, body } => format!(
            "map[{cap}] {} ({} -> {})",
            term(list, depth),
            name(depth),
            term(body, depth + 1)
        ),
        Term::Iota { cap, count } => format!("iota[{cap}] {}", term(count, depth)),
        Term::Fold {
            cap,
            list,
            init,
            body,
        } => format!(
            "fold[{cap}] {} from {} ({} {} -> {})",
            term(list, depth),
            term(init, depth),
            name(depth),
            name(depth + 1),
            term(body, depth + 2)
        ),
        Term::If(c, a, b) => format!(
            "if {} then {} else {}",
            term(c, depth),
            term(a, depth),
            term(b, depth)
        ),
        Term::Prim(PrimOp::Not, args) => format!("not {}", term(&args[0], depth)),
        Term::Prim(PrimOp::ToText, args) => format!("text({})", term(&args[0], depth)),
        Term::Prim(op, args) if args.len() == 2 => format!(
            "({} {} {})",
            term(&args[0], depth),
            prim_name(*op),
            term(&args[1], depth)
        ),
        Term::Prim(op, args) => {
            let xs: Vec<String> = args.iter().map(|a| term(a, depth)).collect();
            format!("{}({})", prim_name(*op), xs.join(", "))
        }
        Term::Rec(fields) => {
            let fs: Vec<String> = fields
                .iter()
                .map(|(k, v)| format!("{k}: {}", term(v, depth)))
                .collect();
            format!("{{{}}}", fs.join(", "))
        }
        Term::Get(r, k) => format!("{}.{k}", term(r, depth)),
        Term::ListNew(items) => {
            let xs: Vec<String> = items.iter().map(|i| term(i, depth)).collect();
            format!("[{}]", xs.join(", "))
        }
        Term::Call(h, args) => {
            let xs: Vec<String> = args.iter().map(|a| term(a, depth)).collect();
            format!("{:?}({})", h, xs.join(", "))
        }
        Term::Effect(kind, fields) => {
            let fs: Vec<String> = fields
                .iter()
                .map(|(k, v)| format!("{k}: {}", term(v, depth)))
                .collect();
            format!("{}{{{}}}", effect_name(*kind), fs.join(", "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{hash_def, PrimOp, Term};
    use std::collections::BTreeSet;

    #[test]
    fn a_projection_is_readable_and_deterministic() {
        // inc(x) = x + 1 with a contract — the audit view a person would read.
        let inc = Def {
            params: vec![Ty::Int],
            ret: Ty::Int,
            effects: BTreeSet::new(),
            body: Term::Prim(PrimOp::Add, vec![Term::Var(0), Term::Int(1)]),
            pre: Some(Term::Prim(PrimOp::Le, vec![Term::Int(0), Term::Var(0)])),
            post: Some(Term::Prim(PrimOp::Lt, vec![Term::Var(1), Term::Var(0)])),
        };
        let h = hash_def(&inc);
        let m = Module::build(vec![inc], 0).unwrap();
        let text = module(&m);
        assert!(text.contains("def (a: Int) -> Int"), "{text}");
        assert!(text.contains("requires (0 <= a)"), "{text}");
        // post binds params + result: result is the innermost binder `b`.
        assert!(text.contains("ensures (a < b)"), "{text}");
        assert!(text.contains("(a + 1)"), "{text}");
        assert!(text.contains(&h.to_string()), "cited by hash");
        assert_eq!(text, module(&m), "same module, same projection, always");
    }

    #[test]
    fn effects_and_lets_render_with_generated_names() {
        let d = Def {
            params: vec![Ty::Int],
            ret: Ty::Action,
            effects: BTreeSet::from([crate::EffectKind::Notify]),
            body: Term::Let(
                Box::new(Term::Prim(PrimOp::ToText, vec![Term::Var(0)])),
                Box::new(Term::Effect(
                    crate::EffectKind::Notify,
                    [("text".to_string(), Term::Var(0))].into(),
                )),
            ),
            pre: None,
            post: None,
        };
        let text = def(&d);
        assert!(text.contains("effects: notify"), "{text}");
        assert!(text.contains("let b = text(a) in"), "{text}");
        assert!(text.contains("notify{text: b}"), "{text}");
    }
}
