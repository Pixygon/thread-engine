//! # weft-pack — packages for a content-addressed language
//!
//! npm distributes *names that move*; weft-pack distributes **hashes that
//! can't**. A definition's name IS the hash of its canonical bytes, so:
//!
//! - a package can never be *changed under you* (no left-pad: unpublishing a
//!   hash breaks nothing for anyone who has the bytes, and the bytes verify),
//! - two packages can never *conflict* (defs are keyed by hash; the same def
//!   pulled in twice is literally one entry),
//! - names are **petnames**: a package's `exports` maps human names to
//!   hashes purely for authoring convenience — the linked module never
//!   contains a name, only hashes,
//! - and there is no trust problem a registry must solve: **verification is
//!   local**. Whoever serves you the bytes, the verifier decides.
//!
//! A registry is therefore just a static file host — a directory, a CDN, or
//! a Thread host's `.well-known/weft/` — serving `<name>.weftpack.json`.
//! The `weftpack` CLI (crates/weft-pack) hashes, verifies, links, and fetches.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{hash_def, verify_module, Def, Module, WeftError, WeftHash};

/// A distributable set of definitions plus the petnames it exports.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Package {
    /// Human-facing package name (informational — identity lives in hashes).
    pub name: String,
    /// Petname → definition hash. What `use`-ing this package gives you.
    pub exports: BTreeMap<String, WeftHash>,
    /// Every definition the exports (transitively) need, keyed by hash.
    pub defs: BTreeMap<WeftHash, Def>,
}

impl Package {
    /// Build a package from defs + `(petname, def_index)` export pairs.
    pub fn build(
        name: &str,
        defs: Vec<Def>,
        exports: Vec<(&str, usize)>,
    ) -> Result<Self, WeftError> {
        let hashed: Vec<(WeftHash, Def)> = defs.into_iter().map(|d| (hash_def(&d), d)).collect();
        let mut map = BTreeMap::new();
        let mut ex = BTreeMap::new();
        for (petname, idx) in exports {
            let (h, _) = hashed.get(idx).ok_or(WeftError::UnknownEntry)?;
            ex.insert(petname.to_string(), *h);
        }
        for (h, d) in hashed {
            map.insert(h, d);
        }
        let pkg = Package {
            name: name.to_string(),
            exports: ex,
            defs: map,
        };
        pkg.verify()?;
        Ok(pkg)
    }

    /// A package must be **self-verifying**: every def's hash matches its
    /// bytes, every export points at a def, and the whole set verifies as a
    /// module (types, effects, fuel — the full trust boundary, locally).
    pub fn verify(&self) -> Result<(), WeftError> {
        for (h, d) in &self.defs {
            if hash_def(d) != *h {
                return Err(WeftError::Type(format!("def {h} does not hash to its key")));
            }
        }
        for (name, h) in &self.exports {
            if !self.defs.contains_key(h) {
                return Err(WeftError::Type(format!(
                    "export '{name}' points outside the package"
                )));
            }
        }
        // Verify the def set as a module (entry choice is arbitrary).
        if let Some(first) = self
            .exports
            .values()
            .next()
            .or_else(|| self.defs.keys().next())
        {
            let m = Module {
                defs: self.defs.clone(),
                entry: *first,
            };
            verify_module(&m)?;
        }
        Ok(())
    }

    /// The export's hash, by petname.
    pub fn export(&self, petname: &str) -> Option<WeftHash> {
        self.exports.get(petname).copied()
    }
}

/// Link packages + local defs into one verified [`Module`]. Because defs are
/// content-addressed, linking is a **set union** — no resolution order, no
/// conflicts, no diamond problem: the same hash from two packages is one def.
pub fn link(
    packages: &[Package],
    local_defs: Vec<Def>,
    entry_index: usize,
) -> Result<Module, WeftError> {
    let mut defs: BTreeMap<WeftHash, Def> = BTreeMap::new();
    for p in packages {
        p.verify()?;
        for (h, d) in &p.defs {
            defs.insert(*h, d.clone());
        }
    }
    let mut entry = None;
    for (i, d) in local_defs.into_iter().enumerate() {
        let h = hash_def(&d);
        if i == entry_index {
            entry = Some(h);
        }
        defs.insert(h, d);
    }
    let entry = entry.ok_or(WeftError::UnknownEntry)?;
    let module = Module { defs, entry };
    // Trim to the entry's transitive closure — ship only what's reachable.
    let mut keep: BTreeSet<WeftHash> = BTreeSet::new();
    let mut stack = vec![entry];
    while let Some(h) = stack.pop() {
        if !keep.insert(h) {
            continue;
        }
        if let Some(d) = module.defs.get(&h) {
            for c in crate::callees_of(&d.body) {
                stack.push(c);
            }
        }
    }
    let module = Module {
        defs: module
            .defs
            .into_iter()
            .filter(|(h, _)| keep.contains(h))
            .collect(),
        entry,
    };
    verify_module(&module)?;
    Ok(module)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PrimOp, Term, Ty};

    fn double_def() -> Def {
        Def {
            params: vec![Ty::Int],
            ret: Ty::Int,
            effects: BTreeSet::new(),
            body: Term::Prim(PrimOp::Mul, vec![Term::Var(0), Term::Int(2)]),
            pre: None,
            post: None,
        }
    }

    #[test]
    fn packages_build_verify_and_link_by_hash() {
        let pkg = Package::build("math-basics", vec![double_def()], vec![("double", 0)]).unwrap();
        let double = pkg.export("double").expect("exported");
        // A consumer module calls the package def BY HASH — no names on the wire.
        let entry = Def {
            params: vec![],
            ret: Ty::Int,
            effects: BTreeSet::new(),
            body: Term::Call(double, vec![Term::Int(21)]),
            pre: None,
            post: None,
        };
        let module = link(&[pkg], vec![entry], 0).expect("links + verifies");
        let out = crate::eval_call(&module, module.entry, vec![], 10_000).unwrap();
        assert_eq!(out.value, crate::Value::Int(42));
    }

    #[test]
    fn linking_is_a_set_union_no_diamond_problem() {
        // Two packages exporting the SAME def (same hash) — linking dedups.
        let a = Package::build("a", vec![double_def()], vec![("dbl", 0)]).unwrap();
        let b = Package::build("b", vec![double_def()], vec![("twice", 0)]).unwrap();
        assert_eq!(
            a.export("dbl"),
            b.export("twice"),
            "same bytes, same identity"
        );
        let entry = Def {
            params: vec![],
            ret: Ty::Int,
            effects: BTreeSet::new(),
            body: Term::Call(a.export("dbl").unwrap(), vec![Term::Int(3)]),
            pre: None,
            post: None,
        };
        let m = link(&[a, b], vec![entry], 0).unwrap();
        assert_eq!(m.defs.len(), 2, "one shared def + the entry");
    }

    #[test]
    fn tampered_defs_are_refused() {
        let mut pkg = Package::build("m", vec![double_def()], vec![("double", 0)]).unwrap();
        // Flip the body of the stored def without re-keying it.
        let key = *pkg.defs.keys().next().unwrap();
        pkg.defs.get_mut(&key).unwrap().body = Term::Int(666);
        assert!(pkg.verify().is_err(), "hash mismatch must be fatal");
    }
}
