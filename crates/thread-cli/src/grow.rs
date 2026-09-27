//! `thread grow <recipe.json> [-o tree.glb] [--preview sheet.png] [--sockets tree.sockets.json]
//!              [--seed n] [--age seasons] [--season 0..1]
//!              [--hang thing.glb [--hang-count n] [--hang-scale s] [--hang-drop m] [--hang-level l]]`
//!
//! Grow a plant from a recipe file. The file is a
//! [species](grove::grow::Species) — the rules, the plant's identity — plus
//! the individual (`seed`) and the moment (`age`, `season`) it shows by
//! default; `--seed`, `--age` and `--season` override them, so one recipe
//! renders a whole stand of individuals and a whole life without editing
//! anything.
//!
//! Out come LOD0 to `-o`, coarser LODs beside it as `<stem>.lod1.glb`,
//! `<stem>.lod2.glb`…, the sockets to a JSON file the layout binder / the
//! Unity importer can hang props on, and the same turntable proof every other
//! model gets.
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
use grove::grow::{fallen, grow_planting, Planting, SocketKind};
use grove::hang::{hang, HangRecipe, Placement};

pub fn cmd_grow(args: &[String]) -> ExitCode {
    let mut file: Option<&String> = None;
    let mut out: Option<&String> = None;
    let mut preview: Option<&String> = None;
    let mut life: Option<&String> = None;
    let mut year: Option<&String> = None;
    let mut withered = false;
    let mut cuts: Vec<u32> = Vec::new();
    let mut takes: Vec<(String, Option<f32>)> = Vec::new();
    let mut fallen_of: Option<u32> = None;
    let mut impostor = false;
    let mut sockets_out: Option<&String> = None;
    let mut views: u32 = 3;
    let mut seed: Option<u32> = None;
    let mut age: Option<f32> = None;
    let mut season: Option<f32> = None;
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
            "--life" => life = it.next(),
            "--year" => year = it.next(),
            "--withered" => withered = true,
            "--cut" => {
                if let Some(id) = it.next().and_then(|v| branch_id(v)) {
                    cuts.push(id);
                }
            }
            "--take" => {
                if let Some(v) = it.next() {
                    // `fruit-0000beef-0` or `fruit-0000beef-0@20` (back at age 20).
                    let (name, until) = match v.split_once('@') {
                        Some((n, u)) => (n.to_string(), u.parse().ok()),
                        None => (v.clone(), None),
                    };
                    takes.push((name, until));
                }
            }
            "--fallen" => fallen_of = it.next().and_then(|v| branch_id(v)),
            "--impostor" => impostor = true,
            "--sockets" => sockets_out = it.next(),
            "--seed" => seed = it.next().and_then(|v| v.parse().ok()),
            "--age" => age = it.next().and_then(|v| v.parse().ok()),
            "--season" => season = it.next().and_then(|v| v.parse().ok()),
            "--views" => views = it.next().and_then(|v| v.parse().ok()).unwrap_or(3),
            "--hang" => hang_path = it.next(),
            "--hang-count" => hang_recipe.count = it.next().and_then(|v| v.parse().ok()).unwrap_or(9),
            "--hang-scale" => hang_recipe.scale = it.next().and_then(|v| v.parse().ok()).unwrap_or(1.0),
            "--hang-drop" => hang_recipe.drop = it.next().and_then(|v| v.parse().ok()).unwrap_or(0.15),
            "--hang-level" => hang_recipe.min_level = it.next().and_then(|v| v.parse().ok()).unwrap_or(1),
            "--hang-kind" => {
                hang_recipe.kind = match it.next().map(|v| v.as_str()) {
                    Some("bloom") => SocketKind::Bloom,
                    Some("fruit") => SocketKind::Fruit,
                    Some("cut") => SocketKind::Cut,
                    _ => SocketKind::Tip,
                }
            }
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
        eprintln!("usage: thread grow <recipe.json> [-o tree.glb] [--preview sheet.png] [--sockets tree.sockets.json] [--seed n] [--age seasons] [--season 0..1] [--withered] [--cut <branch>]… [--take <socket>[@age]]… [--fallen <branch>] [--impostor] [--life life.png] [--year year.png] [--views n] [--hang thing.glb --hang-kind tip|bloom|fruit --hang-count n --hang-scale s --hang-drop m --hang-level l]");
        return ExitCode::from(2);
    };
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("✗ cannot read {file}: {e}");
            return ExitCode::from(1);
        }
    };
    let mut planting = match Planting::from_json(&text) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("✗ {file}: {e}");
            return ExitCode::from(1);
        }
    };
    if let Some(s) = seed {
        planting.seed = s;
    }
    if let Some(a) = age {
        planting.clock.age = Some(a);
    }
    if let Some(s) = season {
        planting.clock.season = s;
    }
    if withered {
        planting.state.withered = true;
    }
    for id in cuts {
        planting.state = planting.state.clone().cut(id);
    }
    for (name, until) in takes {
        planting.state = planting.state.clone().take(&name, until);
    }
    hang_recipe.seed = planting.seed;
    let grown = match grow_planting(&planting) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("✗ {e}");
            return ExitCode::from(1);
        }
    };
    let name = planting.species.name.clone();
    let stem = if name.is_empty() { "tree".to_string() } else { name.clone() };
    let out_path = out.cloned().unwrap_or_else(|| format!("{stem}.glb"));
    let base = out_path.trim_end_matches(".glb").to_string();
    // The clock, as the line should read it: a plant is an individual at a moment.
    let when = match planting.clock.age {
        Some(a) => format!("age {a}, {:?} (maturity {:.2})", grown.stage, grown.maturity).to_lowercase(),
        None => format!("{:?}", grown.stage).to_lowercase(),
    };
    let when = format!(
        "{when}, {} of the year{}{}{}",
        format!("{:?}", grown.phase).to_lowercase(),
        if planting.state.withered { ", withered" } else { "" },
        match planting.state.cut.len() {
            0 => String::new(),
            n => format!(", {n} cut"),
        },
        match planting.state.taken.len() {
            0 => String::new(),
            n => format!(", {n} taken"),
        }
    );

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
                "✓ {name} → {wood_path} — {} tris, {:.2} × {:.2} × {:.2} m, {} socket(s), seed {}, {when}",
                grown.built.triangles(),
                max[0] - min[0],
                max[1] - min[1],
                max[2] - min[2],
                grown.sockets.len(),
                planting.seed
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
    // What the vertex ids mean: every branch, its parent, its joint.
    let branches_path = format!("{base}.branches.json");
    if let Ok(json) = serde_json::to_string_pretty(&grown.branches) {
        if std::fs::write(&branches_path, json).is_ok() {
            println!("  branches → {branches_path} ({})", grown.branches.len());
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

    // The last LOD: eight views of the plant as a cross-quad billboard, with
    // its atlas beside it for looking at.
    if impostor {
        match chisel::impostor::impostor(&grown.built, 512) {
            Ok(imp) => {
                let ip = format!("{base}.impostor.glb");
                match chisel::model::export_glb(&imp) {
                    Ok(glb) if std::fs::write(&ip, &glb).is_ok() => {
                        println!("✓ impostor → {ip} — {} quads, {} px atlas", imp.parts[0].mesh.indices.len() / 6, imp.parts[0].baked.as_ref().map(|b| b.size).unwrap_or(0));
                    }
                    Ok(_) => eprintln!("⚠ cannot write {ip}"),
                    Err(e) => eprintln!("⚠ impostor export failed: {e}"),
                }
                if let Ok(png) = chisel::impostor::atlas_png(&imp) {
                    // `.atlas.png`, so it never collides with the impostor's own turntable.
                    let ap = format!("{base}.atlas.png");
                    if std::fs::write(&ap, png).is_ok() {
                        println!("  atlas → {ap}");
                    }
                }
                if let Some(shot) = preview {
                    // The impostor on the same turntable as the model, so the
                    // two can be held side by side.
                    let ishot = format!("{}.impostor.png", shot.trim_end_matches(".png"));
                    let opts = chisel::preview::PreviewOptions { views, ..Default::default() };
                    match chisel::preview::write_png(&imp, opts, &ishot) {
                        Ok(()) => println!("✓ preview → {ishot}"),
                        Err(e) => eprintln!("⚠ impostor preview failed: {e}"),
                    }
                }
            }
            Err(e) => eprintln!("✗ impostor: {e}"),
        }
    }

    // The part a cut would drop: the subtree on its own base, beside the tree.
    if let Some(id) = fallen_of {
        match fallen(&planting.species, planting.seed, planting.clock, &planting.state, id) {
            Ok(part) => {
                let fp = format!("{base}.fallen-{id:08x}.glb");
                match chisel::model::export_glb(&part.built) {
                    Ok(glb) if std::fs::write(&fp, &glb).is_ok() => {
                        let (min, max) = part.bounds;
                        println!(
                            "✓ fallen {id:08x} → {fp} — {} tris, {:.2} × {:.2} × {:.2} m, {} socket(s)",
                            part.built.triangles(),
                            max[0] - min[0],
                            max[1] - min[1],
                            max[2] - min[2],
                            part.sockets.len()
                        );
                    }
                    Ok(_) => eprintln!("⚠ cannot write {fp}"),
                    Err(e) => eprintln!("⚠ fallen export failed: {e}"),
                }
                if let Some(shot) = preview {
                    let fshot = format!("{}.fallen-{id:08x}.png", shot.trim_end_matches(".png"));
                    let opts = chisel::preview::PreviewOptions { views, ..Default::default() };
                    match chisel::preview::write_png(&part.built, opts, &fshot) {
                        Ok(()) => println!("✓ preview → {fshot}"),
                        Err(e) => eprintln!("⚠ fallen preview failed: {e}"),
                    }
                }
            }
            Err(e) => eprintln!("✗ fallen: {e}"),
        }
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
                println!("✓ {name} + {} × {} → {out_path}", placements.len(), thing.name);
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
    if let Some(sheet) = life {
        match life_sheet(&planting, sheet) {
            Ok(n) => println!("✓ life → {sheet} ({n} ages, one scale)"),
            Err(e) => eprintln!("⚠ life sheet failed: {e}"),
        }
    }
    if let Some(sheet) = year {
        match year_sheet(&planting, sheet) {
            Ok(n) => println!("✓ year → {sheet} ({n} seasons, one scale)"),
            Err(e) => eprintln!("⚠ year sheet failed: {e}"),
        }
    }
    // Same posture as `thread model`: --publish sends the PLANTING — species,
    // seed and clock — and the Quarry grows the plant itself, measuring its
    // sockets. Nothing uploaded.
    if publish {
        let quarry = std::env::var("QUARRY_URL").unwrap_or_else(|_| "https://quarry.pixygon.io".to_string());
        let submission = serde_json::json!({
            "title": title.cloned().unwrap_or_else(|| name.replace('-', " ")),
            "description": String::new(),
            "kind": kind.cloned().unwrap_or_else(|| "tree".into()),
            "style": style.cloned().unwrap_or_default(),
            "tags": tags
                .map(|t| t.split(',').map(|s| s.trim().to_string()).collect::<Vec<_>>())
                .unwrap_or_else(|| vec!["tree".into(), "grown".into()]),
            "package": "grove",
            "recipe": planting.to_value(),
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

/// The plant's whole life in one frame: the same individual at six moments,
/// standing in a row **at one scale**, youngest first.
///
/// A turntable per age would frame each one to fill its tile and so hide the
/// very thing age does. Here they share a camera, so a sapling is a sapling —
/// and because the seed decided the whole potential plant, every branch in the
/// young one is a branch of the old one.
fn life_sheet(planting: &Planting, path: &str) -> Result<usize, String> {
    // Six moments across the life curve: a sprout, a sapling, a young plant,
    // one filling out, one nearly there, and the grown plant.
    const MOMENTS: [f32; 6] = [0.12, 0.25, 0.4, 0.58, 0.78, 1.0];
    let shots: Vec<Planting> = MOMENTS
        .iter()
        .map(|m| {
            let mut at = planting.clone();
            at.clock.age = Some(m * planting.species.seasons_to_grown);
            at
        })
        .collect();
    row_sheet(&shots, path)
}

/// One plant, one year: bud, leaf, bloom, fruit, seed drop, bare — the same
/// individual six times, at one scale, in the order the year runs.
///
/// The wood does not move: what the year changes is what the plant is wearing
/// and what is hanging in it.
fn year_sheet(planting: &Planting, path: &str) -> Result<usize, String> {
    const SEASONS: [f32; 6] = [0.04, 0.18, 0.35, 0.55, 0.76, 0.92];
    let shots: Vec<Planting> = SEASONS
        .iter()
        .map(|season| {
            let mut at = planting.clone();
            at.clock.season = *season;
            at
        })
        .collect();
    row_sheet(&shots, path)
}

/// Grow each planting and stand them in a row under one camera, so the sheet
/// compares them instead of framing each one on its own terms.
fn row_sheet(shots: &[Planting], path: &str) -> Result<usize, String> {
    // The previewer's single view looks in from 35°, so the row is laid
    // broadside to it: six plants in a line, none behind another. Negated, so
    // the row reads left to right.
    let yaw = 35f32.to_radians();
    let (ax, az) = (yaw.sin(), -yaw.cos());
    let name = shots.first().map(|p| p.species.name.clone()).unwrap_or_default();
    let mut row = Built { name: format!("{name}-row"), parts: Vec::new() };
    let mut x = 0.0f32;
    let mut prev_half = 0.0f32;
    for (i, shot) in shots.iter().enumerate() {
        let g = grow_planting(shot)?;
        let (min, max) = g.bounds;
        let half = ((max[0] - min[0]).max(max[2] - min[2]) / 2.0).max(0.05);
        // Stand them a clear gap apart, each on its own centre line.
        x += prev_half + half * 1.15 + 0.2;
        prev_half = half * 1.15;
        let (dx, dz) = (ax * x, az * x);
        for p in &g.built.parts {
            let mut mesh = p.mesh.clone();
            for v in mesh.positions.iter_mut() {
                v[0] += dx;
                v[2] += dz;
            }
            row.parts.push(BuiltPart {
                name: format!("{}-{i}", p.name),
                mesh,
                baked: p.baked.clone(),
                color: p.color,
                emissive: p.emissive,
                double_sided: p.double_sided,
            });
        }
    }
    let opts =
        chisel::preview::PreviewOptions { width: 1600, height: 460, views: 1, pitch: 8.0, fill: 2.6, ..Default::default() };
    chisel::preview::write_png(&row, opts, path)?;
    Ok(shots.len())
}

/// A branch id as a person types it: the number from the sockets file, `0x…`,
/// or a socket's name (`tip-0000beef`, `cut-0000beef`, `fruit-0000beef-1`).
fn branch_id(v: &str) -> Option<u32> {
    if let Some(hex) = v.strip_prefix("0x") {
        return u32::from_str_radix(hex, 16).ok();
    }
    if let Ok(n) = v.parse::<u32>() {
        return Some(n);
    }
    let mut parts = v.split('-');
    let _kind = parts.next()?;
    u32::from_str_radix(parts.next()?, 16).ok()
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
