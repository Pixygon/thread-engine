//! The CLI's promises, exercised end to end through the binary:
//! `parse ∘ fmt = id` on every package we can find, `fmt ∘ parse = id` on the
//! canonical text, and the JSON contract of `verify`/`run` (exit 2 with the
//! violation as JSON when a contract fails).

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value as J;

fn weft() -> Command {
    Command::new(env!("CARGO_BIN_EXE_weft"))
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn out_json(cmd: &mut Command) -> (i32, J, String) {
    let out = cmd.output().expect("weft runs");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let code = out.status.code().unwrap_or(-1);
    let json = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout is not JSON ({e}): {stdout}"));
    (code, json, stdout)
}

/// Every package the round-trip is tested on: the in-repo clock plus the wpm
/// seeds and the API's pilot when the sibling checkouts exist (skipped with
/// a note otherwise — never a silent pass).
fn packs() -> Vec<PathBuf> {
    let root = repo_root();
    let mut found = vec![root.join("worlds/wiki.pixygon.io/weft-clock.weftpack.json")];
    for rel in [
        "../wpm/seed/weft-clock.weftpack.json",
        "../wpm/seed/weft-form.weftpack.json",
        "../wpm/seed/weft-motion.weftpack.json",
        "../PixygonAPI/weft/price-model.weftpack.json",
    ] {
        let p = root.join(rel);
        if p.exists() {
            found.push(p);
        } else {
            eprintln!("note: {rel} not checked out — skipped");
        }
    }
    found
}

#[test]
fn fmt_then_parse_is_identity_on_packs_and_parse_then_fmt_on_text() {
    let dir = std::env::temp_dir().join(format!("weft-cli-roundtrip-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for pack in packs() {
        let original: J = serde_json::from_str(&std::fs::read_to_string(&pack).unwrap()).unwrap();
        let text = weft().arg("fmt").arg(&pack).output().unwrap();
        assert!(
            text.status.success(),
            "fmt {pack:?}: {}",
            String::from_utf8_lossy(&text.stdout)
        );
        let text = String::from_utf8(text.stdout).unwrap();
        let src = dir.join(format!(
            "{}.weft",
            pack.file_stem().unwrap().to_string_lossy()
        ));
        std::fs::write(&src, &text).unwrap();
        let back = dir.join(format!(
            "{}.json",
            pack.file_stem().unwrap().to_string_lossy()
        ));
        let (code, report, _) = out_json(weft().arg("parse").arg(&src).arg("-o").arg(&back));
        assert_eq!(code, 0, "parse {src:?}: {report}");
        let reparsed: J = serde_json::from_str(&std::fs::read_to_string(&back).unwrap()).unwrap();
        assert_eq!(original, reparsed, "parse ∘ fmt = id for {pack:?}");
        let again = weft().arg("fmt").arg(&back).output().unwrap();
        assert_eq!(
            text,
            String::from_utf8(again.stdout).unwrap(),
            "fmt ∘ parse = id for {pack:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn verify_reports_effects_fuel_and_contracts_as_json() {
    let pack = repo_root().join("worlds/wiki.pixygon.io/weft-clock.weftpack.json");
    let (code, v, _) = out_json(weft().arg("verify").arg(&pack));
    assert_eq!(code, 0);
    assert_eq!(v["ok"], true);
    let clock = v["exports"]["clock"].as_str().unwrap();
    let cert = &v["certificates"][clock];
    assert_eq!(cert["name"], "clock");
    assert_eq!(cert["effects"], serde_json::json!(["set_state", "spawn"]));
    assert!(cert["fuel_bound"].as_u64().unwrap() > 0);
    assert_eq!(cert["contracts"]["pre"], false);
}

#[test]
fn run_evaluates_enforces_contracts_and_refuses_bad_input() {
    let dir = std::env::temp_dir().join(format!("weft-cli-run-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("clamp.weft");
    std::fs::write(
        &src,
        "package t\nexport def clamp(x: Int, lo: Int, hi: Int) -> Int\n  requires lo <= hi\n  ensures lo <= result and result <= hi\n  = if x < lo then lo else if hi < x then hi else x\nexport def lie(x: Int) -> Int\n  ensures result == x + 1\n  = x\n",
    )
    .unwrap();
    let pack = dir.join("clamp.json");
    let (code, _, _) = out_json(weft().arg("parse").arg(&src).arg("-o").arg(&pack));
    assert_eq!(code, 0);

    let (code, v, _) =
        out_json(
            weft()
                .arg("run")
                .arg(&pack)
                .args(["--fn", "clamp", "--input", "[10, 0, 5]"]),
        );
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["value"], 5);
    assert!(
        v["fuel"].as_u64().unwrap() <= v["fuel_bound"].as_u64().unwrap(),
        "static bound holds"
    );
    assert!(v["elapsed_us"].is_number());

    // Precondition violated → exit 2, the violation as JSON.
    let (code, v, _) =
        out_json(
            weft()
                .arg("run")
                .arg(&pack)
                .args(["--fn", "clamp", "--input", "[1, 5, 0]"]),
        );
    assert_eq!(code, 2, "{v}");
    assert_eq!(v["violation"]["which"], "pre");
    // Postcondition violated the same way.
    let (code, v, _) = out_json(
        weft()
            .arg("run")
            .arg(&pack)
            .args(["--fn", "lie", "--input", "[1]"]),
    );
    assert_eq!(code, 2, "{v}");
    assert_eq!(v["violation"]["which"], "post");
    // Input that does not fit the parameter types → exit 1, stage "input".
    let (code, v, _) =
        out_json(
            weft()
                .arg("run")
                .arg(&pack)
                .args(["--fn", "clamp", "--input", "[\"x\", 0, 5]"]),
        );
    assert_eq!(code, 1);
    assert_eq!(v["stage"], "input");
    // Unknown export → exit 1.
    let (code, v, _) = out_json(
        weft()
            .arg("run")
            .arg(&pack)
            .args(["--fn", "nope", "--input", "[]"]),
    );
    assert_eq!(code, 1);
    assert_eq!(v["stage"], "run");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn parse_refuses_programs_the_verifier_rejects_unless_asked_not_to() {
    let dir = std::env::temp_dir().join(format!("weft-cli-parse-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("bad.weft");
    std::fs::write(&src, "package t\nexport def f(a: Fix) -> Fix\n  = a + 1\n").unwrap();
    let (code, v, _) = out_json(weft().arg("parse").arg(&src));
    assert_eq!(code, 1);
    assert_eq!(v["stage"], "verify");
    assert!(v["error"].as_str().unwrap().contains("type error"), "{v}");
    let (code, v, _) = out_json(weft().arg("parse").arg(&src).arg("--unverified"));
    assert_eq!(code, 0);
    assert_eq!(v["name"], "t");
    // A syntax error names its line.
    std::fs::write(&src, "package t\nexport def f(a: Int) -> Int\n  = a +\n").unwrap();
    let (code, v, _) = out_json(weft().arg("parse").arg(&src));
    assert_eq!(code, 1);
    assert_eq!(v["stage"], "parse");
    assert!(v["error"].as_str().unwrap().starts_with("line 3"), "{v}");
    let _ = std::fs::remove_dir_all(&dir);
}
