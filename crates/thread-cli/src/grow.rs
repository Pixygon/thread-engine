//! `thread grow <recipe.json> [-o tree.glb] [--preview sheet.png] [--sockets tree.sockets.json] [--views n]`
//!
//! Grow a tree from a [`grove::grow::GrowRecipe`]: LOD0 to `-o`, coarser
//! LODs beside it as `<stem>.lod1.glb`, `<stem>.lod2.glb`…, the tip sockets
//! to a JSON file the layout binder / the Unity importer can hang props on,
//! and the same turntable proof every other model gets.
use std::process::ExitCode;

use grove::grow::{grow, GrowRecipe};
use chisel::model::{Built, BuiltPart};

pub fn cmd_grow(args: &[String]) -> ExitCode {
    let mut file: Option<&String> = None;
    let mut out: Option<&String> = None;
    let mut preview: Option<&String> = None;
    let mut sockets_out: Option<&String> = None;
    let mut views: u32 = 3;
    let mut seed: Option<u32> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-o" | "--out" => out = it.next(),
            "--preview" | "-p" => preview = it.next(),
            "--sockets" => sockets_out = it.next(),
            "--seed" => seed = it.next().and_then(|v| v.parse().ok()),
            "--views" => views = it.next().and_then(|v| v.parse().ok()).unwrap_or(3),
            s if s.starts_with("--") => {}
            _ => file = Some(a),
        }
    }
    let Some(file) = file else {
        eprintln!("usage: thread grow <recipe.json> [-o tree.glb] [--preview sheet.png] [--sockets tree.sockets.json] [--seed n] [--views n]");
        return ExitCode::from(2);
    };
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("✗ cannot read {file}: {e}");
            return ExitCode::from(1);
        }
    };
    let mut recipe: GrowRecipe = match serde_json::from_str(&text) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("✗ {file} is not a grow recipe: {e}");
            return ExitCode::from(1);
        }
    };
    if let Some(s) = seed {
        recipe.seed = s;
    }
    let grown = match grow(&recipe) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("✗ {e}");
            return ExitCode::from(1);
        }
    };
    let stem = if recipe.name.is_empty() { "tree".to_string() } else { recipe.name.clone() };
    let out_path = out.cloned().unwrap_or_else(|| format!("{stem}.glb"));
    let base = out_path.trim_end_matches(".glb").to_string();

    // LOD0 is the Built; coarser LODs share its material.
    match chisel::model::export_glb(&grown.built) {
        Ok(glb) => {
            if let Err(e) = std::fs::write(&out_path, &glb) {
                eprintln!("✗ cannot write {out_path}: {e}");
                return ExitCode::from(1);
            }
            let (min, max) = grown.bounds;
            println!(
                "✓ {} → {out_path} — {} tris, {:.2} × {:.2} × {:.2} m, {} socket(s), seed {}",
                recipe.name,
                grown.built.triangles(),
                max[0] - min[0],
                max[1] - min[1],
                max[2] - min[2],
                grown.sockets.len(),
                recipe.seed
            );
        }
        Err(e) => {
            eprintln!("✗ export failed: {e}");
            return ExitCode::from(1);
        }
    }
    for (i, lod) in grown.lods.iter().enumerate().skip(1) {
        let part = &grown.built.parts[0];
        let lod_built = Built {
            name: format!("{stem}-lod{i}"),
            parts: vec![BuiltPart { name: part.name.clone(), mesh: lod.clone(), baked: part.baked.clone(), color: part.color, emissive: part.emissive }],
        };
        match chisel::model::export_glb(&lod_built) {
            Ok(glb) => {
                let p = format!("{base}.lod{i}.glb");
                if std::fs::write(&p, &glb).is_ok() {
                    println!("  lod{i} → {p} — {} tris", lod_built.triangles());
                }
            }
            Err(e) => eprintln!("⚠ lod{i} export failed: {e}"),
        }
    }
    let sockets_path = sockets_out.cloned().unwrap_or_else(|| format!("{base}.sockets.json"));
    match serde_json::to_string_pretty(&grown.sockets) {
        Ok(json) => {
            if std::fs::write(&sockets_path, json).is_ok() {
                println!("  sockets → {sockets_path}");
            }
        }
        Err(e) => eprintln!("⚠ sockets: {e}"),
    }
    if let Some(shot) = preview {
        let opts = chisel::preview::PreviewOptions { views, ..Default::default() };
        match chisel::preview::write_png(&grown.built, opts, shot) {
            Ok(()) => println!("✓ preview → {shot} ({views}-view turntable)"),
            Err(e) => eprintln!("⚠ preview failed: {e}"),
        }
    }
    ExitCode::SUCCESS
}
