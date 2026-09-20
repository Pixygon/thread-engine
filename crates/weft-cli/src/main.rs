//! `weft` — the agent-facing command line for Weft packages.
//!
//! ```text
//! weft verify <pack.json>                          verifier certificate as JSON
//! weft run    <pack.json> --fn <name|hash> --input <json> [--fuel N]
//!                                                  deterministic evaluation; result as JSON
//! weft fmt    <pack.json>                          the .weft textual projection
//! weft parse  <file.weft> [-o out.json] [--unverified]
//!                                                  text → canonical pack JSON (verified first)
//! ```
//!
//! Every command prints exactly one JSON document on stdout. Exit codes:
//! `0` success · `1` any error (usage, IO, syntax, verification) · `2` a
//! contract violation at run time (the violation is the JSON).

use std::process::exit;
use std::time::Instant;

use serde_json::{json, Value as J};
use weft::json::{args_from_json, value_to_json};
use weft::pack::Package;
use weft::text;
use weft::{verify_module, Module, WeftError, WeftHash};

fn fail(code: i32, stage: &str, message: impl std::fmt::Display) -> ! {
    println!(
        "{}",
        json!({ "ok": false, "stage": stage, "error": message.to_string() })
    );
    exit(code);
}

fn read(path: &str) -> String {
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| fail(1, "io", format!("cannot read {path}: {e}")))
}

fn load(path: &str) -> Package {
    let text = read(path);
    serde_json::from_str(&text)
        .unwrap_or_else(|e| fail(1, "load", format!("{path} is not a weft package: {e}")))
}

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

/// The verifier's certificate for a whole package, as JSON.
fn certificate(pkg: &Package) -> Result<J, WeftError> {
    let names = text::def_names(pkg);
    let entry = *pkg
        .exports
        .values()
        .next()
        .or_else(|| pkg.defs.keys().next())
        .ok_or(WeftError::UnknownEntry)?;
    let m = Module {
        defs: pkg.defs.clone(),
        entry,
    };
    let certs = verify_module(&m)?;
    let mut defs = serde_json::Map::new();
    for (h, d) in &pkg.defs {
        let c = &certs[h];
        defs.insert(
            h.to_string(),
            json!({
                "name": names[h],
                "params": d.params.iter().map(text::ty_text).collect::<Vec<_>>(),
                "ret": text::ty_text(&d.ret),
                "effects": c.effects.iter().map(|e| serde_json::to_value(e).unwrap()).collect::<Vec<_>>(),
                "fuel_bound": c.fuel_bound,
                "contracts": { "pre": d.pre.is_some(), "post": d.post.is_some() },
            }),
        );
    }
    Ok(json!({
        "ok": true,
        "name": pkg.name,
        "defs": pkg.defs.len(),
        "exports": pkg.exports,
        "certificates": defs,
    }))
}

fn verify(pkg: &Package) -> J {
    // Hash integrity + dangling exports first (the package-level checks),
    // then the module verifier for the per-def certificate.
    if let Err(e) = pkg.verify() {
        fail(1, "verify", e);
    }
    certificate(pkg).unwrap_or_else(|e| fail(1, "verify", e))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("verify") => {
            let path = args.get(1).unwrap_or_else(|| usage());
            let pkg = load(path);
            println!("{}", verify(&pkg));
        }
        Some("run") => {
            let path = args.get(1).unwrap_or_else(|| usage());
            let pkg = load(path);
            let fname = flag(&args, "--fn").unwrap_or_else(|| usage());
            let input_text = match (flag(&args, "--input"), flag(&args, "--input-file")) {
                (Some(s), _) => s.to_string(),
                (None, Some(f)) => read(f),
                (None, None) => "[]".to_string(),
            };
            let input: J = serde_json::from_str(&input_text)
                .unwrap_or_else(|e| fail(1, "input", format!("--input is not JSON: {e}")));
            let cert = verify(&pkg);
            let target: WeftHash = match pkg.export(fname) {
                Some(h) => h,
                None => serde_json::from_value(J::String(fname.to_string())).unwrap_or_else(|_| {
                    fail(
                        1,
                        "run",
                        format!(
                            "no export '{fname}' in '{}' (has: {})",
                            pkg.name,
                            pkg.exports.keys().cloned().collect::<Vec<_>>().join(", ")
                        ),
                    )
                }),
            };
            let def = pkg
                .defs
                .get(&target)
                .unwrap_or_else(|| fail(1, "run", format!("{target} is not in the package")));
            let vals = args_from_json(def, &input).unwrap_or_else(|e| fail(1, "input", e));
            let bound = cert["certificates"][target.to_string()]["fuel_bound"]
                .as_u64()
                .unwrap_or(0);
            let max_fuel: u64 = match flag(&args, "--fuel") {
                Some(f) => f
                    .parse()
                    .unwrap_or_else(|_| fail(1, "run", "--fuel must be an integer")),
                // The static bound is a promise: verified code cannot exceed it.
                None => bound.saturating_add(16),
            };
            let m = Module {
                defs: pkg.defs.clone(),
                entry: target,
            };
            let started = Instant::now();
            let out = weft::eval_call(&m, target, vals, max_fuel);
            let elapsed_us = started.elapsed().as_micros() as u64;
            match out {
                Ok(ev) => println!(
                    "{}",
                    json!({
                        "ok": true,
                        "fn": fname,
                        "hash": target.to_string(),
                        "value": value_to_json(&ev.value),
                        "fuel": ev.fuel_spent,
                        "fuel_bound": bound,
                        "elapsed_us": elapsed_us,
                    })
                ),
                Err(WeftError::ContractViolated(which)) => {
                    println!(
                        "{}",
                        json!({
                            "ok": false,
                            "stage": "contract",
                            "error": format!("contract violated: {which}"),
                            "violation": { "which": which, "fn": fname, "hash": target.to_string(), "input": input },
                            "elapsed_us": elapsed_us,
                        })
                    );
                    exit(2);
                }
                Err(e) => fail(1, "run", e),
            }
        }
        Some("fmt") => {
            let path = args.get(1).unwrap_or_else(|| usage());
            let pkg = load(path);
            match text::fmt(&pkg) {
                Ok(t) => print!("{t}"),
                Err(e) => fail(1, "fmt", e),
            }
        }
        Some("parse") => {
            let path = args.get(1).unwrap_or_else(|| usage());
            let src = read(path);
            let pkg = text::parse(&src).unwrap_or_else(|e| fail(1, "parse", e));
            let verified = if args.iter().any(|a| a == "--unverified") {
                None
            } else {
                Some(verify(&pkg))
            };
            let out = serde_json::to_string_pretty(&pkg).expect("serializable");
            match flag(&args, "-o") {
                Some(file) => {
                    std::fs::write(file, format!("{out}\n"))
                        .unwrap_or_else(|e| fail(1, "io", format!("cannot write {file}: {e}")));
                    println!(
                        "{}",
                        json!({
                            "ok": true,
                            "wrote": file,
                            "name": pkg.name,
                            "defs": pkg.defs.len(),
                            "exports": pkg.exports,
                            "verified": verified.map(|v| v["certificates"].clone()),
                        })
                    );
                }
                None => println!("{out}"),
            }
        }
        _ => usage(),
    }
}

fn usage() -> ! {
    eprintln!(
        "weft — verify, run, fmt and parse Weft packages\n\n  weft verify <pack.json>\n  weft run    <pack.json> --fn <export|weft:hash> --input '<json args>' [--input-file f] [--fuel N]\n  weft fmt    <pack.json>\n  weft parse  <file.weft> [-o out.weftpack.json] [--unverified]\n\nOne JSON document on stdout. Exit 0 ok, 1 error, 2 contract violation."
    );
    exit(1);
}
