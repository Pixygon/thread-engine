//! `thread grow <recipe.json> [-o tree.glb] [--preview sheet.png] [--sockets tree.sockets.json]
//!              [--hang thing.glb [--hang-count n] [--hang-scale s] [--hang-drop m] [--hang-level l]]`
//!
//! Grow a tree from a [`grove::grow::GrowRecipe`]: LOD0 to `-o`, coarser
//! LODs beside it as `<stem>.lod1.glb`, `<stem>.lod2.glb`…, the tip sockets
//! to a JSON file the layout binder / the Unity importer can hang props on,
//! and the same turntable proof every other model gets.
//!
//! With `--hang`, a second model (a Trellis fruit, a carved lantern, a leaf
//! cluster) is placed at chosen sockets by [`grove::hang`] and `-o` becomes
//! the composed scene — the wood once, the hung thing once, one node per
//! tip — with the bare wood beside it as `<stem>.wood.glb` and the
//! placements as `<stem>.placements.json`.
use std::process::ExitCode;

use chisel::gltf::{write_glb_scene, SceneMesh, SceneNode};
use chisel::model::{Built, BuiltPart};
use chisel::MeshData;
use grove::grow::{grow, GrowRecipe};
use grove::hang::{hang, HangRecipe, Placement};

pub fn cmd_grow(args: &[String]) -> ExitCode {
    let mut file: Option<&String> = None;
    let mut out: Option<&String> = None;
    let mut preview: Option<&String> = None;
    let mut sockets_out: Option<&String> = None;
    let mut views: u32 = 3;
    let mut seed: Option<u32> = None;
    let mut hang_path: Option<&String> = None;
    let mut hang_recipe = HangRecipe { count: 9, min_level: 1, scale: 1.0, ..Default::default() };
    // Glow for the hung thing (a lantern fruit is a light source); None keeps what its glb says.
    let mut hang_glow: Option<f32> = None;
    let mut publish = false;
    let mut title: Option<&String> = None;
    let mut kind: Option<&String> = None;
    let mut style: Option<&String> = None;
    let mut tags: Option<&String> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-o" | "--out" => out = it.next(),
            "--preview" | "-p" => preview = it.next(),
            "--sockets" => sockets_out = it.next(),
            "--seed" => seed = it.next().and_then(|v| v.parse().ok()),
            "--views" => views = it.next().and_then(|v| v.parse().ok()).unwrap_or(3),
            "--hang" => hang_path = it.next(),
            "--hang-count" => hang_recipe.count = it.next().and_then(|v| v.parse().ok()).unwrap_or(9),
            "--hang-scale" => hang_recipe.scale = it.next().and_then(|v| v.parse().ok()).unwrap_or(1.0),
            "--hang-drop" => hang_recipe.drop = it.next().and_then(|v| v.parse().ok()).unwrap_or(0.15),
            "--hang-level" => hang_recipe.min_level = it.next().and_then(|v| v.parse().ok()).unwrap_or(1),
            "--hang-glow" => hang_glow = it.next().and_then(|v| v.parse().ok()),
            "--publish" => publish = true,
            "--title" => title = it.next(),
            "--kind" => kind = it.next(),
            "--style" => style = it.next(),
            "--tags" => tags = it.next(),
            s if s.starts_with("--") => {}
            _ => file = Some(a),
        }
    }
    let Some(file) = file else {
        eprintln!("usage: thread grow <recipe.json> [-o tree.glb] [--preview sheet.png] [--sockets tree.sockets.json] [--seed n] [--views n] [--hang thing.glb --hang-count n --hang-scale s --hang-drop m --hang-level l]");
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
    hang_recipe.seed = recipe.seed;
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

    // The bare wood: `-o` when nothing hangs, `<stem>.wood.glb` otherwise.
    let wood_path = if hang_path.is_some() { format!("{base}.wood.glb") } else { out_path.clone() };
    match chisel::model::export_glb(&grown.built) {
        Ok(glb) => {
            if let Err(e) = std::fs::write(&wood_path, &glb) {
                eprintln!("✗ cannot write {wood_path}: {e}");
                return ExitCode::from(1);
            }
            let (min, max) = grown.bounds;
            println!(
                "✓ {} → {wood_path} — {} tris, {:.2} × {:.2} × {:.2} m, {} socket(s), seed {}",
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
    for (k, lod_built) in grown.lods.iter().enumerate() {
        let i = k + 1;
        match chisel::model::export_glb(lod_built) {
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

    // The composition: wood + the hung thing, instanced per chosen tip.
    let mut shown = Built {
        name: grown.built.name.clone(),
        parts: grown
            .built
            .parts
            .iter()
            .map(|p| BuiltPart { name: p.name.clone(), mesh: p.mesh.clone(), baked: p.baked.clone(), color: p.color, emissive: p.emissive, double_sided: p.double_sided })
            .collect(),
    };
    if let Some(hp) = hang_path {
        let mut thing = match crate::view::load(hp) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("✗ cannot load {hp}: {e}");
                return ExitCode::from(1);
            }
        };
        if let Some(g) = hang_glow {
            for p in thing.parts.iter_mut() {
                p.emissive = g;
            }
        }
        let (_, tmax) = thing.bounds();
        let placements = hang(&grown.sockets, tmax[1], &hang_recipe);
        let wood = &grown.built.parts[0];
        let mut meshes: Vec<SceneMesh> = vec![SceneMesh { name: wood.name.clone(), mesh: &wood.mesh, baked: wood.baked.as_ref(), base_color: wood.color, emissive: wood.emissive, double_sided: wood.double_sided }];
        for p in &thing.parts {
            meshes.push(SceneMesh { name: format!("{}-{}", thing.name, p.name), mesh: &p.mesh, baked: p.baked.as_ref(), base_color: p.color, emissive: p.emissive, double_sided: p.double_sided });
        }
        let mut nodes: Vec<SceneNode> = vec![SceneNode { name: "wood".into(), mesh: 0, translation: [0.0; 3], rotation: [0.0, 0.0, 0.0, 1.0], scale: [1.0; 3] }];
        for pl in &placements {
            for (pi, _) in thing.parts.iter().enumerate() {
                nodes.push(SceneNode { name: pl.socket.clone(), mesh: 1 + pi, translation: pl.translation, rotation: pl.rotation, scale: pl.scale });
            }
        }
        match write_glb_scene(&meshes, &nodes) {
            Ok(glb) => {
                if let Err(e) = std::fs::write(&out_path, &glb) {
                    eprintln!("✗ cannot write {out_path}: {e}");
                    return ExitCode::from(1);
                }
                println!("✓ {} + {} × {} → {out_path}", recipe.name, placements.len(), thing.name);
            }
            Err(e) => {
                eprintln!("✗ scene export failed: {e}");
                return ExitCode::from(1);
            }
        }
        let pp = format!("{base}.placements.json");
        if let Ok(json) = serde_json::to_string_pretty(&placements) {
            if std::fs::write(&pp, json).is_ok() {
                println!("  placements → {pp}");
            }
        }
        // The turntable sees what the scene holds: bake the instances in.
        for pl in &placements {
            for p in &thing.parts {
                shown.parts.push(BuiltPart { name: format!("{}@{}", p.name, pl.socket), mesh: transformed(&p.mesh, pl), baked: p.baked.clone(), color: p.color, emissive: p.emissive, double_sided: p.double_sided });
            }
        }
    }
    if let Some(shot) = preview {
        let opts = chisel::preview::PreviewOptions { views, ..Default::default() };
        match chisel::preview::write_png(&shown, opts, shot) {
            Ok(()) => println!("✓ preview → {shot} ({views}-view turntable)"),
            Err(e) => eprintln!("⚠ preview failed: {e}"),
        }
    }
    // Same posture as `thread model`: --publish sends the RECIPE and the
    // Quarry grows the tree itself, measuring its sockets. Nothing uploaded.
    if publish {
        let quarry = std::env::var("QUARRY_URL").unwrap_or_else(|_| "https://quarry.pixygon.io".to_string());
        let submission = serde_json::json!({
            "title": title.cloned().unwrap_or_else(|| recipe.name.replace('-', " ")),
            "description": String::new(),
            "kind": kind.cloned().unwrap_or_else(|| "tree".into()),
            "style": style.cloned().unwrap_or_default(),
            "tags": tags
                .map(|t| t.split(',').map(|s| s.trim().to_string()).collect::<Vec<_>>())
                .unwrap_or_else(|| vec!["tree".into(), "grown".into()]),
            "package": "grove",
            "recipe": serde_json::to_value(&recipe).unwrap_or_default(),
            "origin": "grown",
        });
        match crate::post_json(&format!("{}/publish", quarry.trim_end_matches('/')), &submission) {
            Ok(body) => {
                let design = serde_json::from_str::<serde_json::Value>(&body)
                    .ok()
                    .and_then(|v| v["design"].as_str().map(str::to_string))
                    .unwrap_or_default();
                if design.is_empty() {
                    // A reply without a design is a refusal, whatever its status line.
                    eprintln!("✗ the Quarry refused it: {}", body.trim());
                    return ExitCode::FAILURE;
                }
                println!("✓ published to the Quarry — {quarry}/models/{design}.glb");
            }
            Err(e) => {
                eprintln!("✗ publish failed: {e}");
                return ExitCode::FAILURE;
            }
        }
    }
    ExitCode::SUCCESS
}

/// Apply a placement (scale, then rotate, then translate) to a mesh copy.
fn transformed(m: &MeshData, pl: &Placement) -> MeshData {
    let mut out = m.clone();
    for p in out.positions.iter_mut() {
        let s = [p[0] * pl.scale[0], p[1] * pl.scale[1], p[2] * pl.scale[2]];
        let r = rotate(s, pl.rotation);
        *p = [r[0] + pl.translation[0], r[1] + pl.translation[1], r[2] + pl.translation[2]];
    }
    for n in out.normals.iter_mut() {
        *n = rotate(*n, pl.rotation);
    }
    for t in out.tangents.iter_mut() {
        let r = rotate([t[0], t[1], t[2]], pl.rotation);
        *t = [r[0], r[1], r[2], t[3]];
    }
    out
}

/// Rotate `v` by quaternion `q = [x y z w]`.
fn rotate(v: [f32; 3], q: [f32; 4]) -> [f32; 3] {
    let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
    // t = 2 * cross(q.xyz, v); v' = v + w*t + cross(q.xyz, t)
    let t = [2.0 * (y * v[2] - z * v[1]), 2.0 * (z * v[0] - x * v[2]), 2.0 * (x * v[1] - y * v[0])];
    [
        v[0] + w * t[0] + (y * t[2] - z * t[1]),
        v[1] + w * t[1] + (z * t[0] - x * t[2]),
        v[2] + w * t[2] + (x * t[1] - y * t[0]),
    ]
}
