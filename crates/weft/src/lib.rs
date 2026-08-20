//! # Weft — the Thread's native code format (reference implementation)
//!
//! *Ship intent, not instructions.* A Weft program is a typed, **total**,
//! effect-explicit term graph, identified by the hash of its canonical
//! encoding. What crosses the wire is the same object that gets verified;
//! execution backends (this crate's reference interpreter first) compile or
//! interpret it locally. Spec: `docs/spec/weft-v0.1.md`.
//!
//! v0.1 scope — deliberately austere:
//! - **First-order and non-recursive.** Definitions call other definitions by
//!   content hash; since a definition's hash doesn't exist until it is built,
//!   cycles are unconstructible and **every program terminates by
//!   construction**. (`fold` over finite data arrives in v0.2.)
//! - **No local names.** Variables are de Bruijn indices, so
//!   alpha-equivalent programs are *identical* programs — same bytes, same
//!   hash. Names live outside the code, as metadata.
//! - **Deterministic core**: integers, booleans, text. Division by zero is
//!   defined (`= 0`), so no arithmetic traps exist. No floats.
//! - **Effects are values.** Constructing an [`Action`] performs nothing;
//!   the host (the browser) receives the returned actions and decides.
//!   A definition's declared effect row must cover everything it (and its
//!   callees, transitively) can construct — the row IS the permission.
//! - **Cost is static.** The verifier computes a fuel bound per definition;
//!   the interpreter meters actual fuel. Verified code cannot run away.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub mod draft_lib;
pub mod model_lib;
pub mod pack;
pub mod project;

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

/// A definition's identity: the SHA-256 of its canonical encoding.
/// Rendered — including on the wire — as `weft:<hex>`. To know the hash is to
/// know the thing entire.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WeftHash(pub [u8; 32]);

impl Serialize for WeftHash {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for WeftHash {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        let s = String::deserialize(de)?;
        let hex = s
            .strip_prefix("weft:")
            .ok_or_else(|| serde::de::Error::custom("weft hash must start with 'weft:'"))?;
        if hex.len() != 64 {
            return Err(serde::de::Error::custom("weft hash must be 64 hex chars"));
        }
        let mut out = [0u8; 32];
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
                .map_err(|_| serde::de::Error::custom("invalid hex in weft hash"))?;
        }
        Ok(WeftHash(out))
    }
}

impl fmt::Display for WeftHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "weft:")?;
        for b in &self.0 {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for WeftHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Short form for logs/tests: first 8 hex chars.
        write!(f, "weft:")?;
        for b in &self.0[..4] {
            write!(f, "{b:02x}")?;
        }
        write!(f, "…")
    }
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// The v0.1 type language.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Ty {
    Int,
    Bool,
    Text,
    /// Deterministic fixed-point: an i64 counting **millionths** (scale
    /// [`FIX_SCALE`]). The honest float — exact, total, identical on every
    /// host, no NaN, no rounding modes. `Fix(1_500_000)` is 1.5.
    Fix,
    /// An effect request value (see [`Action`]); opaque to programs.
    Action,
    List(Box<Ty>),
    /// Fields are kept sorted by name — the canonical order.
    Record(BTreeMap<String, Ty>),
}

// ---------------------------------------------------------------------------
// Effects
// ---------------------------------------------------------------------------

/// The effect vocabulary — the Behavior ABI's Actions plus the declarative
/// interaction effects, as *types*. Constructing one is pure; only the
/// browser performs it. A behavior whose row lacks `CommerceBuy` cannot
/// spend a traveler's gold — not "shouldn't": the verifier rejects it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectKind {
    Notify,
    Navigate,
    CodexOpen,
    CommerceBuy,
    PresenceEmit,
    SetState,
    GiveItem,
    Despawn,
    /// Conjure a new object into the world (a builtin mesh, placed + colored).
    Spawn,
}

impl EffectKind {
    /// Stable wire tag (used in canonical encoding — never reorder; append only).
    fn tag(self) -> u8 {
        match self {
            EffectKind::Notify => 0,
            EffectKind::Navigate => 1,
            EffectKind::CodexOpen => 2,
            EffectKind::CommerceBuy => 3,
            EffectKind::PresenceEmit => 4,
            EffectKind::SetState => 5,
            EffectKind::GiveItem => 6,
            EffectKind::Despawn => 7,
            EffectKind::Spawn => 8,
        }
    }
}

// ---------------------------------------------------------------------------
// Terms
// ---------------------------------------------------------------------------

/// One whole unit in [`Ty::Fix`]'s scale: fixed-point values count millionths.
pub const FIX_SCALE: i64 = 1_000_000;

/// The longest list a program may build with [`PrimOp::ListCat`]. Bounded
/// like everything else: a model of ten thousand parts is a mistake, not an
/// ambition, and the bound is what lets list growth exist at all without
/// breaking the totality guarantee.
pub const MAX_LIST: usize = 8192;

/// Pure operations on the deterministic core.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrimOp {
    Add,
    Sub,
    Mul,
    /// Total: division by zero is 0 (no traps exist in Weft).
    Div,
    Lt,
    Le,
    EqInt,
    EqText,
    And,
    Or,
    Not,
    Concat,
    /// Render an Int as Text (base 10) — total, deterministic.
    ToText,
    // --- fixed-point (appended v0.1.1; discriminant order is the encoding) ---
    /// Fix + Fix → Fix (wrapping).
    FAdd,
    /// Fix − Fix → Fix (wrapping).
    FSub,
    /// Fix × Fix → Fix — exact via 128-bit intermediate, then rescaled.
    FMul,
    /// Fix ÷ Fix → Fix — exact via 128-bit intermediate; ÷0 = 0 (total).
    FDiv,
    FLt,
    FLe,
    EqFix,
    /// Int → Fix (×[`FIX_SCALE`], wrapping).
    FixOfInt,
    /// Fix → Int (truncate toward zero).
    IntOfFix,
    /// Render a Fix as decimal Text ("1.5", "-0.25") — canonical: no trailing
    /// zeros, no exponent, always a leading integer digit.
    FixToText,
    /// List length (any element type) → Int.
    Len,
    /// Fixed-point sine (radians in, Fix in/out) — Bhaskara I's approximation
    /// computed in pure integer arithmetic: deterministic on every host,
    /// ~0.2 % max error — a world-builder's sine, not a scientist's.
    FSin,
    /// Fixed-point cosine — `FSin(x + π/2)`.
    FCos,
    /// Append two lists of the same element type — the composition primitive.
    /// Weft has no recursive types, so a *tree* cannot be a value; a list of
    /// uniform records can, and `ListCat` is what lets one program's parts
    /// join another's (the modeling library is built on it). Appended after
    /// `FCos`: discriminant order is the encoding, so existing hashes hold.
    ListCat,
}

/// The term graph. Variables are de Bruijn indices (0 = innermost binder);
/// a definition's parameters are the outermost binders, in order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Term {
    Int(i64),
    /// Fixed-point literal (raw millionths — `Term::Fix(1_500_000)` is 1.5).
    Fix(i64),
    Bool(bool),
    Text(String),
    /// de Bruijn index into the binder stack.
    Var(u32),
    /// Bind the first term's value; evaluate the second with it as index 0.
    Let(Box<Term>, Box<Term>),
    If(Box<Term>, Box<Term>, Box<Term>),
    Prim(PrimOp, Vec<Term>),
    /// Record construction; field order is canonical (sorted) via BTreeMap.
    Rec(BTreeMap<String, Term>),
    /// Field projection.
    Get(Box<Term>, String),
    ListNew(Vec<Term>),
    /// Call another definition **by hash** — the only kind of call.
    Call(WeftHash, Vec<Term>),
    /// Construct an effect request (an [`Action`] value). Pure.
    Effect(EffectKind, BTreeMap<String, Term>),
    /// Bounded map: evaluate `body` (element = Var 0) for each of at most
    /// `cap` elements of `list` — elements beyond the cap are dropped. The
    /// cap is what keeps the fuel bound static: totality never bends.
    Map {
        cap: u32,
        list: Box<Term>,
        body: Box<Term>,
    },
    /// Bounded fold: thread an accumulator through at most `cap` elements
    /// (`body` sees acc = Var 1, element = Var 0).
    Fold {
        cap: u32,
        list: Box<Term>,
        init: Box<Term>,
        body: Box<Term>,
    },
    /// The list `0..min(count, cap)` — the generator's seed. `cap` keeps the
    /// fuel bound static; with Map/Fold it makes rings, grids, and spirals.
    Iota {
        cap: u32,
        count: Box<Term>,
    },
}

// ---------------------------------------------------------------------------
// Definitions & modules
// ---------------------------------------------------------------------------

/// One verified unit of Weft: typed parameters, result type, declared effect
/// row, optional contracts, and the body. Its hash is its name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Def {
    pub params: Vec<Ty>,
    pub ret: Ty,
    /// Everything this definition (transitively) may construct.
    pub effects: BTreeSet<EffectKind>,
    pub body: Term,
    /// Contract: precondition over the parameters (must type as Bool).
    #[serde(default)]
    pub pre: Option<Term>,
    /// Contract: postcondition over parameters + result (result = index 0).
    #[serde(default)]
    pub post: Option<Term>,
}

/// A set of definitions keyed by hash — the unit of distribution. `entry`
/// names the behavior's event handler.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Module {
    pub defs: BTreeMap<WeftHash, Def>,
    pub entry: WeftHash,
}

impl Module {
    /// Build a module from defs, hashing each; returns the module and the
    /// hash of `entry_def` (which must be among `defs`).
    pub fn build(defs: Vec<Def>, entry_index: usize) -> Result<Self, WeftError> {
        let mut map = BTreeMap::new();
        let mut entry = None;
        for (i, d) in defs.into_iter().enumerate() {
            let h = hash_def(&d);
            if i == entry_index {
                entry = Some(h);
            }
            map.insert(h, d);
        }
        let entry = entry.ok_or(WeftError::UnknownEntry)?;
        Ok(Module { defs: map, entry })
    }
}

// ---------------------------------------------------------------------------
// Canonical encoding + hashing
// ---------------------------------------------------------------------------

fn enc_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn enc_i64(out: &mut Vec<u8>, v: i64) {
    out.extend_from_slice(&v.to_le_bytes());
}
fn enc_str(out: &mut Vec<u8>, s: &str) {
    enc_u32(out, s.len() as u32);
    out.extend_from_slice(s.as_bytes());
}

fn enc_ty(out: &mut Vec<u8>, ty: &Ty) {
    match ty {
        Ty::Int => out.push(0),
        Ty::Bool => out.push(1),
        Ty::Text => out.push(2),
        Ty::Action => out.push(3),
        Ty::List(t) => {
            out.push(4);
            enc_ty(out, t);
        }
        Ty::Fix => out.push(6),
        Ty::Record(fields) => {
            out.push(5);
            enc_u32(out, fields.len() as u32);
            for (k, t) in fields {
                enc_str(out, k);
                enc_ty(out, t);
            }
        }
    }
}

fn enc_term(out: &mut Vec<u8>, t: &Term) {
    match t {
        Term::Int(v) => {
            out.push(0);
            enc_i64(out, *v);
        }
        Term::Bool(b) => {
            out.push(1);
            out.push(*b as u8);
        }
        Term::Text(s) => {
            out.push(2);
            enc_str(out, s);
        }
        Term::Var(i) => {
            out.push(3);
            enc_u32(out, *i);
        }
        Term::Let(v, b) => {
            out.push(4);
            enc_term(out, v);
            enc_term(out, b);
        }
        Term::If(c, a, b) => {
            out.push(5);
            enc_term(out, c);
            enc_term(out, a);
            enc_term(out, b);
        }
        Term::Prim(op, args) => {
            out.push(6);
            out.push(*op as u8);
            enc_u32(out, args.len() as u32);
            for a in args {
                enc_term(out, a);
            }
        }
        Term::Rec(fields) => {
            out.push(7);
            enc_u32(out, fields.len() as u32);
            for (k, v) in fields {
                enc_str(out, k);
                enc_term(out, v);
            }
        }
        Term::Get(r, k) => {
            out.push(8);
            enc_term(out, r);
            enc_str(out, k);
        }
        Term::ListNew(items) => {
            out.push(9);
            enc_u32(out, items.len() as u32);
            for i in items {
                enc_term(out, i);
            }
        }
        Term::Call(h, args) => {
            out.push(10);
            out.extend_from_slice(&h.0);
            enc_u32(out, args.len() as u32);
            for a in args {
                enc_term(out, a);
            }
        }
        Term::Fix(v) => {
            out.push(12);
            enc_i64(out, *v);
        }
        Term::Map { cap, list, body } => {
            out.push(13);
            enc_u32(out, *cap);
            enc_term(out, list);
            enc_term(out, body);
        }
        Term::Fold {
            cap,
            list,
            init,
            body,
        } => {
            out.push(14);
            enc_u32(out, *cap);
            enc_term(out, list);
            enc_term(out, init);
            enc_term(out, body);
        }
        Term::Iota { cap, count } => {
            out.push(15);
            enc_u32(out, *cap);
            enc_term(out, count);
        }
        Term::Effect(kind, fields) => {
            out.push(11);
            out.push(kind.tag());
            enc_u32(out, fields.len() as u32);
            for (k, v) in fields {
                enc_str(out, k);
                enc_term(out, v);
            }
        }
    }
}

/// The canonical bytes of a definition — what the hash is over.
pub fn canonical_bytes(def: &Def) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"weft/0.1\0");
    enc_u32(&mut out, def.params.len() as u32);
    for p in &def.params {
        enc_ty(&mut out, p);
    }
    enc_ty(&mut out, &def.ret);
    enc_u32(&mut out, def.effects.len() as u32);
    for e in &def.effects {
        out.push(e.tag());
    }
    enc_term(&mut out, &def.body);
    match &def.pre {
        None => out.push(0),
        Some(t) => {
            out.push(1);
            enc_term(&mut out, t);
        }
    }
    match &def.post {
        None => out.push(0),
        Some(t) => {
            out.push(1);
            enc_term(&mut out, t);
        }
    }
    out
}

/// A definition's identity.
pub fn hash_def(def: &Def) -> WeftHash {
    let digest = Sha256::digest(canonical_bytes(def));
    let mut h = [0u8; 32];
    h.copy_from_slice(&digest);
    WeftHash(h)
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WeftError {
    /// A type mismatch, with a machine-usable description.
    Type(String),
    /// A constructed effect kind not covered by the declared row.
    EffectNotDeclared(EffectKind),
    /// A `Call` names a hash the module doesn't contain.
    UnknownCall(String),
    UnknownEntry,
    /// Runtime guard only — verified code cannot reach it (bound is static).
    FuelExhausted,
    /// A contract evaluated to false.
    ContractViolated(&'static str),
}

impl fmt::Display for WeftError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WeftError::Type(m) => write!(f, "type error: {m}"),
            WeftError::EffectNotDeclared(k) => write!(f, "effect not declared: {k:?}"),
            WeftError::UnknownCall(h) => write!(f, "unknown call target {h}"),
            WeftError::UnknownEntry => write!(f, "entry hash not in module"),
            WeftError::FuelExhausted => write!(f, "fuel exhausted"),
            WeftError::ContractViolated(w) => write!(f, "contract violated: {w}"),
        }
    }
}
impl std::error::Error for WeftError {}

// ---------------------------------------------------------------------------
// Verification
// ---------------------------------------------------------------------------

/// The verifier's certificate for one definition: its checked effect row and
/// the static fuel bound (an upper bound on interpreter steps per call).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verified {
    pub effects: BTreeSet<EffectKind>,
    pub fuel_bound: u64,
}

/// Verify every definition in a module: types, effect rows (transitive),
/// contract typing, call closure — and compute static fuel bounds.
/// Verification is per-hash cacheable: same hash, same verdict, forever.
pub fn verify_module(m: &Module) -> Result<BTreeMap<WeftHash, Verified>, WeftError> {
    let mut done: BTreeMap<WeftHash, Verified> = BTreeMap::new();
    // Hash-linking makes cycles unconstructible, so simple iteration to a
    // fixpoint terminates: each pass verifies defs whose callees are done.
    let mut remaining: Vec<WeftHash> = m.defs.keys().copied().collect();
    while !remaining.is_empty() {
        let before = remaining.len();
        let mut next = Vec::new();
        for h in remaining {
            let def = &m.defs[&h];
            if callees(&def.body).iter().all(|c| done.contains_key(c)) {
                let v = verify_def(def, &done, m)?;
                done.insert(h, v);
            } else {
                next.push(h);
            }
        }
        if next.len() == before {
            // Only possible if a call targets a missing def.
            let missing = next
                .iter()
                .flat_map(|h| callees(&m.defs[h].body))
                .find(|c| !m.defs.contains_key(c))
                .map(|c| c.to_string())
                .unwrap_or_else(|| "<cycle?>".into());
            return Err(WeftError::UnknownCall(missing));
        }
        remaining = next;
    }
    if !done.contains_key(&m.entry) {
        return Err(WeftError::UnknownEntry);
    }
    Ok(done)
}

/// Public view of a term's call targets (pack's closure trim uses it).
pub fn callees_of(t: &Term) -> BTreeSet<WeftHash> {
    callees(t)
}

fn callees(t: &Term) -> BTreeSet<WeftHash> {
    let mut out = BTreeSet::new();
    fn walk(t: &Term, out: &mut BTreeSet<WeftHash>) {
        match t {
            Term::Call(h, args) => {
                out.insert(*h);
                args.iter().for_each(|a| walk(a, out));
            }
            Term::Let(a, b) => {
                walk(a, out);
                walk(b, out);
            }
            Term::If(a, b, c) => {
                walk(a, out);
                walk(b, out);
                walk(c, out);
            }
            Term::Prim(_, xs) | Term::ListNew(xs) => xs.iter().for_each(|x| walk(x, out)),
            Term::Map { list, body, .. } => {
                walk(list, out);
                walk(body, out);
            }
            Term::Fold {
                list, init, body, ..
            } => {
                walk(list, out);
                walk(init, out);
                walk(body, out);
            }
            Term::Iota { count, .. } => walk(count, out),
            Term::Rec(fs) | Term::Effect(_, fs) => fs.values().for_each(|x| walk(x, out)),
            Term::Get(r, _) => walk(r, out),
            _ => {}
        }
    }
    walk(t, &mut out);
    out
}

fn verify_def(
    def: &Def,
    done: &BTreeMap<WeftHash, Verified>,
    m: &Module,
) -> Result<Verified, WeftError> {
    let mut ctx: Vec<Ty> = def.params.clone();
    let mut used = BTreeSet::new();
    let mut fuel: u64 = 0;
    let ty = check(&def.body, &mut ctx, &mut used, &mut fuel, done, m)?;
    if ty != def.ret {
        return Err(WeftError::Type(format!(
            "body is {ty:?}, declared {:?}",
            def.ret
        )));
    }
    for k in &used {
        if !def.effects.contains(k) {
            return Err(WeftError::EffectNotDeclared(*k));
        }
    }
    // Contracts type against params (pre) / params+result (post), as Bool.
    if let Some(pre) = &def.pre {
        let mut c = def.params.clone();
        let (mut u, mut f) = (BTreeSet::new(), 0u64);
        let t = check(pre, &mut c, &mut u, &mut f, done, m)?;
        if t != Ty::Bool || !u.is_empty() {
            return Err(WeftError::Type("pre must be a pure Bool".into()));
        }
        fuel += f;
    }
    if let Some(post) = &def.post {
        let mut c = def.params.clone();
        c.push(def.ret.clone());
        let (mut u, mut f) = (BTreeSet::new(), 0u64);
        let t = check(post, &mut c, &mut u, &mut f, done, m)?;
        if t != Ty::Bool || !u.is_empty() {
            return Err(WeftError::Type("post must be a pure Bool".into()));
        }
        fuel += f;
    }
    Ok(Verified {
        effects: used,
        fuel_bound: fuel,
    })
}

/// Typecheck one term; accumulate used effects and the static step bound.
fn check(
    t: &Term,
    ctx: &mut Vec<Ty>,
    used: &mut BTreeSet<EffectKind>,
    fuel: &mut u64,
    done: &BTreeMap<WeftHash, Verified>,
    m: &Module,
) -> Result<Ty, WeftError> {
    *fuel += 1;
    match t {
        Term::Int(_) => Ok(Ty::Int),
        Term::Fix(_) => Ok(Ty::Fix),
        Term::Bool(_) => Ok(Ty::Bool),
        Term::Text(_) => Ok(Ty::Text),
        Term::Var(i) => {
            let idx = ctx.len().checked_sub(1 + *i as usize);
            idx.and_then(|k| ctx.get(k).cloned())
                .ok_or_else(|| WeftError::Type(format!("unbound var {i}")))
        }
        Term::Let(v, b) => {
            let vt = check(v, ctx, used, fuel, done, m)?;
            ctx.push(vt);
            let bt = check(b, ctx, used, fuel, done, m);
            ctx.pop();
            bt
        }
        Term::If(c, a, b) => {
            if check(c, ctx, used, fuel, done, m)? != Ty::Bool {
                return Err(WeftError::Type("if condition must be Bool".into()));
            }
            let at = check(a, ctx, used, fuel, done, m)?;
            let bt = check(b, ctx, used, fuel, done, m)?;
            if at != bt {
                return Err(WeftError::Type("if branches differ".into()));
            }
            Ok(at)
        }
        Term::Prim(op, args) => {
            let ats: Result<Vec<Ty>, _> = args
                .iter()
                .map(|a| check(a, ctx, used, fuel, done, m))
                .collect();
            let ats = ats?;
            use PrimOp::*;
            let (want, out): (Vec<Ty>, Ty) = match op {
                Add | Sub | Mul | Div => (vec![Ty::Int, Ty::Int], Ty::Int),
                Lt | Le | EqInt => (vec![Ty::Int, Ty::Int], Ty::Bool),
                EqText => (vec![Ty::Text, Ty::Text], Ty::Bool),
                And | Or => (vec![Ty::Bool, Ty::Bool], Ty::Bool),
                Not => (vec![Ty::Bool], Ty::Bool),
                Concat => (vec![Ty::Text, Ty::Text], Ty::Text),
                ToText => (vec![Ty::Int], Ty::Text),
                FAdd | FSub | FMul | FDiv => (vec![Ty::Fix, Ty::Fix], Ty::Fix),
                FLt | FLe | EqFix => (vec![Ty::Fix, Ty::Fix], Ty::Bool),
                FixOfInt => (vec![Ty::Int], Ty::Fix),
                IntOfFix => (vec![Ty::Fix], Ty::Int),
                FixToText => (vec![Ty::Fix], Ty::Text),
                FSin | FCos => (vec![Ty::Fix], Ty::Fix),
                // Len is the one polymorphic prim: List(T) → Int for any T.
                Len => {
                    return match ats.as_slice() {
                        [Ty::List(_)] => Ok(Ty::Int),
                        other => Err(WeftError::Type(format!(
                            "len expects a list, got {other:?}"
                        ))),
                    };
                }
                // ListCat: List(T) × List(T) → List(T) — same T, no coercion.
                ListCat => {
                    return match ats.as_slice() {
                        [Ty::List(a), Ty::List(b)] if a == b => Ok(Ty::List(a.clone())),
                        other => Err(WeftError::Type(format!(
                            "list-cat expects two lists of the same element type, got {other:?}"
                        ))),
                    };
                }
            };
            if ats != want {
                return Err(WeftError::Type(format!(
                    "{op:?} expects {want:?}, got {ats:?}"
                )));
            }
            Ok(out)
        }
        Term::Rec(fields) => {
            let mut tys = BTreeMap::new();
            for (k, v) in fields {
                tys.insert(k.clone(), check(v, ctx, used, fuel, done, m)?);
            }
            Ok(Ty::Record(tys))
        }
        Term::Get(r, k) => match check(r, ctx, used, fuel, done, m)? {
            Ty::Record(fs) => fs
                .get(k)
                .cloned()
                .ok_or_else(|| WeftError::Type(format!("no field '{k}'"))),
            other => Err(WeftError::Type(format!("get on non-record {other:?}"))),
        },
        Term::ListNew(items) => {
            let mut elem: Option<Ty> = None;
            for i in items {
                let t = check(i, ctx, used, fuel, done, m)?;
                match &elem {
                    None => elem = Some(t),
                    Some(e) if *e == t => {}
                    Some(e) => return Err(WeftError::Type(format!("list mixes {e:?} and {t:?}"))),
                }
            }
            Ok(Ty::List(Box::new(elem.unwrap_or(Ty::Action))))
        }
        Term::Call(h, args) => {
            let callee = m
                .defs
                .get(h)
                .ok_or_else(|| WeftError::UnknownCall(h.to_string()))?;
            let cert = done
                .get(h)
                .ok_or_else(|| WeftError::UnknownCall(h.to_string()))?;
            if args.len() != callee.params.len() {
                return Err(WeftError::Type("call arity".into()));
            }
            for (a, want) in args.iter().zip(&callee.params) {
                let at = check(a, ctx, used, fuel, done, m)?;
                if at != *want {
                    return Err(WeftError::Type(format!("call arg {at:?} vs {want:?}")));
                }
            }
            // Transitive effects + cost flow into the caller's certificate.
            used.extend(cert.effects.iter().copied());
            *fuel += cert.fuel_bound;
            Ok(callee.ret.clone())
        }
        Term::Effect(kind, fields) => {
            for v in fields.values() {
                check(v, ctx, used, fuel, done, m)?;
            }
            used.insert(*kind);
            Ok(Ty::Action)
        }
        Term::Map { cap, list, body } => {
            let lt = check(list, ctx, used, fuel, done, m)?;
            let Ty::List(elem) = lt else {
                return Err(WeftError::Type(format!("map over non-list {lt:?}")));
            };
            // The body is costed once, then charged `cap` times — that is the
            // whole trick: iteration whose worst case is written in the term.
            let mut body_fuel: u64 = 0;
            ctx.push((*elem).clone());
            let bt = check(body, ctx, used, &mut body_fuel, done, m);
            ctx.pop();
            let bt = bt?;
            *fuel = fuel.saturating_add(body_fuel.saturating_mul(*cap as u64) + *cap as u64);
            Ok(Ty::List(Box::new(bt)))
        }
        Term::Fold {
            cap,
            list,
            init,
            body,
        } => {
            let lt = check(list, ctx, used, fuel, done, m)?;
            let Ty::List(elem) = lt else {
                return Err(WeftError::Type(format!("fold over non-list {lt:?}")));
            };
            let it = check(init, ctx, used, fuel, done, m)?;
            let mut body_fuel: u64 = 0;
            ctx.push(it.clone()); // acc = Var 1
            ctx.push((*elem).clone()); // element = Var 0
            let bt = check(body, ctx, used, &mut body_fuel, done, m);
            ctx.pop();
            ctx.pop();
            let bt = bt?;
            if bt != it {
                return Err(WeftError::Type(format!(
                    "fold body is {bt:?}, acc is {it:?}"
                )));
            }
            *fuel = fuel.saturating_add(body_fuel.saturating_mul(*cap as u64) + *cap as u64);
            Ok(it)
        }
        Term::Iota { cap, count } => {
            let ct = check(count, ctx, used, fuel, done, m)?;
            if ct != Ty::Int {
                return Err(WeftError::Type(format!(
                    "iota count must be Int, got {ct:?}"
                )));
            }
            *fuel = fuel.saturating_add(*cap as u64);
            Ok(Ty::List(Box::new(Ty::Int)))
        }
    }
}

// ---------------------------------------------------------------------------
// Values & evaluation
// ---------------------------------------------------------------------------

/// A runtime value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Value {
    Int(i64),
    /// Raw millionths (see [`FIX_SCALE`]).
    Fix(i64),
    Bool(bool),
    Text(String),
    List(Vec<Value>),
    Rec(BTreeMap<String, Value>),
    /// A constructed effect request — inert data until the host acts on it.
    Action {
        kind: EffectKind,
        fields: BTreeMap<String, Value>,
    },
}

/// Evaluate `entry`-style: call a definition with argument values, metering
/// fuel and enforcing contracts. Deterministic: same module + args → same
/// value + same fuel spent, on every host.
#[derive(Debug, Clone, PartialEq)]
pub struct Evaluated {
    pub value: Value,
    pub fuel_spent: u64,
}

pub fn eval_call(
    m: &Module,
    target: WeftHash,
    args: Vec<Value>,
    max_fuel: u64,
) -> Result<Evaluated, WeftError> {
    let mut fuel = max_fuel;
    let def = m
        .defs
        .get(&target)
        .ok_or_else(|| WeftError::UnknownCall(target.to_string()))?;
    // Precondition gate.
    if let Some(pre) = &def.pre {
        let mut env = args.clone();
        if eval(pre, &mut env, m, &mut fuel)? != Value::Bool(true) {
            return Err(WeftError::ContractViolated("pre"));
        }
    }
    let mut env = args.clone();
    let value = eval(&def.body, &mut env, m, &mut fuel)?;
    // Postcondition gate (params + result, result innermost).
    if let Some(post) = &def.post {
        let mut env = args;
        env.push(value.clone());
        if eval(post, &mut env, m, &mut fuel)? != Value::Bool(true) {
            return Err(WeftError::ContractViolated("post"));
        }
    }
    Ok(Evaluated {
        value,
        fuel_spent: max_fuel - fuel,
    })
}

fn eval(t: &Term, env: &mut Vec<Value>, m: &Module, fuel: &mut u64) -> Result<Value, WeftError> {
    if *fuel == 0 {
        return Err(WeftError::FuelExhausted);
    }
    *fuel -= 1;
    Ok(match t {
        Term::Int(v) => Value::Int(*v),
        Term::Fix(v) => Value::Fix(*v),
        Term::Bool(b) => Value::Bool(*b),
        Term::Text(s) => Value::Text(s.clone()),
        Term::Var(i) => env[env.len() - 1 - *i as usize].clone(),
        Term::Let(v, b) => {
            let val = eval(v, env, m, fuel)?;
            env.push(val);
            let out = eval(b, env, m, fuel);
            env.pop();
            out?
        }
        Term::If(c, a, b) => {
            if eval(c, env, m, fuel)? == Value::Bool(true) {
                eval(a, env, m, fuel)?
            } else {
                eval(b, env, m, fuel)?
            }
        }
        Term::Prim(op, args) => {
            let vals: Result<Vec<Value>, _> = args.iter().map(|a| eval(a, env, m, fuel)).collect();
            let vals = vals?;
            // `ListCat` is the one prim whose *result* can grow: fold it over
            // itself and a list doubles each round. Charge fuel by the length
            // produced (work costs fuel — the same law as everywhere else) and
            // cap the length, so growth is bounded by the same static budget
            // the verifier proved. Totality never bends.
            if *op == PrimOp::ListCat {
                let n = match (vals.first(), vals.get(1)) {
                    (Some(Value::List(a)), Some(Value::List(b))) => a.len() + b.len(),
                    _ => 0,
                } as u64;
                if n > *fuel {
                    return Err(WeftError::FuelExhausted);
                }
                *fuel -= n;
                if n as usize > MAX_LIST {
                    return Err(WeftError::FuelExhausted);
                }
            }
            prim(*op, vals)
        }
        Term::Rec(fields) => {
            let mut out = BTreeMap::new();
            for (k, v) in fields {
                out.insert(k.clone(), eval(v, env, m, fuel)?);
            }
            Value::Rec(out)
        }
        Term::Get(r, k) => match eval(r, env, m, fuel)? {
            Value::Rec(fs) => fs[k].clone(),
            _ => unreachable!("verified"),
        },
        Term::ListNew(items) => {
            let vals: Result<Vec<Value>, _> = items.iter().map(|i| eval(i, env, m, fuel)).collect();
            Value::List(vals?)
        }
        Term::Call(h, args) => {
            let vals: Result<Vec<Value>, _> = args.iter().map(|a| eval(a, env, m, fuel)).collect();
            let def = m
                .defs
                .get(h)
                .ok_or_else(|| WeftError::UnknownCall(h.to_string()))?;
            let mut callee_env = vals?;
            eval(&def.body, &mut callee_env, m, fuel)?
        }
        Term::Effect(kind, fields) => {
            let mut out = BTreeMap::new();
            for (k, v) in fields {
                out.insert(k.clone(), eval(v, env, m, fuel)?);
            }
            Value::Action {
                kind: *kind,
                fields: out,
            }
        }
        Term::Map { cap, list, body } => {
            let Value::List(items) = eval(list, env, m, fuel)? else {
                unreachable!("verified")
            };
            let mut out = Vec::new();
            for item in items.into_iter().take(*cap as usize) {
                env.push(item);
                let v = eval(body, env, m, fuel);
                env.pop();
                out.push(v?);
            }
            Value::List(out)
        }
        Term::Fold {
            cap,
            list,
            init,
            body,
        } => {
            let Value::List(items) = eval(list, env, m, fuel)? else {
                unreachable!("verified")
            };
            let mut acc = eval(init, env, m, fuel)?;
            for item in items.into_iter().take(*cap as usize) {
                env.push(acc);
                env.push(item);
                let v = eval(body, env, m, fuel);
                env.pop();
                env.pop();
                acc = v?;
            }
            acc
        }
        Term::Iota { cap, count } => {
            let Value::Int(n) = eval(count, env, m, fuel)? else {
                unreachable!("verified")
            };
            let n = n.max(0).min(*cap as i64);
            Value::List((0..n).map(Value::Int).collect())
        }
    })
}

fn prim(op: PrimOp, vals: Vec<Value>) -> Value {
    use PrimOp::*;
    use Value::*;
    match (op, vals.as_slice()) {
        (Add, [Int(a), Int(b)]) => Int(a.wrapping_add(*b)),
        (Sub, [Int(a), Int(b)]) => Int(a.wrapping_sub(*b)),
        (Mul, [Int(a), Int(b)]) => Int(a.wrapping_mul(*b)),
        // Total division: /0 = 0 (documented; no traps exist in Weft).
        (Div, [Int(a), Int(b)]) => Int(if *b == 0 { 0 } else { a.wrapping_div(*b) }),
        (Lt, [Int(a), Int(b)]) => Bool(a < b),
        (Le, [Int(a), Int(b)]) => Bool(a <= b),
        (EqInt, [Int(a), Int(b)]) => Bool(a == b),
        (EqText, [Text(a), Text(b)]) => Bool(a == b),
        (And, [Bool(a), Bool(b)]) => Bool(*a && *b),
        (Or, [Bool(a), Bool(b)]) => Bool(*a || *b),
        (Not, [Bool(a)]) => Bool(!a),
        (Concat, [Text(a), Text(b)]) => Text(format!("{a}{b}")),
        (ToText, [Int(a)]) => Text(a.to_string()),
        (FAdd, [Fix(a), Fix(b)]) => Fix(a.wrapping_add(*b)),
        (FSub, [Fix(a), Fix(b)]) => Fix(a.wrapping_sub(*b)),
        (FMul, [Fix(a), Fix(b)]) => Fix(((*a as i128 * *b as i128) / FIX_SCALE as i128) as i64),
        (FDiv, [Fix(a), Fix(b)]) => Fix(if *b == 0 {
            0
        } else {
            ((*a as i128 * FIX_SCALE as i128) / *b as i128) as i64
        }),
        (FLt, [Fix(a), Fix(b)]) => Bool(a < b),
        (FLe, [Fix(a), Fix(b)]) => Bool(a <= b),
        (EqFix, [Fix(a), Fix(b)]) => Bool(a == b),
        (FixOfInt, [Int(a)]) => Fix(a.wrapping_mul(FIX_SCALE)),
        (IntOfFix, [Fix(a)]) => Int(a / FIX_SCALE),
        (FixToText, [Fix(a)]) => Text(fix_to_text(*a)),
        (Len, [List(xs)]) => Int(xs.len() as i64),
        (FSin, [Fix(a)]) => Fix(fix_sin(*a)),
        (FCos, [Fix(a)]) => Fix(fix_sin(a.wrapping_add(FIX_PI / 2))),
        (ListCat, [List(a), List(b)]) => {
            let mut out = a.clone();
            out.extend(b.iter().cloned());
            List(out)
        }
        _ => unreachable!("verified"),
    }
}

/// π in Fix micros — the reference constant every host shares.
pub const FIX_PI: i64 = 3_141_593;
/// τ (2π) in Fix micros.
pub const FIX_TAU: i64 = 6_283_185;

/// Deterministic fixed-point sine: range-reduce to [0, π] with sign, then
/// Bhaskara I's rational approximation `16x(π−x) / (5π² − 4x(π−x))` in i128 —
/// integer in, integer out, identical everywhere.
fn fix_sin(x: i64) -> i64 {
    let mut r = x % FIX_TAU;
    if r < 0 {
        r += FIX_TAU;
    }
    let (r, sign) = if r > FIX_PI {
        (r - FIX_PI, -1i128)
    } else {
        (r, 1i128)
    };
    let x = r as i128;
    let pi = FIX_PI as i128;
    let scale = FIX_SCALE as i128;
    let num = 16 * x * (pi - x); // ≤ 16·(π/2)² ≈ 4e13 — well inside i128
    let den = 5 * pi * pi - 4 * x * (pi - x);
    if den == 0 {
        return 0;
    }
    ((sign * num * scale) / den) as i64
}

/// Canonical decimal rendering of a Fix: sign, integer part, then up to six
/// fractional digits with trailing zeros trimmed. Total and deterministic —
/// this string is part of the language's observable behavior.
fn fix_to_text(raw: i64) -> String {
    let neg = raw < 0;
    let mag = (raw as i128).unsigned_abs();
    let whole = mag / FIX_SCALE as u128;
    let frac = (mag % FIX_SCALE as u128) as u64;
    let mut s = String::new();
    if neg {
        s.push('-');
    }
    s.push_str(&whole.to_string());
    if frac != 0 {
        let f = format!("{frac:06}");
        s.push('.');
        s.push_str(f.trim_end_matches('0'));
    }
    s
}

// ---------------------------------------------------------------------------
// Tests — the calculus proves itself headlessly
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Bounded iteration: map doubles, fold sums, the cap truncates, and the
    /// static fuel bound covers the worst case — loops that cannot run away.
    #[test]
    fn bounded_map_and_fold_stay_total() {
        let body = Term::Rec(
            [
                (
                    "doubled".to_string(),
                    Term::Map {
                        cap: 8,
                        list: Box::new(Term::ListNew(vec![
                            Term::Int(1),
                            Term::Int(2),
                            Term::Int(3),
                        ])),
                        body: Box::new(Term::Prim(PrimOp::Mul, vec![Term::Var(0), Term::Int(2)])),
                    },
                ),
                (
                    "sum".to_string(),
                    Term::Fold {
                        cap: 8,
                        list: Box::new(Term::ListNew(vec![
                            Term::Int(10),
                            Term::Int(20),
                            Term::Int(30),
                        ])),
                        init: Box::new(Term::Int(0)),
                        body: Box::new(Term::Prim(PrimOp::Add, vec![Term::Var(1), Term::Var(0)])),
                    },
                ),
                (
                    "capped".to_string(),
                    Term::Fold {
                        cap: 2, // only the first two elements count
                        list: Box::new(Term::ListNew(vec![
                            Term::Int(1),
                            Term::Int(1),
                            Term::Int(1),
                        ])),
                        init: Box::new(Term::Int(0)),
                        body: Box::new(Term::Prim(PrimOp::Add, vec![Term::Var(1), Term::Var(0)])),
                    },
                ),
                (
                    "count".to_string(),
                    Term::Prim(
                        PrimOp::Len,
                        vec![Term::ListNew(vec![Term::Int(7), Term::Int(7)])],
                    ),
                ),
            ]
            .into(),
        );
        let ret = Ty::Record(
            [
                ("doubled".to_string(), Ty::List(Box::new(Ty::Int))),
                ("sum".to_string(), Ty::Int),
                ("capped".to_string(), Ty::Int),
                ("count".to_string(), Ty::Int),
            ]
            .into(),
        );
        let def = Def {
            params: vec![],
            ret,
            effects: BTreeSet::new(),
            body,
            pre: None,
            post: None,
        };
        let m = Module::build(vec![def], 0).unwrap();
        let certs = verify_module(&m).expect("bounded iteration verifies");
        let bound = certs[&m.entry].fuel_bound;
        let out = eval_call(&m, m.entry, vec![], bound + 8).unwrap();
        assert!(
            out.fuel_spent <= bound,
            "static bound covers the run ({} <= {bound})",
            out.fuel_spent
        );
        let Value::Rec(r) = out.value else { panic!() };
        assert_eq!(
            r["doubled"],
            Value::List(vec![Value::Int(2), Value::Int(4), Value::Int(6)])
        );
        assert_eq!(r["sum"], Value::Int(60));
        assert_eq!(r["capped"], Value::Int(2), "cap truncates");
        assert_eq!(r["count"], Value::Int(2));
    }

    /// Fixed-point is exact and total: 0.1 +. 0.2 is EXACTLY 0.3 (no float
    /// drift), ÷0 is 0, and the decimal rendering is canonical.
    #[test]
    fn fix_arithmetic_is_exact_total_and_typed() {
        let fx = |m: i64| Term::Fix(m);
        let body = Term::Rec(
            [
                (
                    "sum".to_string(),
                    Term::Prim(PrimOp::FAdd, vec![fx(100_000), fx(200_000)]),
                ),
                (
                    "prod".to_string(),
                    Term::Prim(PrimOp::FMul, vec![fx(1_500_000), fx(2_000_000)]),
                ),
                (
                    "quot".to_string(),
                    Term::Prim(PrimOp::FDiv, vec![fx(1_000_000), fx(0)]),
                ),
                (
                    "text".to_string(),
                    Term::Prim(
                        PrimOp::FixToText,
                        vec![Term::Prim(PrimOp::FSub, vec![fx(0), fx(250_000)])],
                    ),
                ),
                (
                    "trunc".to_string(),
                    Term::Prim(PrimOp::IntOfFix, vec![fx(2_900_000)]),
                ),
            ]
            .into(),
        );
        let ret = Ty::Record(
            [
                ("sum".to_string(), Ty::Fix),
                ("prod".to_string(), Ty::Fix),
                ("quot".to_string(), Ty::Fix),
                ("text".to_string(), Ty::Text),
                ("trunc".to_string(), Ty::Int),
            ]
            .into(),
        );
        let def = Def {
            params: vec![],
            ret,
            effects: BTreeSet::new(),
            body,
            pre: None,
            post: None,
        };
        let m = Module::build(vec![def], 0).unwrap();
        verify_module(&m).expect("fix ops verify");
        let out = eval_call(&m, m.entry, vec![], 10_000).unwrap();
        let Value::Rec(r) = out.value else { panic!() };
        assert_eq!(r["sum"], Value::Fix(300_000)); // 0.1 + 0.2 == 0.3, exactly
        assert_eq!(r["prod"], Value::Fix(3_000_000)); // 1.5 × 2 == 3
        assert_eq!(r["quot"], Value::Fix(0)); // total ÷0
        assert_eq!(r["text"], Value::Text("-0.25".into()));
        assert_eq!(r["trunc"], Value::Int(2));
    }

    /// Mixing Fix and Int without a conversion is a TYPE ERROR — the two
    /// numeric worlds only meet through fix/trunc.
    #[test]
    fn fix_and_int_do_not_mix_silently() {
        let body = Term::Prim(PrimOp::FAdd, vec![Term::Fix(1), Term::Int(1)]);
        let def = Def {
            params: vec![],
            ret: Ty::Fix,
            effects: BTreeSet::new(),
            body,
            pre: None,
            post: None,
        };
        let m = Module::build(vec![def], 0).unwrap();
        assert!(matches!(verify_module(&m), Err(WeftError::Type(_))));
    }

    fn rec(fields: Vec<(&str, Term)>) -> Term {
        Term::Rec(
            fields
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        )
    }
    fn rec_ty(fields: Vec<(&str, Ty)>) -> Ty {
        Ty::Record(
            fields
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        )
    }

    /// The Meadow's chop-a-tree interaction, as a Weft behavior:
    /// `(state {hits: Int}, event {}) → {state, actions}` — three chops fell
    /// the tree, granting wood, a message, and a despawn.
    fn chop_module() -> Module {
        let state_ty = rec_ty(vec![("hits", Ty::Int)]);
        let out_ty = rec_ty(vec![
            ("actions", Ty::List(Box::new(Ty::Action))),
            ("state", state_ty.clone()),
        ]);
        // params: [state (Var 1), event (Var 0)]
        let hits_plus_one = Term::Prim(
            PrimOp::Add,
            vec![
                Term::Get(Box::new(Term::Var(1)), "hits".into()),
                Term::Int(1),
            ],
        );
        let body = Term::Let(
            Box::new(hits_plus_one), // Var 0 = new hit count (state→2, event→1)
            Box::new(Term::If(
                Box::new(Term::Prim(PrimOp::Le, vec![Term::Int(3), Term::Var(0)])),
                // Felled: give wood ×3, message, despawn; reset hits.
                Box::new(rec(vec![
                    (
                        "actions",
                        Term::ListNew(vec![
                            Term::Effect(
                                EffectKind::GiveItem,
                                [
                                    ("item".to_string(), Term::Int(20100001)),
                                    ("count".to_string(), Term::Int(3)),
                                ]
                                .into(),
                            ),
                            Term::Effect(
                                EffectKind::Notify,
                                [("text".to_string(), Term::Text("The tree falls.".into()))].into(),
                            ),
                            Term::Effect(EffectKind::Despawn, BTreeMap::new()),
                        ]),
                    ),
                    ("state", rec(vec![("hits", Term::Int(0))])),
                ])),
                // Not yet: keep counting.
                Box::new(rec(vec![
                    ("actions", Term::ListNew(vec![])),
                    ("state", rec(vec![("hits", Term::Var(0))])),
                ])),
            )),
        );
        let def = Def {
            params: vec![state_ty, rec_ty(vec![])],
            ret: out_ty,
            effects: [
                EffectKind::GiveItem,
                EffectKind::Notify,
                EffectKind::Despawn,
            ]
            .into(),
            body,
            pre: None,
            post: None,
        };
        Module::build(vec![def], 0).unwrap()
    }

    fn state(hits: i64) -> Value {
        Value::Rec([("hits".to_string(), Value::Int(hits))].into())
    }
    fn event() -> Value {
        Value::Rec(BTreeMap::new())
    }

    #[test]
    fn identity_is_content_and_only_content() {
        let a = chop_module();
        let b = chop_module();
        assert_eq!(a.entry, b.entry, "same structure, same hash — always");
        // One constant changed → a different program, a different identity.
        let mut defs: Vec<Def> = a.defs.values().cloned().collect();
        if let Term::Let(_, body) = &mut defs[0].body {
            if let Term::If(cond, _, _) = body.as_mut() {
                **cond = Term::Prim(PrimOp::Le, vec![Term::Int(4), Term::Var(0)]);
            }
        }
        let c = Module::build(defs, 0).unwrap();
        assert_ne!(a.entry, c.entry);
        // And the rendering is stable + prefixed.
        assert!(a.entry.to_string().starts_with("weft:"));
    }

    #[test]
    fn the_chop_behavior_verifies_and_runs() {
        let m = chop_module();
        let certs = verify_module(&m).expect("verifies");
        let cert = &certs[&m.entry];
        assert_eq!(
            cert.effects,
            [
                EffectKind::GiveItem,
                EffectKind::Notify,
                EffectKind::Despawn
            ]
            .into()
        );
        // Two chops: nothing yet, hits accumulate.
        let r = eval_call(&m, m.entry, vec![state(1), event()], cert.fuel_bound).unwrap();
        let Value::Rec(out) = &r.value else { panic!() };
        assert_eq!(out["actions"], Value::List(vec![]));
        assert_eq!(out["state"], state(2));
        // Third chop: the tree falls.
        let r = eval_call(&m, m.entry, vec![state(2), event()], cert.fuel_bound).unwrap();
        let Value::Rec(out) = &r.value else { panic!() };
        let Value::List(actions) = &out["actions"] else {
            panic!()
        };
        assert_eq!(actions.len(), 3);
        assert!(matches!(
            &actions[0],
            Value::Action {
                kind: EffectKind::GiveItem,
                ..
            }
        ));
        assert_eq!(out["state"], state(0));
    }

    #[test]
    fn the_effect_row_is_the_permission() {
        let m = chop_module();
        let mut defs: Vec<Def> = m.defs.values().cloned().collect();
        // Strip Despawn from the declared row; the body still constructs it.
        defs[0].effects.remove(&EffectKind::Despawn);
        let m2 = Module::build(defs, 0).unwrap();
        assert_eq!(
            verify_module(&m2).unwrap_err(),
            WeftError::EffectNotDeclared(EffectKind::Despawn),
        );
        // And a behavior CANNOT quietly gain commerce: constructing a buy
        // without declaring it is rejected the same way.
        let sneaky = Def {
            params: vec![],
            ret: Ty::Action,
            effects: BTreeSet::new(),
            body: Term::Effect(EffectKind::CommerceBuy, BTreeMap::new()),
            pre: None,
            post: None,
        };
        let m3 = Module::build(vec![sneaky], 0).unwrap();
        assert_eq!(
            verify_module(&m3).unwrap_err(),
            WeftError::EffectNotDeclared(EffectKind::CommerceBuy),
        );
    }

    #[test]
    fn type_errors_are_rejected() {
        // if on an Int condition
        let bad = Def {
            params: vec![],
            ret: Ty::Int,
            effects: BTreeSet::new(),
            body: Term::If(
                Box::new(Term::Int(1)),
                Box::new(Term::Int(2)),
                Box::new(Term::Int(3)),
            ),
            pre: None,
            post: None,
        };
        let m = Module::build(vec![bad], 0).unwrap();
        assert!(matches!(verify_module(&m), Err(WeftError::Type(_))));
    }

    #[test]
    fn static_fuel_bound_dominates_actual_cost() {
        let m = chop_module();
        let cert = &verify_module(&m).unwrap()[&m.entry];
        for hits in 0..5 {
            let r = eval_call(&m, m.entry, vec![state(hits), event()], cert.fuel_bound).unwrap();
            assert!(
                r.fuel_spent <= cert.fuel_bound,
                "spent {} > bound {}",
                r.fuel_spent,
                cert.fuel_bound
            );
        }
        // Determinism: same inputs, same fuel, same value — every time.
        let a = eval_call(&m, m.entry, vec![state(2), event()], cert.fuel_bound).unwrap();
        let b = eval_call(&m, m.entry, vec![state(2), event()], cert.fuel_bound).unwrap();
        assert_eq!(a.value, b.value);
        assert_eq!(a.fuel_spent, b.fuel_spent);
    }

    #[test]
    fn calls_compose_by_hash_and_effects_flow_transitively() {
        // helper: double(x) = x + x  (pure)
        let double = Def {
            params: vec![Ty::Int],
            ret: Ty::Int,
            effects: BTreeSet::new(),
            body: Term::Prim(PrimOp::Add, vec![Term::Var(0), Term::Var(0)]),
            pre: None,
            post: None,
        };
        let dh = hash_def(&double);
        // shout(x): notify with double(x); declares Notify.
        let shout = Def {
            params: vec![Ty::Int],
            ret: Ty::Action,
            effects: [EffectKind::Notify].into(),
            body: Term::Effect(
                EffectKind::Notify,
                [("n".to_string(), Term::Call(dh, vec![Term::Var(0)]))].into(),
            ),
            pre: None,
            post: None,
        };
        let m = Module::build(vec![double, shout], 1).unwrap();
        let certs = verify_module(&m).unwrap();
        assert!(certs[&m.entry].effects.contains(&EffectKind::Notify));
        let r = eval_call(
            &m,
            m.entry,
            vec![Value::Int(21)],
            certs[&m.entry].fuel_bound,
        )
        .unwrap();
        assert_eq!(
            r.value,
            Value::Action {
                kind: EffectKind::Notify,
                fields: [("n".to_string(), Value::Int(42))].into()
            }
        );
        // A caller that FORGETS the callee's effect is rejected: rows are
        // transitive, not copy-paste.
        let sneaky = Def {
            params: vec![Ty::Int],
            ret: Ty::Action,
            effects: BTreeSet::new(), // declares nothing
            body: Term::Call(m.entry, vec![Term::Var(0)]),
            pre: None,
            post: None,
        };
        let mut defs: Vec<Def> = m.defs.values().cloned().collect();
        defs.push(sneaky);
        let m2 = Module::build(defs, 2).unwrap();
        assert_eq!(
            verify_module(&m2).unwrap_err(),
            WeftError::EffectNotDeclared(EffectKind::Notify)
        );
    }

    #[test]
    fn contracts_gate_execution() {
        // inc(x) with pre: 0 <= x, post: x < result
        let inc = Def {
            params: vec![Ty::Int],
            ret: Ty::Int,
            effects: BTreeSet::new(),
            body: Term::Prim(PrimOp::Add, vec![Term::Var(0), Term::Int(1)]),
            pre: Some(Term::Prim(PrimOp::Le, vec![Term::Int(0), Term::Var(0)])),
            // post ctx: [x, result] — result is innermost (Var 0), x is Var 1.
            post: Some(Term::Prim(PrimOp::Lt, vec![Term::Var(1), Term::Var(0)])),
        };
        let m = Module::build(vec![inc], 0).unwrap();
        let cert = &verify_module(&m).unwrap()[&m.entry];
        let ok = eval_call(&m, m.entry, vec![Value::Int(5)], cert.fuel_bound + 16).unwrap();
        assert_eq!(ok.value, Value::Int(6));
        let bad = eval_call(&m, m.entry, vec![Value::Int(-1)], cert.fuel_bound + 16);
        assert_eq!(bad.unwrap_err(), WeftError::ContractViolated("pre"));
    }

    #[test]
    fn list_cat_composes_and_stays_bounded() {
        // Composition: two lists of the same element type join.
        let cat = Def {
            params: vec![Ty::List(Box::new(Ty::Int)), Ty::List(Box::new(Ty::Int))],
            ret: Ty::List(Box::new(Ty::Int)),
            effects: BTreeSet::new(),
            body: Term::Prim(PrimOp::ListCat, vec![Term::Var(1), Term::Var(0)]),
            pre: None,
            post: None,
        };
        let m = Module::build(vec![cat], 0).unwrap();
        let out = eval_call(
            &m,
            m.entry,
            vec![
                Value::List(vec![Value::Int(1), Value::Int(2)]),
                Value::List(vec![Value::Int(3)]),
            ],
            10_000,
        )
        .unwrap();
        assert_eq!(
            out.value,
            Value::List(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
        );

        // Mismatched element types don't verify.
        let bad = Def {
            params: vec![Ty::List(Box::new(Ty::Int)), Ty::List(Box::new(Ty::Text))],
            ret: Ty::List(Box::new(Ty::Int)),
            effects: BTreeSet::new(),
            body: Term::Prim(PrimOp::ListCat, vec![Term::Var(1), Term::Var(0)]),
            pre: None,
            post: None,
        };
        assert!(
            Module::build(vec![bad], 0).is_err()
                || verify_module(
                    &Module::build(
                        vec![Def {
                            params: vec![Ty::List(Box::new(Ty::Int)), Ty::List(Box::new(Ty::Text))],
                            ret: Ty::List(Box::new(Ty::Int)),
                            effects: BTreeSet::new(),
                            body: Term::Prim(PrimOp::ListCat, vec![Term::Var(1), Term::Var(0)]),
                            pre: None,
                            post: None,
                        }],
                        0
                    )
                    .unwrap()
                )
                .is_err()
        );

        // Growth costs fuel: doubling a list inside a fold exhausts the
        // budget instead of exploding memory. (16 doublings of a 4-element
        // seed would be 262144 elements; the fuel charge stops it long before.)
        let doubling = Def {
            params: vec![Ty::List(Box::new(Ty::Int))],
            ret: Ty::List(Box::new(Ty::Int)),
            effects: BTreeSet::new(),
            body: Term::Fold {
                cap: 16,
                list: Term::Iota {
                    cap: 16,
                    count: Box::new(Term::Int(16)),
                }
                .into(),
                init: Box::new(Term::Var(0)),
                // acc = Var 1, element = Var 0 → acc ++ acc
                body: Box::new(Term::Prim(
                    PrimOp::ListCat,
                    vec![Term::Var(1), Term::Var(1)],
                )),
            },
            pre: None,
            post: None,
        };
        let m = Module::build(vec![doubling], 0).unwrap();
        let seed = Value::List(vec![Value::Int(0); 4]);
        let r = eval_call(&m, m.entry, vec![seed], 5_000);
        assert!(
            matches!(r, Err(WeftError::FuelExhausted)),
            "runaway list growth is stopped by fuel, not by memory: {r:?}"
        );
    }

    #[test]
    fn division_is_total() {
        let d = Def {
            params: vec![Ty::Int, Ty::Int],
            ret: Ty::Int,
            effects: BTreeSet::new(),
            body: Term::Prim(PrimOp::Div, vec![Term::Var(1), Term::Var(0)]),
            pre: None,
            post: None,
        };
        let m = Module::build(vec![d], 0).unwrap();
        let cert = &verify_module(&m).unwrap()[&m.entry];
        let r = eval_call(
            &m,
            m.entry,
            vec![Value::Int(7), Value::Int(0)],
            cert.fuel_bound,
        )
        .unwrap();
        assert_eq!(r.value, Value::Int(0), "x/0 = 0 — no traps exist in Weft");
    }
}
