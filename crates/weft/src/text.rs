//! **The `.weft` text surface** — a faithful textual projection of a package
//! and the parser that takes it back.
//!
//! Weft has no source text (spec §1.1): the graph is the ground truth and
//! names are metadata. This module is the *agent-facing* projection: a small,
//! unambiguous syntax for the v0 core so that an author (a person, or far
//! more often an agent) can write a definition as text, `parse` it into a
//! canonical pack, and read any pack back with `fmt`.
//!
//! Guarantees (tested in [`tests`] and by `crates/weft-cli`):
//!
//! - `parse(fmt(P)) == P` for every verifiable package `P` — the text loses
//!   nothing the graph holds.
//! - `fmt(parse(T)) == T` for every *canonical* text `T` (a text `fmt`
//!   produced). Hand-written text is canonical after one `fmt`: binder and
//!   parameter names are not part of the graph, so `fmt` regenerates them
//!   (`a b c …` by depth, `result` in `ensures`), drops comments, and lays
//!   the code out deterministically. Definition names survive **only** as
//!   export petnames; an unexported definition is rendered `_<16 hex of its
//!   hash>`.
//!
//! The grammar is documented in `crates/weft/README.md`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use crate::pack::Package;
use crate::{hash_def, Def, EffectKind, PrimOp, Term, Ty, WeftHash, FIX_SCALE};

/// Line width the pretty-printer aims for (indent included).
pub const WIDTH: usize = 96;

// ---------------------------------------------------------------------------
// Vocabulary shared by both directions
// ---------------------------------------------------------------------------

const KEYWORDS: &[&str] = &[
    "package", "export", "alias", "def", "effects", "requires", "ensures", "let", "in", "if",
    "then", "else", "map", "fold", "iota", "true", "false", "and", "or", "not", "result",
];

/// Prim spelled as a function call: `name(args)`.
const PRIM_FNS: &[(&str, PrimOp)] = &[
    ("text", PrimOp::ToText),
    ("fix_text", PrimOp::FixToText),
    ("fix", PrimOp::FixOfInt),
    ("trunc", PrimOp::IntOfFix),
    ("len", PrimOp::Len),
    ("sin", PrimOp::FSin),
    ("cos", PrimOp::FCos),
    ("cat", PrimOp::ListCat),
];

/// Binary operators: spelling, op, precedence (higher binds tighter).
/// Precedence 4 (comparisons) is non-associative; the rest are left.
const BINOPS: &[(&str, PrimOp, u8)] = &[
    ("or", PrimOp::Or, 1),
    ("and", PrimOp::And, 2),
    ("<", PrimOp::Lt, 4),
    ("<=", PrimOp::Le, 4),
    ("==", PrimOp::EqInt, 4),
    ("<.", PrimOp::FLt, 4),
    ("<=.", PrimOp::FLe, 4),
    ("==.", PrimOp::EqFix, 4),
    ("==$", PrimOp::EqText, 4),
    ("++", PrimOp::Concat, 5),
    ("+", PrimOp::Add, 6),
    ("-", PrimOp::Sub, 6),
    ("+.", PrimOp::FAdd, 6),
    ("-.", PrimOp::FSub, 6),
    ("*", PrimOp::Mul, 7),
    ("/", PrimOp::Div, 7),
    ("*.", PrimOp::FMul, 7),
    ("/.", PrimOp::FDiv, 7),
];
const PREC_NOT: u8 = 3;
const PREC_ATOM: u8 = 9;

const EFFECTS: &[(&str, EffectKind)] = &[
    ("notify", EffectKind::Notify),
    ("navigate", EffectKind::Navigate),
    ("codex_open", EffectKind::CodexOpen),
    ("commerce_buy", EffectKind::CommerceBuy),
    ("presence_emit", EffectKind::PresenceEmit),
    ("set_state", EffectKind::SetState),
    ("give_item", EffectKind::GiveItem),
    ("despawn", EffectKind::Despawn),
    ("spawn", EffectKind::Spawn),
];

const TYPE_NAMES: &[&str] = &["Int", "Bool", "Text", "Fix", "Action", "List"];

fn effect_name(k: EffectKind) -> &'static str {
    EFFECTS
        .iter()
        .find(|(_, e)| *e == k)
        .map(|(n, _)| *n)
        .expect("every effect is named")
}
fn effect_by_name(n: &str) -> Option<EffectKind> {
    EFFECTS.iter().find(|(s, _)| *s == n).map(|(_, e)| *e)
}
fn prim_fn_name(op: PrimOp) -> Option<&'static str> {
    PRIM_FNS.iter().find(|(_, p)| *p == op).map(|(n, _)| *n)
}
fn prim_fn_by_name(n: &str) -> Option<PrimOp> {
    PRIM_FNS.iter().find(|(s, _)| *s == n).map(|(_, p)| *p)
}
fn binop(op: PrimOp) -> Option<(&'static str, u8)> {
    BINOPS
        .iter()
        .find(|(_, p, _)| *p == op)
        .map(|(s, _, pr)| (*s, *pr))
}
fn binop_by_name(n: &str) -> Option<(PrimOp, u8)> {
    BINOPS
        .iter()
        .find(|(s, _, _)| *s == n)
        .map(|(_, p, pr)| (*p, *pr))
}

fn is_reserved(s: &str) -> bool {
    KEYWORDS.contains(&s)
        || TYPE_NAMES.contains(&s)
        || prim_fn_by_name(s).is_some()
        || effect_by_name(s).is_some()
}

/// `ident := [A-Za-z_][A-Za-z0-9_]* ( '-' [A-Za-z_][A-Za-z0-9_]* )*` — hyphens
/// are allowed *inside* a name (petnames like `ring-point`) when followed by a
/// letter, so `a-b` is one name and `a - b` is a subtraction.
pub fn is_ident(s: &str) -> bool {
    let mut chars = s.chars().peekable();
    let Some(c0) = chars.next() else { return false };
    if !(c0.is_ascii_alphabetic() || c0 == '_') {
        return false;
    }
    let mut prev_hyphen = false;
    for c in chars {
        if c == '-' {
            if prev_hyphen {
                return false;
            }
            prev_hyphen = true;
        } else if c.is_ascii_alphanumeric() || c == '_' {
            if prev_hyphen && !(c.is_ascii_alphabetic() || c == '_') {
                return false;
            }
            prev_hyphen = false;
        } else {
            return false;
        }
    }
    !prev_hyphen && !is_reserved(s)
}

/// Binder names by depth: `a b c … z aa ab …`; a name that would collide
/// with the vocabulary gets a trailing underscore.
fn binder_name(depth: usize) -> String {
    let mut n = depth;
    let mut s = String::new();
    loop {
        s.insert(0, (b'a' + (n % 26) as u8) as char);
        if n < 26 {
            break;
        }
        n = n / 26 - 1;
    }
    if is_reserved(&s) {
        s.push('_');
    }
    s
}

/// The name each definition carries in text: its first export petname (in
/// petname order), else `_` + the first 16 hex characters of its hash.
pub fn def_names(pkg: &Package) -> BTreeMap<WeftHash, String> {
    let mut names: BTreeMap<WeftHash, String> = BTreeMap::new();
    for (pet, h) in &pkg.exports {
        names.entry(*h).or_insert_with(|| pet.clone());
    }
    for h in pkg.defs.keys() {
        names
            .entry(*h)
            .or_insert_with(|| format!("_{}", &h.to_string()[5..21]));
    }
    names
}

// ---------------------------------------------------------------------------
// fmt — package → text
// ---------------------------------------------------------------------------

/// Render a package as canonical `.weft` text. Fails only when a petname or
/// the package name cannot be spelled in the grammar.
pub fn fmt(pkg: &Package) -> Result<String, String> {
    for pet in pkg.exports.keys() {
        if !is_ident(pet) {
            return Err(format!("export petname {pet:?} is not a valid identifier"));
        }
    }
    for (pet, h) in &pkg.exports {
        if !pkg.defs.contains_key(h) {
            return Err(format!("export '{pet}' points outside the package"));
        }
    }
    let names = def_names(pkg);
    let mut out = String::new();
    if is_ident(&pkg.name) {
        let _ = writeln!(out, "package {}", pkg.name);
    } else {
        let _ = writeln!(out, "package {}", quote(&pkg.name));
    }
    // Aliases: a hash exported under more than one petname.
    for (pet, h) in &pkg.exports {
        let canonical = &names[h];
        if canonical != pet {
            let _ = writeln!(out, "export alias {pet} = {canonical}");
        }
    }
    let mut order: Vec<(&String, &WeftHash)> = pkg.defs.keys().map(|h| (&names[h], h)).collect();
    order.sort();
    let p = Printer { names: &names };
    for (name, h) in order {
        let d = &pkg.defs[h];
        out.push('\n');
        out.push_str(&p.def(name, pkg.exports.values().any(|x| x == h), d));
    }
    Ok(out)
}

fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                let _ = write!(out, "\\u{{{:x}}}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn fix_literal(v: i64) -> String {
    let sign = if v < 0 { "-" } else { "" };
    let mag = (v as i128).unsigned_abs();
    let (w, f) = (mag / FIX_SCALE as u128, mag % FIX_SCALE as u128);
    if f == 0 {
        format!("{sign}{w}.0")
    } else {
        format!("{sign}{w}.{}", format!("{f:06}").trim_end_matches('0'))
    }
}

pub fn ty_text(t: &Ty) -> String {
    match t {
        Ty::Int => "Int".into(),
        Ty::Bool => "Bool".into(),
        Ty::Text => "Text".into(),
        Ty::Fix => "Fix".into(),
        Ty::Action => "Action".into(),
        Ty::List(e) => format!("List {}", ty_text(e)),
        Ty::Record(fs) => {
            let fields: Vec<String> = fs
                .iter()
                .map(|(k, v)| format!("{k}: {}", ty_text(v)))
                .collect();
            format!("{{{}}}", fields.join(", "))
        }
    }
}

struct Printer<'a> {
    names: &'a BTreeMap<WeftHash, String>,
}

/// Naming context while rendering a term: how many binders are in scope and
/// which index (if any) is the contract's `result`.
#[derive(Clone, Copy)]
struct Scope {
    depth: usize,
    result_at: Option<usize>,
}

impl Scope {
    fn name_at(&self, k: usize) -> String {
        if self.result_at == Some(k) {
            "result".into()
        } else {
            binder_name(k)
        }
    }
    fn push(&self) -> Scope {
        Scope {
            depth: self.depth + 1,
            result_at: self.result_at,
        }
    }
}

fn prec_of(t: &Term) -> u8 {
    match t {
        Term::Prim(PrimOp::Not, a) if a.len() == 1 => PREC_NOT,
        Term::Prim(op, a) if a.len() == 2 => binop(*op).map(|(_, p)| p).unwrap_or(PREC_ATOM),
        Term::Let(..) | Term::If(..) => 0,
        _ => PREC_ATOM,
    }
}

fn indent(n: usize) -> String {
    " ".repeat(n)
}

/// A rendering is "flat" when it has one line that fits the width.
fn fits(s: &str, ind: usize) -> bool {
    !s.contains('\n') && s.chars().count() + ind <= WIDTH
}

impl<'a> Printer<'a> {
    fn def(&self, name: &str, exported: bool, d: &Def) -> String {
        let mut out = String::new();
        let params: Vec<String> = d
            .params
            .iter()
            .enumerate()
            .map(|(i, t)| format!("{}: {}", binder_name(i), ty_text(t)))
            .collect();
        let _ = writeln!(
            out,
            "{}def {name}({}) -> {}",
            if exported { "export " } else { "" },
            params.join(", "),
            ty_text(&d.ret)
        );
        if !d.effects.is_empty() {
            let effs: Vec<&str> = d.effects.iter().map(|e| effect_name(*e)).collect();
            let _ = writeln!(out, "  effects {}", effs.join(", "));
        }
        let scope = Scope {
            depth: d.params.len(),
            result_at: None,
        };
        if let Some(pre) = &d.pre {
            let _ = writeln!(out, "  requires {}", self.expr(pre, scope, 11));
        }
        if let Some(post) = &d.post {
            let ps = Scope {
                depth: d.params.len() + 1,
                result_at: Some(d.params.len()),
            };
            let _ = writeln!(out, "  ensures {}", self.expr(post, ps, 10));
        }
        let _ = writeln!(out, "  = {}", self.expr(&d.body, scope, 4));
        out
    }

    /// Render a child that must parenthesize when its precedence is too low.
    fn child(&self, t: &Term, s: Scope, ind: usize, parens: bool) -> String {
        if parens {
            let inner = self.expr(t, s, ind + 1);
            format!("({inner})")
        } else {
            self.expr(t, s, ind)
        }
    }

    /// Render `items` either flat (`open a, b close`) or one per line.
    fn seq(&self, open: &str, items: Vec<String>, close: &str, ind: usize) -> String {
        let flat = format!("{open}{}{close}", items.join(", "));
        if fits(&flat, ind) {
            return flat;
        }
        let mut out = String::from(open);
        for it in items {
            out.push('\n');
            out.push_str(&indent(ind + 2));
            out.push_str(&it);
            out.push(',');
        }
        out.push('\n');
        out.push_str(&indent(ind));
        out.push_str(close);
        out
    }

    /// Render one term with continuation lines indented by `ind`.
    fn expr(&self, t: &Term, s: Scope, ind: usize) -> String {
        match t {
            Term::Int(v) => v.to_string(),
            Term::Fix(v) => fix_literal(*v),
            Term::Bool(b) => b.to_string(),
            Term::Text(x) => quote(x),
            Term::Var(i) => match s.depth.checked_sub(1 + *i as usize) {
                Some(k) => s.name_at(k),
                None => format!("?var{i}"),
            },
            Term::Let(v, b) => {
                let name = binder_name(s.depth);
                let head_ind = ind + 4 + name.len() + 3;
                let val = self.expr(v, s, head_ind);
                let body = self.expr(b, s.push(), ind);
                format!("let {name} = {val} in\n{}{body}", indent(ind))
            }
            Term::If(c, a, b) => {
                let cond = self.expr(c, s, ind + 3);
                let then = self.expr(a, s, ind + 2);
                let flat_else = self.expr(b, s, ind + 2);
                let flat = format!("if {cond} then {then} else {flat_else}");
                if fits(&flat, ind) {
                    return flat;
                }
                let mut out = format!(
                    "if {cond} then\n{}{then}\n{}else",
                    indent(ind + 2),
                    indent(ind)
                );
                if matches!(**b, Term::If(..)) {
                    // else-if chains stay at one indentation level.
                    out.push(' ');
                    out.push_str(&self.expr(b, s, ind));
                } else {
                    out.push('\n');
                    out.push_str(&indent(ind + 2));
                    out.push_str(&flat_else);
                }
                out
            }
            Term::Prim(PrimOp::Not, args) if args.len() == 1 => {
                let p = prec_of(&args[0]);
                format!("not {}", self.child(&args[0], s, ind + 4, p < 4))
            }
            Term::Prim(op, args) if args.len() == 2 && binop(*op).is_some() => {
                let (sym, prec) = binop(*op).expect("checked");
                let (lp, rp) = (prec_of(&args[0]), prec_of(&args[1]));
                let nonassoc = prec == 4;
                let l = self.child(&args[0], s, ind, lp < prec || (nonassoc && lp == prec));
                let r = self.child(&args[1], s, ind + 2, rp <= prec);
                let flat = format!("{l} {sym} {r}");
                if fits(&flat, ind) {
                    flat
                } else {
                    format!("{l}\n{}{sym} {r}", indent(ind + 2))
                }
            }
            Term::Prim(op, args) => {
                let name = prim_fn_name(*op).unwrap_or("?prim");
                let items: Vec<String> = args.iter().map(|a| self.expr(a, s, ind + 2)).collect();
                self.seq(&format!("{name}("), items, ")", ind)
            }
            Term::Rec(fields) => {
                let items: Vec<String> = fields
                    .iter()
                    .map(|(k, v)| format!("{k}: {}", self.expr(v, s, ind + 2 + k.len() + 2)))
                    .collect();
                self.seq("{", items, "}", ind)
            }
            Term::Get(r, k) => {
                let p = prec_of(r);
                format!("{}.{k}", self.child(r, s, ind, p < PREC_ATOM))
            }
            Term::ListNew(items) => {
                let items: Vec<String> = items.iter().map(|i| self.expr(i, s, ind + 2)).collect();
                self.seq("[", items, "]", ind)
            }
            Term::Call(h, args) => {
                let name = self
                    .names
                    .get(h)
                    .cloned()
                    .unwrap_or_else(|| format!("_{}", &h.to_string()[5..21]));
                let items: Vec<String> = args.iter().map(|a| self.expr(a, s, ind + 2)).collect();
                self.seq(&format!("{name}("), items, ")", ind)
            }
            Term::Effect(kind, fields) => {
                let name = effect_name(*kind);
                let items: Vec<String> = fields
                    .iter()
                    .map(|(k, v)| format!("{k}: {}", self.expr(v, s, ind + 2 + k.len() + 2)))
                    .collect();
                self.seq(&format!("{name}{{"), items, "}", ind)
            }
            Term::Map { cap, list, body } => {
                let var = binder_name(s.depth);
                let items = vec![
                    self.expr(list, s, ind + 2),
                    format!("{var} -> {}", self.expr(body, s.push(), ind + 2)),
                ];
                self.seq(&format!("map[{cap}]("), items, ")", ind)
            }
            Term::Fold {
                cap,
                list,
                init,
                body,
            } => {
                let acc = binder_name(s.depth);
                let var = binder_name(s.depth + 1);
                let items = vec![
                    self.expr(list, s, ind + 2),
                    self.expr(init, s, ind + 2),
                    format!(
                        "{acc} {var} -> {}",
                        self.expr(body, s.push().push(), ind + 2)
                    ),
                ];
                self.seq(&format!("fold[{cap}]("), items, ")", ind)
            }
            Term::Iota { cap, count } => {
                let items = vec![self.expr(count, s, ind + 2)];
                self.seq(&format!("iota[{cap}]("), items, ")", ind)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// parse — text → package
// ---------------------------------------------------------------------------

/// A parse (or resolution) failure with its line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}
impl std::error::Error for ParseError {}

fn err<T>(line: usize, message: impl Into<String>) -> Result<T, ParseError> {
    Err(ParseError {
        line,
        message: message.into(),
    })
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    Int(i128),
    Fix(i64),
    Str(String),
    Sym(&'static str),
    Eof,
}

impl std::fmt::Display for Tok {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Tok::Ident(s) => write!(f, "'{s}'"),
            Tok::Int(v) => write!(f, "integer {v}"),
            Tok::Fix(v) => write!(f, "fixed-point {}", fix_literal(*v)),
            Tok::Str(s) => write!(f, "text {}", quote(s)),
            Tok::Sym(s) => write!(f, "'{s}'"),
            Tok::Eof => write!(f, "end of file"),
        }
    }
}

const SYMS: &[&str] = &[
    "==$", "<=.", "==.", "->", "==", "<=", "++", "+.", "-.", "*.", "/.", "<.", "(", ")", "[", "]",
    "{", "}", ",", ":", ".", "=", "<", "+", "-", "*", "/",
];

fn lex(src: &str) -> Result<Vec<(Tok, usize)>, ParseError> {
    let chars: Vec<char> = src.chars().collect();
    let mut toks = Vec::new();
    let (mut i, mut line) = (0usize, 1usize);
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            line += 1;
            i += 1;
            continue;
        }
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() {
                let d = chars[i];
                if d.is_ascii_alphanumeric() || d == '_' {
                    i += 1;
                } else if d == '-'
                    && chars
                        .get(i + 1)
                        .map(|n| n.is_ascii_alphabetic() || *n == '_')
                        .unwrap_or(false)
                {
                    i += 1;
                } else {
                    break;
                }
            }
            toks.push((Tok::Ident(chars[start..i].iter().collect()), line));
            continue;
        }
        if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            let whole: String = chars[start..i].iter().collect();
            if chars.get(i) == Some(&'.')
                && chars
                    .get(i + 1)
                    .map(|d| d.is_ascii_digit())
                    .unwrap_or(false)
            {
                i += 1;
                let fs = i;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
                let frac: String = chars[fs..i].iter().collect();
                if frac.len() > 6 {
                    return err(
                        line,
                        format!("fixed-point literal {whole}.{frac} has more than 6 decimals"),
                    );
                }
                let w: i128 = whole.parse().map_err(|_| ParseError {
                    line,
                    message: format!("integer part {whole} is too large"),
                })?;
                let f: i128 = format!("{frac:0<6}").parse().expect("digits");
                let raw = w * FIX_SCALE as i128 + f;
                if raw > i64::MAX as i128 {
                    return err(line, "fixed-point literal out of range");
                }
                toks.push((Tok::Fix(raw as i64), line));
            } else {
                let v: i128 = whole.parse().map_err(|_| ParseError {
                    line,
                    message: format!("integer {whole} is too large"),
                })?;
                toks.push((Tok::Int(v), line));
            }
            continue;
        }
        if c == '"' {
            i += 1;
            let mut s = String::new();
            loop {
                let Some(&d) = chars.get(i) else {
                    return err(line, "unterminated text literal");
                };
                i += 1;
                match d {
                    '"' => break,
                    '\n' => return err(line, "newline inside text literal (use \\n)"),
                    '\\' => {
                        let Some(&e) = chars.get(i) else {
                            return err(line, "unterminated escape");
                        };
                        i += 1;
                        match e {
                            '"' => s.push('"'),
                            '\\' => s.push('\\'),
                            'n' => s.push('\n'),
                            'r' => s.push('\r'),
                            't' => s.push('\t'),
                            'u' => {
                                if chars.get(i) != Some(&'{') {
                                    return err(line, "expected \\u{hex}");
                                }
                                i += 1;
                                let hs = i;
                                while i < chars.len() && chars[i] != '}' {
                                    i += 1;
                                }
                                let hex: String = chars[hs..i].iter().collect();
                                i += 1;
                                let cp = u32::from_str_radix(&hex, 16)
                                    .ok()
                                    .and_then(char::from_u32)
                                    .ok_or_else(|| ParseError {
                                        line,
                                        message: format!("bad escape \\u{{{hex}}}"),
                                    })?;
                                s.push(cp);
                            }
                            other => return err(line, format!("unknown escape \\{other}")),
                        }
                    }
                    d => s.push(d),
                }
            }
            toks.push((Tok::Str(s), line));
            continue;
        }
        let rest: String = chars[i..chars.len().min(i + 3)].iter().collect();
        match SYMS.iter().find(|s| rest.starts_with(**s)) {
            Some(s) => {
                toks.push((Tok::Sym(s), line));
                i += s.chars().count();
            }
            None => return err(line, format!("unexpected character {c:?}")),
        }
    }
    // End of file reports the last token's line — where the author is looking.
    let last = toks.last().map(|t| t.1).unwrap_or(line);
    toks.push((Tok::Eof, last));
    Ok(toks)
}

/// The surface tree: names instead of indices, names instead of hashes.
#[derive(Debug, Clone)]
enum SExpr {
    Int(i64),
    Fix(i64),
    Bool(bool),
    Text(String),
    Var(String, usize),
    Let(String, Box<SExpr>, Box<SExpr>),
    If(Box<SExpr>, Box<SExpr>, Box<SExpr>),
    Prim(PrimOp, Vec<SExpr>),
    Rec(BTreeMap<String, SExpr>),
    Get(Box<SExpr>, String),
    List(Vec<SExpr>),
    Call(String, Vec<SExpr>, usize),
    Effect(EffectKind, BTreeMap<String, SExpr>),
    Map {
        cap: u32,
        list: Box<SExpr>,
        var: String,
        body: Box<SExpr>,
    },
    Fold {
        cap: u32,
        list: Box<SExpr>,
        init: Box<SExpr>,
        acc: String,
        var: String,
        body: Box<SExpr>,
    },
    Iota {
        cap: u32,
        count: Box<SExpr>,
    },
}

struct SDef {
    name: String,
    export: bool,
    params: Vec<(String, Ty)>,
    ret: Ty,
    effects: BTreeSet<EffectKind>,
    pre: Option<SExpr>,
    post: Option<SExpr>,
    body: SExpr,
    line: usize,
}

struct Parser {
    toks: Vec<(Tok, usize)>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.toks[self.pos].0
    }
    fn line(&self) -> usize {
        self.toks[self.pos].1
    }
    fn next(&mut self) -> Tok {
        let t = self.toks[self.pos].0.clone();
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        t
    }
    fn is_sym(&self, s: &str) -> bool {
        matches!(self.peek(), Tok::Sym(x) if *x == s)
    }
    fn is_kw(&self, s: &str) -> bool {
        matches!(self.peek(), Tok::Ident(x) if x == s)
    }
    fn eat_sym(&mut self, s: &str) -> bool {
        if self.is_sym(s) {
            self.next();
            true
        } else {
            false
        }
    }
    fn eat_kw(&mut self, s: &str) -> bool {
        if self.is_kw(s) {
            self.next();
            true
        } else {
            false
        }
    }
    fn expect_sym(&mut self, s: &str) -> Result<(), ParseError> {
        if self.eat_sym(s) {
            Ok(())
        } else {
            err(
                self.line(),
                format!("expected '{s}', found {}", self.peek()),
            )
        }
    }
    fn expect_kw(&mut self, s: &str) -> Result<(), ParseError> {
        if self.eat_kw(s) {
            Ok(())
        } else {
            err(
                self.line(),
                format!("expected '{s}', found {}", self.peek()),
            )
        }
    }
    /// A user-chosen name: identifier that is not reserved.
    fn name(&mut self, what: &str) -> Result<String, ParseError> {
        let line = self.line();
        match self.next() {
            Tok::Ident(s) if is_ident(&s) => Ok(s),
            Tok::Ident(s) => err(line, format!("'{s}' is reserved and cannot name a {what}")),
            other => err(line, format!("expected a {what} name, found {other}")),
        }
    }
    /// A record/effect field label: any identifier (field names are data,
    /// so `text`, `fix` or `result` are all fine here).
    fn label(&mut self) -> Result<String, ParseError> {
        let line = self.line();
        match self.next() {
            Tok::Ident(s) => Ok(s),
            other => err(line, format!("expected a field name, found {other}")),
        }
    }

    fn ty(&mut self) -> Result<Ty, ParseError> {
        let line = self.line();
        if self.eat_sym("{") {
            let mut fields = BTreeMap::new();
            while !self.is_sym("}") {
                let k = self.label()?;
                self.expect_sym(":")?;
                let t = self.ty()?;
                if fields.insert(k.clone(), t).is_some() {
                    return err(line, format!("duplicate field '{k}' in record type"));
                }
                if !self.eat_sym(",") {
                    break;
                }
            }
            self.expect_sym("}")?;
            return Ok(Ty::Record(fields));
        }
        match self.next() {
            Tok::Ident(s) => match s.as_str() {
                "Int" => Ok(Ty::Int),
                "Bool" => Ok(Ty::Bool),
                "Text" => Ok(Ty::Text),
                "Fix" => Ok(Ty::Fix),
                "Action" => Ok(Ty::Action),
                "List" => Ok(Ty::List(Box::new(self.ty()?))),
                other => err(line, format!("unknown type '{other}'")),
            },
            other => err(line, format!("expected a type, found {other}")),
        }
    }

    fn cap(&mut self) -> Result<u32, ParseError> {
        self.expect_sym("[")?;
        let line = self.line();
        let cap = match self.next() {
            Tok::Int(v) if v >= 0 && v <= u32::MAX as i128 => v as u32,
            other => return err(line, format!("expected a bound like [16], found {other}")),
        };
        self.expect_sym("]")?;
        Ok(cap)
    }

    fn args(&mut self) -> Result<Vec<SExpr>, ParseError> {
        // Called after the '(' has been consumed.
        let mut out = Vec::new();
        while !self.is_sym(")") {
            out.push(self.expr()?);
            if !self.eat_sym(",") {
                break;
            }
        }
        self.expect_sym(")")?;
        Ok(out)
    }

    fn fields(&mut self) -> Result<BTreeMap<String, SExpr>, ParseError> {
        // Called after the '{' has been consumed.
        let line = self.line();
        let mut out = BTreeMap::new();
        while !self.is_sym("}") {
            let k = self.label()?;
            self.expect_sym(":")?;
            let v = self.expr()?;
            if out.insert(k.clone(), v).is_some() {
                return err(line, format!("duplicate field '{k}'"));
            }
            if !self.eat_sym(",") {
                break;
            }
        }
        self.expect_sym("}")?;
        Ok(out)
    }

    /// expr := let | if | binary
    fn expr(&mut self) -> Result<SExpr, ParseError> {
        if self.eat_kw("let") {
            let name = self.name("binder")?;
            self.expect_sym("=")?;
            let v = self.expr()?;
            self.expect_kw("in")?;
            let b = self.expr()?;
            return Ok(SExpr::Let(name, Box::new(v), Box::new(b)));
        }
        if self.eat_kw("if") {
            let c = self.expr()?;
            self.expect_kw("then")?;
            let a = self.expr()?;
            self.expect_kw("else")?;
            let b = self.expr()?;
            return Ok(SExpr::If(Box::new(c), Box::new(a), Box::new(b)));
        }
        self.binary(1)
    }

    fn peek_binop(&self) -> Option<(&'static str, PrimOp, u8)> {
        let s = match self.peek() {
            Tok::Sym(s) => *s,
            Tok::Ident(w) if w == "and" => "and",
            Tok::Ident(w) if w == "or" => "or",
            _ => return None,
        };
        binop_by_name(s).map(|(op, p)| (s, op, p))
    }

    fn binary(&mut self, min: u8) -> Result<SExpr, ParseError> {
        let mut lhs = self.unary()?;
        loop {
            let Some((_, op, prec)) = self.peek_binop() else {
                break;
            };
            if prec < min {
                break;
            }
            let line = self.line();
            self.next();
            let rhs = self.binary(prec + 1)?;
            lhs = SExpr::Prim(op, vec![lhs, rhs]);
            if prec == 4 {
                if let Some((sym, _, 4)) = self.peek_binop() {
                    return err(
                        line,
                        format!("comparisons do not chain: parenthesize before '{sym}'"),
                    );
                }
            }
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> Result<SExpr, ParseError> {
        if self.eat_kw("not") {
            let e = self.binary(PREC_NOT + 1)?;
            return Ok(SExpr::Prim(PrimOp::Not, vec![e]));
        }
        self.postfix()
    }

    fn postfix(&mut self) -> Result<SExpr, ParseError> {
        let mut e = self.atom()?;
        while self.eat_sym(".") {
            let k = self.label()?;
            e = SExpr::Get(Box::new(e), k);
        }
        Ok(e)
    }

    fn atom(&mut self) -> Result<SExpr, ParseError> {
        let line = self.line();
        match self.next() {
            Tok::Int(v) => int_lit(line, v),
            Tok::Fix(v) => Ok(SExpr::Fix(v)),
            Tok::Str(s) => Ok(SExpr::Text(s)),
            Tok::Sym("-") => match self.next() {
                Tok::Int(v) => int_lit(line, -v),
                Tok::Fix(v) => Ok(SExpr::Fix(-v)),
                other => err(line, format!("expected a number after '-', found {other}")),
            },
            Tok::Sym("(") => {
                let e = self.expr()?;
                self.expect_sym(")")?;
                Ok(e)
            }
            Tok::Sym("[") => {
                let mut items = Vec::new();
                while !self.is_sym("]") {
                    items.push(self.expr()?);
                    if !self.eat_sym(",") {
                        break;
                    }
                }
                self.expect_sym("]")?;
                Ok(SExpr::List(items))
            }
            Tok::Sym("{") => Ok(SExpr::Rec(self.fields()?)),
            Tok::Ident(w) => match w.as_str() {
                "true" => Ok(SExpr::Bool(true)),
                "false" => Ok(SExpr::Bool(false)),
                "map" => {
                    let cap = self.cap()?;
                    self.expect_sym("(")?;
                    let list = self.expr()?;
                    self.expect_sym(",")?;
                    let var = self.name("binder")?;
                    self.expect_sym("->")?;
                    let body = self.expr()?;
                    self.eat_sym(",");
                    self.expect_sym(")")?;
                    Ok(SExpr::Map {
                        cap,
                        list: Box::new(list),
                        var,
                        body: Box::new(body),
                    })
                }
                "fold" => {
                    let cap = self.cap()?;
                    self.expect_sym("(")?;
                    let list = self.expr()?;
                    self.expect_sym(",")?;
                    let init = self.expr()?;
                    self.expect_sym(",")?;
                    let acc = self.name("accumulator")?;
                    let var = self.name("binder")?;
                    self.expect_sym("->")?;
                    let body = self.expr()?;
                    self.eat_sym(",");
                    self.expect_sym(")")?;
                    Ok(SExpr::Fold {
                        cap,
                        list: Box::new(list),
                        init: Box::new(init),
                        acc,
                        var,
                        body: Box::new(body),
                    })
                }
                "iota" => {
                    let cap = self.cap()?;
                    self.expect_sym("(")?;
                    let count = self.expr()?;
                    self.eat_sym(",");
                    self.expect_sym(")")?;
                    Ok(SExpr::Iota {
                        cap,
                        count: Box::new(count),
                    })
                }
                _ if effect_by_name(&w).is_some() => {
                    self.expect_sym("{")?;
                    Ok(SExpr::Effect(
                        effect_by_name(&w).expect("checked"),
                        self.fields()?,
                    ))
                }
                _ if prim_fn_by_name(&w).is_some() => {
                    let op = prim_fn_by_name(&w).expect("checked");
                    self.expect_sym("(")?;
                    let args = self.args()?;
                    let want = if op == PrimOp::ListCat { 2 } else { 1 };
                    if args.len() != want {
                        return err(
                            line,
                            format!("{w} takes {want} argument(s), got {}", args.len()),
                        );
                    }
                    Ok(SExpr::Prim(op, args))
                }
                _ if KEYWORDS.contains(&w.as_str()) && w != "result" => {
                    err(line, format!("unexpected keyword '{w}'"))
                }
                _ if TYPE_NAMES.contains(&w.as_str()) => {
                    err(line, format!("type '{w}' used as a value"))
                }
                _ => {
                    if self.eat_sym("(") {
                        let args = self.args()?;
                        Ok(SExpr::Call(w, args, line))
                    } else {
                        Ok(SExpr::Var(w, line))
                    }
                }
            },
            other => err(line, format!("unexpected {other}")),
        }
    }

    fn def(&mut self, export: bool) -> Result<SDef, ParseError> {
        let line = self.line();
        self.expect_kw("def")?;
        let name = self.name("definition")?;
        self.expect_sym("(")?;
        let mut params = Vec::new();
        while !self.is_sym(")") {
            let p = self.name("parameter")?;
            self.expect_sym(":")?;
            let t = self.ty()?;
            params.push((p, t));
            if !self.eat_sym(",") {
                break;
            }
        }
        self.expect_sym(")")?;
        self.expect_sym("->")?;
        let ret = self.ty()?;
        let mut effects = BTreeSet::new();
        if self.eat_kw("effects") {
            loop {
                let l = self.line();
                match self.next() {
                    Tok::Ident(e) => match effect_by_name(&e) {
                        Some(k) => {
                            effects.insert(k);
                        }
                        None => return err(l, format!("unknown effect '{e}'")),
                    },
                    other => return err(l, format!("expected an effect name, found {other}")),
                }
                if !self.eat_sym(",") {
                    break;
                }
            }
        }
        let mut pre = None;
        let mut post = None;
        loop {
            if self.eat_kw("requires") {
                if pre.is_some() {
                    return err(self.line(), "a definition has at most one 'requires'");
                }
                pre = Some(self.expr()?);
            } else if self.eat_kw("ensures") {
                if post.is_some() {
                    return err(self.line(), "a definition has at most one 'ensures'");
                }
                post = Some(self.expr()?);
            } else {
                break;
            }
        }
        self.expect_sym("=")?;
        let body = self.expr()?;
        Ok(SDef {
            name,
            export,
            params,
            ret,
            effects,
            pre,
            post,
            body,
            line,
        })
    }
}

fn int_lit(line: usize, v: i128) -> Result<SExpr, ParseError> {
    i64::try_from(v).map(SExpr::Int).map_err(|_| ParseError {
        line,
        message: format!("integer {v} does not fit in 64 bits"),
    })
}

/// Names of every definition an expression calls.
fn calls_of(e: &SExpr, out: &mut BTreeSet<String>) {
    match e {
        SExpr::Call(n, args, _) => {
            out.insert(n.clone());
            args.iter().for_each(|a| calls_of(a, out));
        }
        SExpr::Let(_, a, b) => {
            calls_of(a, out);
            calls_of(b, out);
        }
        SExpr::If(a, b, c) => {
            calls_of(a, out);
            calls_of(b, out);
            calls_of(c, out);
        }
        SExpr::Prim(_, xs) | SExpr::List(xs) => xs.iter().for_each(|x| calls_of(x, out)),
        SExpr::Rec(fs) | SExpr::Effect(_, fs) => fs.values().for_each(|x| calls_of(x, out)),
        SExpr::Get(r, _) => calls_of(r, out),
        SExpr::Map { list, body, .. } => {
            calls_of(list, out);
            calls_of(body, out);
        }
        SExpr::Fold {
            list, init, body, ..
        } => {
            calls_of(list, out);
            calls_of(init, out);
            calls_of(body, out);
        }
        SExpr::Iota { count, .. } => calls_of(count, out),
        _ => {}
    }
}

/// Lower a surface expression to a term: names → de Bruijn indices, calls →
/// hashes. `scope` is the binder stack (innermost last).
fn lower(
    e: &SExpr,
    scope: &mut Vec<String>,
    hashes: &BTreeMap<String, WeftHash>,
    known: &BTreeSet<String>,
) -> Result<Term, ParseError> {
    Ok(match e {
        SExpr::Int(v) => Term::Int(*v),
        SExpr::Fix(v) => Term::Fix(*v),
        SExpr::Bool(b) => Term::Bool(*b),
        SExpr::Text(s) => Term::Text(s.clone()),
        SExpr::Var(n, line) => match scope.iter().rposition(|s| s == n) {
            Some(k) => Term::Var((scope.len() - 1 - k) as u32),
            None => {
                let hint = if known.contains(n) {
                    format!(" ('{n}' is a definition — call it as {n}(...))")
                } else {
                    String::new()
                };
                return err(*line, format!("unbound name '{n}'{hint}"));
            }
        },
        SExpr::Let(n, v, b) => {
            let v = lower(v, scope, hashes, known)?;
            scope.push(n.clone());
            let b = lower(b, scope, hashes, known);
            scope.pop();
            Term::Let(Box::new(v), Box::new(b?))
        }
        SExpr::If(c, a, b) => Term::If(
            Box::new(lower(c, scope, hashes, known)?),
            Box::new(lower(a, scope, hashes, known)?),
            Box::new(lower(b, scope, hashes, known)?),
        ),
        SExpr::Prim(op, xs) => {
            let mut out = Vec::with_capacity(xs.len());
            for x in xs {
                out.push(lower(x, scope, hashes, known)?);
            }
            Term::Prim(*op, out)
        }
        SExpr::Rec(fs) => {
            let mut out = BTreeMap::new();
            for (k, v) in fs {
                out.insert(k.clone(), lower(v, scope, hashes, known)?);
            }
            Term::Rec(out)
        }
        SExpr::Get(r, k) => Term::Get(Box::new(lower(r, scope, hashes, known)?), k.clone()),
        SExpr::List(xs) => {
            let mut out = Vec::with_capacity(xs.len());
            for x in xs {
                out.push(lower(x, scope, hashes, known)?);
            }
            Term::ListNew(out)
        }
        SExpr::Call(n, xs, line) => {
            let h = *hashes.get(n).ok_or_else(|| ParseError {
                line: *line,
                message: format!("call to unknown definition '{n}'"),
            })?;
            let mut out = Vec::with_capacity(xs.len());
            for x in xs {
                out.push(lower(x, scope, hashes, known)?);
            }
            Term::Call(h, out)
        }
        SExpr::Effect(k, fs) => {
            let mut out = BTreeMap::new();
            for (n, v) in fs {
                out.insert(n.clone(), lower(v, scope, hashes, known)?);
            }
            Term::Effect(*k, out)
        }
        SExpr::Map {
            cap,
            list,
            var,
            body,
        } => {
            let list = lower(list, scope, hashes, known)?;
            scope.push(var.clone());
            let body = lower(body, scope, hashes, known);
            scope.pop();
            Term::Map {
                cap: *cap,
                list: Box::new(list),
                body: Box::new(body?),
            }
        }
        SExpr::Fold {
            cap,
            list,
            init,
            acc,
            var,
            body,
        } => {
            let list = lower(list, scope, hashes, known)?;
            let init = lower(init, scope, hashes, known)?;
            scope.push(acc.clone());
            scope.push(var.clone());
            let body = lower(body, scope, hashes, known);
            scope.pop();
            scope.pop();
            Term::Fold {
                cap: *cap,
                list: Box::new(list),
                init: Box::new(init),
                body: Box::new(body?),
            }
        }
        SExpr::Iota { cap, count } => Term::Iota {
            cap: *cap,
            count: Box::new(lower(count, scope, hashes, known)?),
        },
    })
}

/// Parse `.weft` text into a package. The result is **not** verified — run
/// [`Package::verify`] (the CLI does) to learn whether it types, what it
/// may do, and what it costs.
pub fn parse(src: &str) -> Result<Package, ParseError> {
    let toks = lex(src)?;
    let mut p = Parser { toks, pos: 0 };
    p.expect_kw("package")?;
    let line = p.line();
    let name = match p.next() {
        Tok::Ident(s) if is_ident(&s) => s,
        Tok::Str(s) => s,
        other => return err(line, format!("expected a package name, found {other}")),
    };
    let mut sdefs: Vec<SDef> = Vec::new();
    let mut aliases: Vec<(String, String, usize)> = Vec::new();
    loop {
        if matches!(p.peek(), Tok::Eof) {
            break;
        }
        let line = p.line();
        if p.eat_kw("export") {
            if p.eat_kw("alias") {
                let a = p.name("export")?;
                p.expect_sym("=")?;
                let b = p.name("definition")?;
                aliases.push((a, b, line));
            } else {
                sdefs.push(p.def(true)?);
            }
        } else if p.is_kw("def") {
            sdefs.push(p.def(false)?);
        } else {
            return err(
                line,
                format!("expected 'def' or 'export', found {}", p.peek()),
            );
        }
    }
    // Resolve in dependency order — a def's hash needs its callees' hashes.
    let mut by_name: BTreeMap<String, usize> = BTreeMap::new();
    for (i, d) in sdefs.iter().enumerate() {
        if by_name.insert(d.name.clone(), i).is_some() {
            return err(d.line, format!("definition '{}' is declared twice", d.name));
        }
    }
    let mut hashes: BTreeMap<String, WeftHash> = BTreeMap::new();
    let mut defs: BTreeMap<WeftHash, Def> = BTreeMap::new();
    let mut in_progress: Vec<String> = Vec::new();
    fn resolve(
        name: &str,
        sdefs: &[SDef],
        by_name: &BTreeMap<String, usize>,
        hashes: &mut BTreeMap<String, WeftHash>,
        defs: &mut BTreeMap<WeftHash, Def>,
        in_progress: &mut Vec<String>,
    ) -> Result<(), ParseError> {
        if hashes.contains_key(name) {
            return Ok(());
        }
        let d = &sdefs[by_name[name]];
        if in_progress.iter().any(|n| n == name) {
            let chain: Vec<&str> = in_progress
                .iter()
                .map(String::as_str)
                .chain([name])
                .collect();
            return err(
                d.line,
                format!(
                    "recursion is not expressible in Weft v0.1: {} (use fold over finite data)",
                    chain.join(" -> ")
                ),
            );
        }
        in_progress.push(name.to_string());
        let mut callees = BTreeSet::new();
        calls_of(&d.body, &mut callees);
        if let Some(pre) = &d.pre {
            calls_of(pre, &mut callees);
        }
        if let Some(post) = &d.post {
            calls_of(post, &mut callees);
        }
        for c in &callees {
            if !by_name.contains_key(c) {
                return err(
                    d.line,
                    format!("'{}' calls unknown definition '{c}'", d.name),
                );
            }
            resolve(c, sdefs, by_name, hashes, defs, in_progress)?;
        }
        in_progress.pop();
        let known: BTreeSet<String> = by_name.keys().cloned().collect();
        let mut scope: Vec<String> = d.params.iter().map(|(n, _)| n.clone()).collect();
        let body = lower(&d.body, &mut scope, hashes, &known)?;
        let pre = match &d.pre {
            Some(e) => Some(lower(e, &mut scope, hashes, &known)?),
            None => None,
        };
        let post = match &d.post {
            Some(e) => {
                scope.push("result".into());
                let t = lower(e, &mut scope, hashes, &known);
                scope.pop();
                Some(t?)
            }
            None => None,
        };
        let def = Def {
            params: d.params.iter().map(|(_, t)| t.clone()).collect(),
            ret: d.ret.clone(),
            effects: d.effects.clone(),
            body,
            pre,
            post,
        };
        let h = hash_def(&def);
        hashes.insert(name.to_string(), h);
        defs.insert(h, def);
        Ok(())
    }
    for d in &sdefs {
        resolve(
            &d.name,
            &sdefs,
            &by_name,
            &mut hashes,
            &mut defs,
            &mut in_progress,
        )?;
    }
    let mut exports = BTreeMap::new();
    for d in &sdefs {
        if d.export {
            exports.insert(d.name.clone(), hashes[&d.name]);
        }
    }
    for (a, b, line) in aliases {
        let Some(h) = hashes.get(&b) else {
            return err(line, format!("alias '{a}' names unknown definition '{b}'"));
        };
        if exports.insert(a.clone(), *h).is_some() {
            return err(line, format!("export '{a}' is declared twice"));
        }
    }
    Ok(Package {
        name,
        exports,
        defs,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn pkg_json(p: &Package) -> serde_json::Value {
        serde_json::to_value(p).unwrap()
    }

    /// `parse ∘ fmt = id` on the package and `fmt ∘ parse = id` on the text.
    fn assert_roundtrip(pkg: &Package) {
        pkg.verify().expect("input verifies");
        let text = fmt(pkg).expect("fmt");
        let back = parse(&text).unwrap_or_else(|e| panic!("{e}\n---\n{text}"));
        assert_eq!(
            pkg_json(pkg),
            pkg_json(&back),
            "parse ∘ fmt = id\n---\n{text}"
        );
        back.verify().expect("parsed package verifies");
        let again = fmt(&back).expect("fmt again");
        assert_eq!(text, again, "fmt ∘ parse = id on canonical text");
    }

    #[test]
    fn the_library_packages_round_trip() {
        assert_roundtrip(&crate::model_lib::package());
        assert_roundtrip(&crate::draft_lib::package());
    }

    #[test]
    fn a_hand_written_program_parses_verifies_and_is_canonical_after_one_fmt() {
        let src = r#"
package demo
// a comment the graph will not keep
export def costs_us(marginal: Fix, extra: Fix, drivers: List Text) -> Bool
  = 0.0 <. marginal or 0.0 <. extra or 0 < len(drivers)

export def clamp(x: Int, lo: Int, hi: Int) -> Int
  requires lo <= hi
  ensures lo <= result and result <= hi
  = if x < lo then lo else if hi < x then hi else x

def helper(n: Int) -> {double: Int, label: Text}
  = let d = n * 2 in {double: d, label: "n=" ++ text(n)}

export def beat(state: {hits: Int}, event: {}) -> {actions: List Action, state: {hits: Int}}
  effects notify, despawn
  = let h = helper(state.hits) in
    if 3 <= h.double
    then {actions: [notify{text: h.label ++ " falls"}, despawn{}], state: {hits: 0}}
    else {actions: [], state: {hits: h.double}}

export def sum(xs: List Int) -> Int
  = fold[8](xs, 0, acc x -> acc + x) + len(map[8](iota[8](3), i -> i * i))
"#;
        let pkg = parse(src).expect("parses");
        pkg.verify().expect("verifies");
        assert_eq!(pkg.exports.len(), 4);
        assert_eq!(pkg.defs.len(), 5, "helper is a def without a petname");
        let canonical = fmt(&pkg).unwrap();
        assert!(
            canonical.contains("def _"),
            "helper renders by hash: {canonical}"
        );
        assert!(
            canonical.contains("ensures b <= result and result <= c"),
            "{canonical}"
        );
        assert_roundtrip(&pkg);
        // Runs: clamp(10, 0, 5) = 5; sum([1,2,3]) = 6 + 3.
        let m = crate::Module {
            defs: pkg.defs.clone(),
            entry: pkg.export("clamp").unwrap(),
        };
        let v = crate::eval_call(
            &m,
            m.entry,
            vec![
                crate::Value::Int(10),
                crate::Value::Int(0),
                crate::Value::Int(5),
            ],
            1000,
        )
        .unwrap();
        assert_eq!(v.value, crate::Value::Int(5));
        let v = crate::eval_call(
            &m,
            pkg.export("sum").unwrap(),
            vec![crate::Value::List(vec![
                crate::Value::Int(1),
                crate::Value::Int(2),
                crate::Value::Int(3),
            ])],
            1000,
        )
        .unwrap();
        assert_eq!(v.value, crate::Value::Int(9));
    }

    #[test]
    fn precedence_and_parentheses_agree_between_fmt_and_parse() {
        let src = r#"
package prec
export def f(a: Int, b: Int, c: Bool) -> Bool
  = (a + b) * 2 < a - -3 and not (c or a == b) or not not c
export def g(a: Fix, s: Text) -> Text
  = if 0.5 <=. a *. 2.0 then s ++ "x" ++ fix_text(a) else "no"
export def h(r: {p: {q: Int}}) -> Int
  = r.p.q - (if 1 < r.p.q then 1 else 0)
"#;
        let pkg = parse(src).expect("parses");
        assert_roundtrip(&pkg);
        let text = fmt(&pkg).unwrap();
        assert!(
            text.contains("(a + b) * 2 < a - -3 and not (c or a == b) or not (not c)"),
            "{text}"
        );
        assert!(
            text.contains("a.p.q - (if 1 < a.p.q then 1 else 0)"),
            "{text}"
        );
    }

    #[test]
    fn errors_name_the_line_and_the_reason() {
        let cases: &[(&str, &str)] = &[
            (
                "package x\nexport def f(a: Int) -> Int\n  = b",
                "unbound name 'b'",
            ),
            (
                "package x\nexport def f(a: Int) -> Int\n  = f(a)",
                "recursion is not expressible",
            ),
            (
                "package x\nexport def f(a: Int) -> Int\n  = g(a)",
                "unknown definition 'g'",
            ),
            (
                "package x\nexport def f(a: Int) -> Bool\n  = a < 1 < 2",
                "comparisons do not chain",
            ),
            (
                "package x\nexport def f(a: Int) -> Int\n  = 1.1234567",
                "more than 6 decimals",
            ),
            (
                "package x\nexport def text(a: Int) -> Int\n  = a",
                "reserved",
            ),
            (
                "package x\nexport def f(a: Int) -> Int\n  = a\nexport def f(a: Int) -> Int\n  = a",
                "declared twice",
            ),
            (
                "package x\nexport def f(a: Int) -> Int\n  = f",
                "call it as f(...)",
            ),
        ];
        for (src, want) in cases {
            let e = parse(src).expect_err(src);
            assert!(e.message.contains(want), "{src:?} -> {e}");
        }
    }

    #[test]
    fn text_literals_escape_and_unescape() {
        let src = "package t\nexport def s() -> Text\n  = \"quote \\\" slash \\\\ nl \\n tab \\t uni \\u{1f}\"";
        let pkg = parse(src).unwrap();
        let m = crate::Module {
            defs: pkg.defs.clone(),
            entry: pkg.export("s").unwrap(),
        };
        let v = crate::eval_call(&m, m.entry, vec![], 100).unwrap();
        assert_eq!(
            v.value,
            crate::Value::Text("quote \" slash \\ nl \n tab \t uni \u{1f}".into())
        );
        assert_roundtrip(&pkg);
    }

    #[test]
    fn aliases_keep_every_petname() {
        let src =
            "package t\nexport def double(a: Int) -> Int\n  = a * 2\nexport alias twice = double";
        let pkg = parse(src).unwrap();
        assert_eq!(pkg.export("twice"), pkg.export("double"));
        assert_roundtrip(&pkg);
    }

    #[test]
    fn the_verifier_not_the_parser_catches_type_errors() {
        // Parses fine — `+` on Fix is a *type* error, and that is the verifier's job.
        let pkg = parse("package t\nexport def f(a: Fix) -> Fix\n  = a + 1").unwrap();
        assert!(matches!(pkg.verify(), Err(crate::WeftError::Type(_))));
        // An undeclared effect is refused the same way.
        let pkg = parse("package t\nexport def f() -> Action\n  = notify{text: \"hi\"}").unwrap();
        assert!(matches!(
            pkg.verify(),
            Err(crate::WeftError::EffectNotDeclared(_))
        ));
    }
}
