//! `weftpack` — npm for a language where packages cannot lie.
//!
//! Every definition is named by the hash of its canonical bytes, so the
//! whole package story collapses into four verbs:
//!
//! ```text
//! weftpack verify <pkg.weftpack.json>          is every byte what it claims?
//! weftpack show   <pkg> <petname>              read a def as human text
//! weftpack link   <pkg...> --entry <petname> -o module.json
//!                                              a runnable module from an export
//! weftpack fetch  <url> [-o out.weftpack.json] pull + verify from any host
//! ```
//!
//! There is no login, no publish endpoint, and no registry server to trust:
//! hosting a package is serving a file (a dir, a CDN, a Thread host's
//! `.well-known/weft/`), and `verify` decides locally whether the bytes are
//! honest. Names are petnames; identity is the hash.

use std::process::exit;

use weft::pack::Package;

fn load(path: &str) -> Package {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("cannot read {path}: {e}");
        exit(2);
    });
    serde_json::from_str(&text).unwrap_or_else(|e| {
        eprintln!("{path} is not a weft package: {e}");
        exit(2);
    })
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("verify") => {
            let path = args.get(1).unwrap_or_else(|| usage());
            let pkg = load(path);
            match pkg.verify() {
                Ok(()) => {
                    println!(
                        "✓ '{}' verifies — {} defs, {} exports",
                        pkg.name,
                        pkg.defs.len(),
                        pkg.exports.len()
                    );
                    for (name, h) in &pkg.exports {
                        println!("  {name}  →  {h}");
                    }
                }
                Err(e) => {
                    eprintln!("✗ '{}' REFUSED: {e}", pkg.name);
                    exit(1);
                }
            }
        }
        Some("show") => {
            let (path, petname) = (
                args.get(1).unwrap_or_else(|| usage()),
                args.get(2).unwrap_or_else(|| usage()),
            );
            let pkg = load(path);
            let Some(h) = pkg.export(petname) else {
                eprintln!(
                    "no export '{petname}' in '{}' (has: {})",
                    pkg.name,
                    pkg.exports.keys().cloned().collect::<Vec<_>>().join(", ")
                );
                exit(1);
            };
            let def = &pkg.defs[&h];
            println!("// {petname} = {h}");
            println!("{}", weft::project::def(def));
        }
        Some("link") => {
            // weftpack link a.json b.json --entry ring -o module.json
            let mut pkgs = Vec::new();
            let mut entry: Option<String> = None;
            let mut out = "module.weft.json".to_string();
            let mut i = 1;
            while i < args.len() {
                match args[i].as_str() {
                    "--entry" => {
                        entry = args.get(i + 1).cloned();
                        i += 2;
                    }
                    "-o" => {
                        out = args.get(i + 1).cloned().unwrap_or(out);
                        i += 2;
                    }
                    p => {
                        pkgs.push(load(p));
                        i += 1;
                    }
                }
            }
            let entry = entry.unwrap_or_else(|| usage());
            let hash = pkgs
                .iter()
                .find_map(|p| p.export(&entry))
                .unwrap_or_else(|| {
                    eprintln!("no package exports '{entry}'");
                    exit(1);
                });
            // The module's entry is the export itself; link trims to closure.
            let entry_def = pkgs
                .iter()
                .find_map(|p| p.defs.get(&hash).cloned())
                .expect("export verified to exist");
            match weft::pack::link(&pkgs, vec![entry_def], 0) {
                Ok(module) => {
                    std::fs::write(&out, serde_json::to_string(&module).unwrap()).unwrap();
                    println!(
                        "✓ linked '{entry}' + closure ({} defs) → {out}",
                        module.defs.len()
                    );
                }
                Err(e) => {
                    eprintln!("✗ link failed: {e}");
                    exit(1);
                }
            }
        }
        Some("fetch") => {
            let url = args.get(1).unwrap_or_else(|| usage());
            let out = match args.get(2).map(String::as_str) {
                Some("-o") => args.get(3).cloned(),
                _ => None,
            }
            .unwrap_or_else(|| {
                url.rsplit('/')
                    .next()
                    .unwrap_or("package.weftpack.json")
                    .to_string()
            });
            let text = reqwest::blocking::get(url.as_str())
                .and_then(|r| r.error_for_status())
                .and_then(|r| r.text())
                .unwrap_or_else(|e| {
                    eprintln!("fetch failed: {e}");
                    exit(1);
                });
            let pkg: Package = serde_json::from_str(&text).unwrap_or_else(|e| {
                eprintln!("not a weft package: {e}");
                exit(1);
            });
            // Verify BEFORE writing — unverified bytes never touch disk.
            if let Err(e) = pkg.verify() {
                eprintln!("✗ fetched package REFUSED: {e}");
                exit(1);
            }
            std::fs::write(&out, &text).unwrap();
            println!("✓ '{}' fetched + verified → {out}", pkg.name);
        }
        Some("list") => {
            // weftpack list [registry]  — the shelf's index.
            let registry = args
                .get(1)
                .cloned()
                .or_else(|| std::env::var("WPM_REGISTRY").ok())
                .unwrap_or_else(|| "https://wpm.pixygon.io".into());
            let body = reqwest::blocking::get(format!("{registry}/"))
                .and_then(|r| r.error_for_status())
                .and_then(|r| r.text())
                .unwrap_or_else(|e| {
                    eprintln!("registry unreachable: {e}");
                    exit(1);
                });
            let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
            for p in v["packages"].as_array().cloned().unwrap_or_default() {
                println!(
                    "{}  ({} defs)  exports: {}",
                    p["name"].as_str().unwrap_or("?"),
                    p["defs"],
                    p["exports"]
                        .as_array()
                        .map(|a| a
                            .iter()
                            .filter_map(|e| e.as_str())
                            .collect::<Vec<_>>()
                            .join(", "))
                        .unwrap_or_default()
                );
                println!("    {registry}{}", p["url"].as_str().unwrap_or(""));
            }
        }
        Some("publish") => {
            // weftpack publish <pkg.weftpack.json> [--registry URL]
            let path = args.get(1).unwrap_or_else(|| usage());
            let registry = args
                .iter()
                .position(|a| a == "--registry")
                .and_then(|i| args.get(i + 1).cloned())
                .or_else(|| std::env::var("WPM_REGISTRY").ok())
                .unwrap_or_else(|| "https://wpm.pixygon.io".into());
            let pkg = load(path);
            // Verify BEFORE sending — never ship bytes you haven't checked.
            if let Err(e) = pkg.verify() {
                eprintln!("✗ refusing to publish '{}': {e}", pkg.name);
                exit(1);
            }
            let text = std::fs::read_to_string(path).unwrap();
            let mut req = reqwest::blocking::Client::new()
                .post(format!("{registry}/publish"))
                .body(text);
            if let Ok(tok) = std::env::var("WPM_TOKEN") {
                req = req.header("authorization", format!("Bearer {tok}"));
            }
            match req.send() {
                Ok(r) if r.status().is_success() => {
                    println!("✓ '{}' published to {registry}", pkg.name);
                }
                Ok(r) => {
                    eprintln!(
                        "✗ registry said {}: {}",
                        r.status(),
                        r.text().unwrap_or_default()
                    );
                    exit(1);
                }
                Err(e) => {
                    eprintln!("✗ publish failed: {e}");
                    exit(1);
                }
            }
        }
        _ => {
            usage();
        }
    }
}

fn usage() -> ! {
    eprintln!(
        "weftpack — packages for a content-addressed language\n\n  weftpack verify <pkg.weftpack.json>\n  weftpack show   <pkg> <petname>\n  weftpack link   <pkg...> --entry <petname> [-o module.json]\n  weftpack fetch  <url> [-o file]\n  weftpack list   [registry]                   (WPM_REGISTRY env)\n  weftpack publish <pkg> [--registry URL]      (WPM_TOKEN env)"
    );
    exit(2);
}
