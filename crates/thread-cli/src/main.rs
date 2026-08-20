//! `thread` — the command-line companion for authoring worlds on the Thread.
//!
//!   thread init <name>        scaffold a new world you can host anywhere
//!   thread validate [path]    check a world.json against the spec
//!   thread doctor <host>      verify a live host serves a world correctly
//!   thread preview [path]     open a local world in the browser
//!
//! The whole point: get a domain, `thread init`, edit, drop it at
//! `https://yourdomain/.well-known/thread/`, `thread doctor yourdomain` — and
//! anyone can walk `thread://yourdomain`, with zero contact with anyone.

mod level;

use std::path::PathBuf;
use std::process::ExitCode;

use infinite_manifest::{markup, well_known_url, Locator, WorldManifest};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("help");
    // Flags may appear anywhere after the command; positionals are what's left.
    let markup_flag = args.iter().any(|a| a == "--markup");
    let rest: &[String] = args.get(1..).unwrap_or(&[]);
    let positionals: Vec<&String> = rest.iter().filter(|a| !a.starts_with("--")).collect();
    match cmd {
        "init" => cmd_init(positionals.first().copied(), markup_flag),
        "validate" => cmd_validate(args.get(1)),
        "lint" => cmd_lint(args.get(1)),
        "export" => cmd_export(rest),
        "model" => cmd_model(rest),
        "level" => cmd_level(rest),
        "compile" => cmd_compile(args.get(1)),
        "doctor" => cmd_doctor(args.get(1)),
        "preview" => cmd_preview(args.get(1)),
        "help" | "-h" | "--help" => {
            print_help();
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("unknown command '{other}'\n");
            print_help();
            ExitCode::FAILURE
        }
    }
}

fn print_help() {
    println!(
        "thread — author worlds on the Thread\n\n\
         USAGE:\n  \
         thread init <name>       scaffold a new world (--markup for a .thread source)\n  \
         thread lint <world>      quality findings (floating/buried/overflow/dark)\n  \
         thread export <world> --id <name> [-o out.glb]   carved shape -> glTF\n  \
         thread model <model|weftpack> [-o out.glb] [--preview sheet.png] [--publish]\n  \
         thread level --figure <hall|courtyard> --args '[…]' [--publish]\n  \
         thread validate [path]   check a world.json or .thread file (default ./world.json)\n  \
         thread compile <file>    compile a .thread markup file → world.json\n  \
         thread doctor <host>     verify a live host serves a world (host, host/path, or thread://…)\n  \
         thread preview [path]    open a local world in the browser\n\n\
         Publish: drop your world folder at https://<domain>/.well-known/thread/\n  \
         then anyone can walk  thread://<domain>"
    );
}

// --- init ---------------------------------------------------------------------

fn cmd_init(name: Option<&String>, use_markup: bool) -> ExitCode {
    let Some(name) = name else {
        eprintln!("usage: thread init <name> [--markup]");
        return ExitCode::FAILURE;
    };
    let dir = PathBuf::from(name);
    if dir.exists() {
        eprintln!("'{name}' already exists");
        return ExitCode::FAILURE;
    }
    let title = titleize(name);
    if let Err(e) = std::fs::create_dir_all(dir.join("assets")) {
        eprintln!("could not create {name}/: {e}");
        return ExitCode::FAILURE;
    }
    let (file, world) = if use_markup {
        ("world.thread", template_world_markup(name, &title))
    } else {
        ("world.json", template_world(name, &title))
    };
    // Sanity: the scaffold we ship must itself be conformant (either source form).
    if let Err(e) = WorldManifest::from_text(&world) {
        eprintln!("internal error: template is invalid: {e}");
        return ExitCode::FAILURE;
    }
    let _ = std::fs::write(dir.join(file), &world);
    let _ = std::fs::write(dir.join("README.md"), readme(name, &title, file));
    println!("✓ created {name}/{file}\n");
    println!("Next:");
    println!("  1. edit {name}/{file}");
    println!("  2. thread validate {name}/{file}");
    println!("  3. thread preview {name}/{file}");
    println!("  4. publish {name}/ to  https://<yourdomain>/.well-known/thread/");
    println!("  5. thread doctor <yourdomain>   →  walk  thread://<yourdomain>");
    ExitCode::SUCCESS
}

// --- validate -----------------------------------------------------------------

/// `thread export <world> --id <placement> [-o out.glb]` — a carved creation
/// leaves the Thread as a standard, self-contained binary glTF: the shape
/// meshed, its procedural material baked to embedded PNGs (albedo + normal +
/// occlusion-roughness-metallic). Opens in Blender, Unity, any glTF viewer.
fn cmd_export(args: &[String]) -> ExitCode {
    let mut world_path: Option<&String> = None;
    let mut id: Option<&String> = None;
    let mut out: Option<&String> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--id" => id = it.next(),
            "-o" | "--out" => out = it.next(),
            _ if !a.starts_with("--") && world_path.is_none() => world_path = Some(a),
            _ => {}
        }
    }
    let whole_world = args.iter().any(|a| a == "--world");
    let (Some(world_path), true) = (world_path, id.is_some() || whole_world) else {
        eprintln!("usage: thread export <world> (--id <placement-id> | --world) [-o out.glb]");
        return ExitCode::FAILURE;
    };
    let text = match std::fs::read_to_string(world_path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("✗ cannot read {world_path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let m = match WorldManifest::from_text(&text) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("✗ not a valid world: {e}");
            return ExitCode::FAILURE;
        }
    };
    if whole_world {
        return export_world(&m, world_path, out);
    }
    let id = id.expect("checked above");
    let Some(pl) = m
        .placements
        .iter()
        .flat_map(|p| p.iter_tree())
        .find(|p| &p.name == id)
    else {
        let named: Vec<&str> = m
            .placements
            .iter()
            .flat_map(|p| p.iter_tree())
            .map(|p| p.name.as_str())
            .filter(|n| !n.is_empty())
            .collect();
        eprintln!("✗ no placement named '{id}' — named placements: {named:?}");
        return ExitCode::FAILURE;
    };
    let Some(prefab) = m.prefabs.iter().find(|p| p.id == pl.prefab) else {
        eprintln!("✗ placement '{id}' has no prefab");
        return ExitCode::FAILURE;
    };
    let Some(shape) = &prefab.mesh.shape else {
        eprintln!("✗ '{id}' is not a carved shape (only shape prefabs export; glTF assets already are one)");
        return ExitCode::FAILURE;
    };
    let meshed = chisel::mesh(shape, prefab.mesh.resolution);
    let baked = prefab
        .material
        .as_ref()
        .and_then(|mat| mat.texture.as_ref())
        .map(chisel::texture::bake);
    let glb = match chisel::gltf::write_glb(&meshed, baked.as_ref(), id) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("✗ export failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    let out_path = out.cloned().unwrap_or_else(|| format!("{id}.glb"));
    if let Err(e) = std::fs::write(&out_path, &glb) {
        eprintln!("✗ cannot write {out_path}: {e}");
        return ExitCode::FAILURE;
    }
    println!(
        "✓ exported '{id}' → {out_path} ({} verts, {} tris{}, {:.0} KB)",
        meshed.positions.len(),
        meshed.indices.len() / 3,
        if baked.is_some() {
            ", baked PBR textures"
        } else {
            ""
        },
        glb.len() as f32 / 1024.0
    );
    ExitCode::SUCCESS
}

/// `thread model <model.json|pack.weftpack.json> [--entry name] [--args …]
/// [-o out.glb] [--preview shot.png] [--views n]`
///
/// The agent's modeling loop, in one command: take a model — written as data
/// or **computed by a Weft program** — carve it, bake its PBR materials,
/// write a standard `.glb`, and render a turntable proof sheet so the author
/// can SEE it without a GPU or a browser.
fn cmd_model(args: &[String]) -> ExitCode {
    let mut path: Option<&String> = None;
    let mut entry: Option<&String> = None;
    let mut arg_json: Option<&String> = None;
    let mut out: Option<&String> = None;
    let mut preview: Option<&String> = None;
    let mut views: u32 = 3;
    let mut lib: Option<&String> = None;
    let mut material: Option<&str> = None;
    let mut publish = false;
    let (mut title, mut tags, mut kind, mut style): (
        Option<&String>,
        Option<&String>,
        Option<&String>,
        Option<&String>,
    ) = (None, None, None, None);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--lib" | "-l" => lib = it.next(),
            "--entry" | "-e" => entry = it.next(),
            "--args" | "-a" => arg_json = it.next(),
            "-o" | "--out" => out = it.next(),
            "--preview" | "-p" => preview = it.next(),
            "--views" => views = it.next().and_then(|v| v.parse().ok()).unwrap_or(3),
            "--material" | "-m" => material = it.next().map(|s| s.as_str()),
            "--publish" => publish = true,
            "--title" => title = it.next(),
            "--tags" => tags = it.next(),
            "--kind" => kind = it.next(),
            "--style" => style = it.next(),
            _ if !a.starts_with('-') && path.is_none() => path = Some(a),
            _ => {}
        }
    }
    if path.is_none() && lib.is_none() {
        eprintln!(
            "usage: thread model --lib <part> [--args '[5.2, 0.44]'] [-o out.glb] [--preview sheet.png]\n   \
             or: thread model <model.json>\n   \
             or: thread model <package.weftpack.json> --entry <export> [--args …]\n\n\
             The built-in library (--lib) is `weft-model`: parts, materials and\n\
             the words to combine them. List them with `thread model --lib ?`."
        );
        return ExitCode::FAILURE;
    }
    // `--lib` reaches for the built-in modeling library — no file, no setup:
    // the standard library ships inside the tool, the way three.js ships
    // geometries. Every export is a verified, content-addressed program.
    let text = match lib {
        Some(_) => serde_json::to_string(&weft::model_lib::package()).unwrap_or_default(),
        None => match std::fs::read_to_string(path.expect("checked")) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("✗ cannot read {}: {e}", path.expect("checked"));
                return ExitCode::FAILURE;
            }
        },
    };
    if let Some(name) = lib {
        if name == "?" || name == "list" {
            let pkg = weft::model_lib::package();
            println!("weft-model — {} exports:", pkg.exports.len());
            for (petname, hash) in &pkg.exports {
                let d = &pkg.defs[hash];
                println!("  {petname:<12} {} arg(s)   {hash}", d.params.len());
            }
            return ExitCode::SUCCESS;
        }
    }
    let entry = lib.or(entry);

    // Data form, or code form: a Weft package + an export to call. The code
    // form is the one that scales — parameters, loops, verified and hashed.
    let args_json: Vec<serde_json::Value> = match arg_json {
        None => Vec::new(),
        Some(a) => match serde_json::from_str(a) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("✗ --args must be a JSON array: {e}");
                return ExitCode::FAILURE;
            }
        },
    };
    let model: infinite_manifest::model::Model = match entry {
        None => match serde_json::from_str(&text) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("✗ not a model: {e}\n  (a Weft package? pass --entry <export>)");
                return ExitCode::FAILURE;
            }
        },
        Some(export) => {
            match chisel::weft_model::eval_model_or_part(&text, export, &args_json, material) {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("✗ {e}");
                    return ExitCode::FAILURE;
                }
            }
        }
    };

    let built = match chisel::model::build(&model) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("✗ {e}");
            return ExitCode::FAILURE;
        }
    };
    let stem = if model.name.is_empty() {
        "model".to_string()
    } else {
        model.name.clone()
    };
    let out_path = out.cloned().unwrap_or_else(|| format!("{stem}.glb"));
    match chisel::model::export_glb(&built) {
        Ok(glb) => {
            if let Err(e) = std::fs::write(&out_path, &glb) {
                eprintln!("✗ cannot write {out_path}: {e}");
                return ExitCode::FAILURE;
            }
            println!(
                "✓ {} → {out_path} — {} part(s), {} tris, {:.0} KB, PBR complete",
                model.name,
                built.parts.len(),
                built.triangles(),
                glb.len() as f32 / 1024.0
            );
        }
        Err(e) => {
            eprintln!("✗ export failed: {e}");
            return ExitCode::FAILURE;
        }
    }
    if let Some(shot) = preview {
        let opts = chisel::preview::PreviewOptions {
            views,
            ..Default::default()
        };
        match chisel::preview::write_png(&built, opts, shot) {
            Ok(()) => println!("✓ preview → {shot} ({views}-view turntable)"),
            Err(e) => eprintln!("⚠ preview failed: {e}"),
        }
    }
    // Sharing is the default posture, not a chore: `--publish` sends the
    // RECIPE to the Quarry, which derives the artifact itself. Nothing is
    // uploaded, so nothing can drift from what the program actually makes.
    if publish {
        let Some(export) = entry else {
            eprintln!("⚠ --publish needs a recipe (use --lib <part> or --entry <export>)");
            return ExitCode::FAILURE;
        };
        let quarry =
            std::env::var("QUARRY_URL").unwrap_or_else(|_| "https://quarry.pixygon.io".to_string());
        let submission = serde_json::json!({
            "title": title.cloned().unwrap_or_else(|| pretty_title(export, &args_json)),
            "description": String::new(),
            "kind": kind.cloned().unwrap_or_else(|| export.clone()),
            "style": style.cloned().unwrap_or_default(),
            "tags": tags
                .map(|t| t.split(',').map(|s| s.trim().to_string()).collect::<Vec<_>>())
                .unwrap_or_else(|| vec![export.clone()]),
            "package": "weft-model",
            "export": export,
            "args": args_json,
            "material": material.unwrap_or(""),
            "origin": "authored",
        });
        match post_json(
            &format!("{}/publish", quarry.trim_end_matches('/')),
            &submission,
        ) {
            Ok(body) => {
                let design = serde_json::from_str::<serde_json::Value>(&body)
                    .ok()
                    .and_then(|v| v["design"].as_str().map(str::to_string))
                    .unwrap_or_default();
                println!("✓ published to the Quarry — {quarry}/models/{design}.glb");
            }
            Err(e) => eprintln!("⚠ publish failed: {e}"),
        }
    }
    ExitCode::SUCCESS
}

/// "column" + [5.2, 0.44] → "Column 5.2 × 0.44" — a title an agent doesn't
/// have to think about, overridable with `--title`.
/// One small blocking GET, curl for TLS — same reasoning as `post_json`.
pub(crate) fn http_get(url: &str) -> Result<String, String> {
    let out = std::process::Command::new("curl")
        .args(["-s", "--max-time", "20", url])
        .output()
        .map_err(|e| format!("curl: {e}"))?;
    if !out.status.success() {
        return Err(format!("GET {url} failed"));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn pretty_title(export: &str, args: &[serde_json::Value]) -> String {
    let mut name = export.to_string();
    if let Some(c) = name.get_mut(0..1) {
        c.make_ascii_uppercase();
    }
    if args.is_empty() {
        return name;
    }
    let nums: Vec<String> = args
        .iter()
        .map(|a| {
            a.as_f64()
                .map(|f| format!("{f:.2}").trim_end_matches(['0', '.']).to_string())
                .unwrap_or_default()
        })
        .filter(|s| !s.is_empty())
        .collect();
    format!("{name} {}", nums.join(" × "))
}

/// One small blocking POST — the CLI has no async runtime and needs none.
pub(crate) fn post_json(url: &str, body: &serde_json::Value) -> Result<String, String> {
    let text = body.to_string();
    // A store that DERIVES what it is sent is doing real work per request, so
    // it may be gated. Carry the token when the environment has one.
    let token = std::env::var("QUARRY_TOKEN").ok().filter(|t| !t.is_empty());
    let rest = url
        .strip_prefix("https://")
        .map(|r| (r, 443u16, true))
        .or_else(|| url.strip_prefix("http://").map(|r| (r, 80u16, false)));
    let Some((rest, default_port, tls)) = rest else {
        return Err("bad URL".into());
    };
    let (hostport, path) = rest.split_once('/').unwrap_or((rest, ""));
    let (host, port) = match hostport.split_once(':') {
        Some((h, p)) => (h, p.parse().unwrap_or(default_port)),
        None => (hostport, default_port),
    };
    let auth = token
        .as_ref()
        .map(|t| format!("authorization: Bearer {t}\r\n"))
        .unwrap_or_default();
    let request = format!(
        "POST /{path} HTTP/1.1\r\nhost: {host}\r\ncontent-type: application/json\r\n{auth}content-length: {}\r\nconnection: close\r\n\r\n{text}",
        text.len()
    );
    use std::io::{Read, Write};
    let mut raw = std::net::TcpStream::connect((host, port)).map_err(|e| e.to_string())?;
    let mut response = String::new();
    if tls {
        // Delegate TLS to curl rather than carrying a TLS stack in the CLI.
        drop(raw);
        let mut cmd = std::process::Command::new("curl");
        cmd.args([
            "-s",
            "-X",
            "POST",
            url,
            "-H",
            "content-type: application/json",
        ]);
        if let Some(t) = &token {
            cmd.args(["-H", &format!("authorization: Bearer {t}")]);
        }
        let out = cmd
            .args(["--data-binary", &text])
            .output()
            .map_err(|e| format!("curl: {e}"))?;
        return Ok(String::from_utf8_lossy(&out.stdout).to_string());
    }
    raw.write_all(request.as_bytes())
        .map_err(|e| e.to_string())?;
    raw.read_to_string(&mut response)
        .map_err(|e| e.to_string())?;
    Ok(response.split("\r\n\r\n").nth(1).unwrap_or("").to_string())
}

/// `--world`: every top-level placement whose prefab is carved or builtin,
/// as one glTF scene — meshes shared across placements, materials baked.
/// Child placements (relative transforms) are skipped in v1 and counted.
fn export_world(m: &WorldManifest, world_path: &str, out: Option<&String>) -> ExitCode {
    use std::collections::HashMap;
    let mut mesh_store: Vec<(
        chisel::MeshData,
        Option<chisel::texture::Baked>,
        [f32; 4],
        String,
    )> = Vec::new();
    let mut by_prefab: HashMap<u64, usize> = HashMap::new();
    let mut nodes: Vec<(String, usize, [f32; 3], [f32; 4], [f32; 3])> = Vec::new();
    let mut skipped_children = 0usize;
    let mut skipped_assets = 0usize;

    for (i, pl) in m.placements.iter().enumerate() {
        skipped_children += pl.children.len();
        let Some(prefab) = m.prefabs.iter().find(|p| p.id == pl.prefab) else {
            continue;
        };
        let entry = if let Some(idx) = by_prefab.get(&(prefab.id.0 as u64)) {
            Some(*idx)
        } else {
            let mesh = if let Some(shape) = &prefab.mesh.shape {
                Some(chisel::mesh(shape, prefab.mesh.resolution))
            } else {
                prefab.mesh.builtin.as_deref().and_then(builtin_meshdata)
            };
            match mesh {
                Some(mesh) => {
                    let baked = prefab
                        .material
                        .as_ref()
                        .and_then(|mat| mat.texture.as_ref())
                        .map(chisel::texture::bake);
                    let color = prefab
                        .material
                        .as_ref()
                        .map(|mat| mat.base_color)
                        .unwrap_or([1.0; 4]);
                    mesh_store.push((mesh, baked, color, format!("prefab-{}", prefab.id.0)));
                    by_prefab.insert(prefab.id.0 as u64, mesh_store.len() - 1);
                    Some(mesh_store.len() - 1)
                }
                None => {
                    skipped_assets += 1;
                    None
                }
            }
        };
        if let Some(mesh_idx) = entry {
            let name = if pl.name.is_empty() {
                format!("placement-{i}")
            } else {
                pl.name.clone()
            };
            nodes.push((name, mesh_idx, pl.position, pl.rotation, pl.scale));
        }
    }
    if nodes.is_empty() {
        eprintln!("✗ nothing exportable (no carved or builtin placements)");
        return ExitCode::FAILURE;
    }
    let meshes: Vec<chisel::gltf::SceneMesh> = mesh_store
        .iter()
        .map(|(mesh, baked, color, name)| chisel::gltf::SceneMesh {
            name: name.clone(),
            mesh,
            baked: baked.as_ref(),
            base_color: *color,
            emissive: 0.0,
        })
        .collect();
    let scene_nodes: Vec<chisel::gltf::SceneNode> = nodes
        .iter()
        .map(|(name, mesh, t, r, sc)| chisel::gltf::SceneNode {
            name: name.clone(),
            mesh: *mesh,
            translation: *t,
            rotation: *r,
            scale: *sc,
        })
        .collect();
    let glb = match chisel::gltf::write_glb_scene(&meshes, &scene_nodes) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("✗ export failed: {e}");
            return ExitCode::FAILURE;
        }
    };
    let default_name = std::path::Path::new(world_path)
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| format!("{}.glb", n.to_string_lossy()))
        .unwrap_or_else(|| "world.glb".into());
    let out_path = out.cloned().unwrap_or(default_name);
    if let Err(e) = std::fs::write(&out_path, &glb) {
        eprintln!("✗ cannot write {out_path}: {e}");
        return ExitCode::FAILURE;
    }
    let notes = match (skipped_children, skipped_assets) {
        (0, 0) => String::new(),
        (c, a) => format!(" (skipped: {c} child placement(s), {a} asset-mesh prefab(s))"),
    };
    println!(
        "✓ exported '{}' → {out_path} — {} node(s), {} unique mesh(es), {:.0} KB{notes}",
        m.world.title,
        scene_nodes.len(),
        meshes.len(),
        glb.len() as f32 / 1024.0
    );
    ExitCode::SUCCESS
}

/// Builtin primitive → mesh. The geometry itself lives in the mesher
/// ([`chisel::builtin`]) because it is spec surface, not a private choice:
/// every browser that reads `"builtin": "cube"` has to draw the same box.
fn builtin_meshdata(name: &str) -> Option<chisel::MeshData> {
    chisel::builtin::mesh(name)
}

/// `thread level --figure hall --args '[…]'` — lay out a place, work out
/// what it needs, take what the store has, commission the rest, and write a
/// world. The design is a verified program; the shopping is not.
fn cmd_level(args: &[String]) -> ExitCode {
    let mut figure: Option<&String> = None;
    let mut arg_json: Option<&String> = None;
    let mut out: Option<&String> = None;
    let mut quarry: Option<&String> = None;
    let mut relay: Option<&String> = None;
    let mut publish = false;
    let mut no_store = false;
    let mut dry = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--figure" | "-f" => figure = it.next(),
            "--args" | "-a" => arg_json = it.next(),
            "-o" | "--out" => out = it.next(),
            "--quarry" => quarry = it.next(),
            "--relay" => relay = it.next(),
            "--publish" => publish = true,
            "--no-store" => no_store = true,
            "--dry" => dry = true,
            _ => {}
        }
    }
    let Some(figure) = figure else {
        eprintln!(
            "usage: thread level --figure <hall|courtyard|…> --args '[\"Name\", 14, 5.2, 12, \"classical\", \"marble\", \"dusk\"]'\n   \
             [-o world.json] [--relay wss://…] [--quarry URL] [--publish] [--no-store] [--dry]\n\n\
             Figures come from the built-in `weft-draft` library; list them with --figure ?"
        );
        return ExitCode::FAILURE;
    };
    let library = match serde_json::to_string(&weft::draft_lib::package()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("✗ drafting library: {e}");
            return ExitCode::FAILURE;
        }
    };
    if figure == "?" || figure == "list" {
        let pkg = weft::draft_lib::package();
        println!("weft-draft — {} exports:", pkg.exports.len());
        for (petname, hash) in &pkg.exports {
            println!(
                "  {petname:<12} {} arg(s)   {hash}",
                pkg.defs[hash].params.len()
            );
        }
        return ExitCode::SUCCESS;
    }
    let args_json: Vec<serde_json::Value> = match arg_json {
        None => Vec::new(),
        Some(a) => match serde_json::from_str(a) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("✗ --args must be a JSON array: {e}");
                return ExitCode::FAILURE;
            }
        },
    };
    // 1. The layout: a verified program, from a brief to a bill of needs.
    let plan_json = match chisel::weft_model::eval_export(&library, figure, &args_json) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("✗ {e}");
            return ExitCode::FAILURE;
        }
    };
    let plan: infinite_manifest::plan::Plan = match serde_json::from_value(plan_json) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("✗ '{figure}' did not return a plan: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!(
        "▸ {} — {} need(s), {} built piece(s), {} light(s), {} veil(s)",
        plan.name,
        plan.needs.len(),
        plan.builds.len(),
        plan.lights.len(),
        plan.veils.len()
    );
    // 2. Check the layout before shopping for it: a plan with a veil on the
    // spawn is not a plan worth furnishing.
    let faults = plan.check();
    for f in &faults {
        println!("  ⚠ {f}");
    }
    if dry {
        println!(
            "{}",
            serde_json::to_string_pretty(&plan).unwrap_or_default()
        );
        return ExitCode::SUCCESS;
    }
    if no_store && publish {
        eprintln!("✗ --no-store builds without a store; --publish needs one. Pick one.");
        return ExitCode::FAILURE;
    }
    // 3. Bind: shop, commission, emit.
    let opts = level::Options {
        quarry: quarry.cloned().unwrap_or_else(|| {
            std::env::var("QUARRY_URL").unwrap_or_else(|_| "https://quarry.pixygon.io".into())
        }),
        publish,
        verbose: true,
        relay: relay
            .cloned()
            .unwrap_or_else(|| std::env::var("THREAD_RELAY").unwrap_or_default()),
        // Models land beside the world, so the pair travels together.
        local: no_store.then(|| {
            out.map(std::path::PathBuf::from)
                .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or_else(|| std::path::PathBuf::from("."))
        }),
    };
    let (manifest, bound) = match level::build(&plan, &opts) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("✗ {e}");
            return ExitCode::FAILURE;
        }
    };
    let (mut stock, mut scaled, mut made, mut unmet) = (0, 0, 0, 0);
    let mut seen: std::collections::BTreeSet<String> = Default::default();
    for (need, b) in &bound {
        match b.source {
            level::Source::Stock => stock += 1,
            level::Source::Scaled => scaled += 1,
            level::Source::Commissioned => made += 1,
            level::Source::Unmet => {
                unmet += 1;
                println!("  ✗ unmet: {} — {}", need.kind, b.note);
            }
        }
        if seen.insert(format!("{}|{}", b.design, b.title)) && b.source != level::Source::Unmet {
            println!(
                "  {} {:<26} {}",
                match b.source {
                    level::Source::Stock => "◆ stock      ",
                    level::Source::Scaled => "◇ scaled     ",
                    level::Source::Commissioned => "✦ commissioned",
                    level::Source::Unmet => "✗            ",
                },
                b.title,
                b.note
            );
        }
    }
    println!(
        "▸ bound {} need(s): {stock} from stock, {scaled} scaled, {made} commissioned, {unmet} unmet",
        bound.len()
    );
    // 4. Emit, and lint what we emitted — real models have real sizes, so
    // the layout's promises get checked against the furniture that arrived.
    let out_path = out
        .cloned()
        .unwrap_or_else(|| format!("{}.json", slug_name(&plan.name)));
    match std::fs::write(&out_path, manifest.to_json()) {
        Ok(()) => println!("✓ world → {out_path}"),
        Err(e) => {
            eprintln!("✗ cannot write {out_path}: {e}");
            return ExitCode::FAILURE;
        }
    }
    let findings = infinite_manifest::lint::lint(&manifest);
    if findings.is_empty() {
        println!("✓ lints clean");
    } else {
        for f in findings.iter().take(8) {
            println!("  ⚠ {f}");
        }
    }
    ExitCode::SUCCESS
}

fn slug_name(name: &str) -> String {
    let s: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    s.split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// `thread lint <world>` — the quality eye: everything conformance can't say
/// but a visitor would feel. Floating furniture, buried spawns, veils that
/// fire on arrival, overflowing text, dark worlds. Advisory: findings never
/// fail the exit code unless `--strict` becomes a thing; the point is to see
/// them BEFORE the first screenshot.
fn cmd_lint(path: Option<&String>) -> ExitCode {
    let path = resolve_manifest_path(path);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("✗ cannot read {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
    };
    let m = match WorldManifest::from_text(&text) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("✗ not a valid world (lint runs after validity): {e}");
            return ExitCode::FAILURE;
        }
    };
    let findings = infinite_manifest::lint::lint(&m);
    if findings.is_empty() {
        println!(
            "✓ '{}' lints clean — nothing floats, nothing's buried, the copy fits.",
            m.world.title
        );
    } else {
        println!("'{}' — {} finding(s):", m.world.title, findings.len());
        for f in &findings {
            println!("  ⚠ {f}");
        }
        println!(
            "
(advisory — the world is still valid; these are what a visitor would notice)"
        );
    }
    ExitCode::SUCCESS
}

fn cmd_validate(path: Option<&String>) -> ExitCode {
    let path = resolve_manifest_path(path);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("✗ cannot read {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
    };
    // `from_text` accepts both source forms: JSON and `.thread` markup.
    match WorldManifest::from_text(&text) {
        Ok(m) => {
            println!(
                "✓ {} is a valid thread/0.x world — '{}' ({} prefab(s), {} placement(s), {} portal(s))",
                path.display(),
                m.world.title,
                m.prefabs.len(),
                m.placements.len(),
                m.portals.len(),
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("✗ {} is not conformant:\n  {e}", path.display());
            ExitCode::FAILURE
        }
    }
}

// --- compile ------------------------------------------------------------------

/// Compile a `.thread` markup file (HTML/CSS-like) to a `world.json` manifest —
/// the source → DOM step. Writes alongside the input (or `world.json`).
fn cmd_compile(path: Option<&String>) -> ExitCode {
    let Some(path) = path else {
        eprintln!("usage: thread compile <file.thread>");
        return ExitCode::FAILURE;
    };
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("✗ cannot read {path}: {e}");
            return ExitCode::FAILURE;
        }
    };
    match markup::compile(&src) {
        Ok(manifest) => {
            let out = PathBuf::from(path).with_extension("json");
            match std::fs::write(&out, manifest.to_json()) {
                Ok(()) => {
                    println!(
                        "✓ compiled {path} → {} ('{}', {} placement(s), {} portal(s), {} style rule(s))",
                        out.display(),
                        manifest.world.title,
                        manifest.placements.len(),
                        manifest.portals.len(),
                        manifest.styles.len(),
                    );
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("✗ cannot write {}: {e}", out.display());
                    ExitCode::FAILURE
                }
            }
        }
        Err(e) => {
            eprintln!("✗ {path} did not compile:\n  {e}");
            ExitCode::FAILURE
        }
    }
}

// --- doctor -------------------------------------------------------------------

fn cmd_doctor(target: Option<&String>) -> ExitCode {
    let Some(target) = target else {
        eprintln!("usage: thread doctor <host | host/path | thread://…>");
        return ExitCode::FAILURE;
    };
    let (host, path) = split_target(target);
    let url = well_known_url(&host, &path);
    println!(
        "checking  thread://{host}{}  →  {url}\n",
        if path.is_empty() {
            String::new()
        } else {
            format!("/{path}")
        }
    );

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let mut report = rt.block_on(doctor_fetch(&url));
    // A host may serve Thread markup instead of JSON — same convention, `.thread`
    // extension (exactly like serving HTML). Fall back before failing the host.
    if report.first().is_some_and(|c| !c.pass) {
        let markup_url = format!("{}world.thread", url.trim_end_matches("world.json"));
        let markup_report = rt.block_on(doctor_fetch(&markup_url));
        if markup_report.first().is_some_and(|c| c.pass) {
            println!("  (world.json absent — found {markup_url})\n");
            report = markup_report;
        }
    }
    let mut ok = true;
    for check in &report {
        println!(
            "  {} {}{}",
            if check.pass { "✓" } else { "✗" },
            check.name,
            if check.detail.is_empty() {
                String::new()
            } else {
                format!(" — {}", check.detail)
            }
        );
        ok &= check.pass;
    }
    println!();
    if ok {
        println!("✓ thread://{host} is live and walkable. Ship it.");
        ExitCode::SUCCESS
    } else {
        println!("✗ not walkable yet — fix the ✗ items above.");
        ExitCode::FAILURE
    }
}

struct Check {
    name: &'static str,
    pass: bool,
    detail: String,
}

async fn doctor_fetch(url: &str) -> Vec<Check> {
    let mut checks = Vec::new();
    let resp = match reqwest::get(url).await {
        Ok(r) => r,
        Err(e) => {
            checks.push(Check {
                name: "reachable over HTTPS",
                pass: false,
                detail: e.to_string(),
            });
            return checks;
        }
    };
    let status = resp.status();
    checks.push(Check {
        name: "reachable over HTTPS",
        pass: status.is_success(),
        detail: format!("HTTP {}", status.as_u16()),
    });

    let headers = resp.headers().clone();
    let ctype = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    if url.ends_with(".thread") {
        checks.push(Check {
            name: "Content-Type is text",
            pass: ctype.starts_with("text/"),
            detail: ctype,
        });
    } else {
        checks.push(Check {
            name: "Content-Type is JSON",
            pass: ctype.contains("json"),
            detail: ctype,
        });
    }

    let cors = headers
        .get("access-control-allow-origin")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    checks.push(Check {
        name: "CORS allows any origin",
        pass: cors == "*",
        detail: if cors.is_empty() {
            "missing Access-Control-Allow-Origin".into()
        } else {
            cors
        },
    });

    let body = resp.text().await.unwrap_or_default();
    match WorldManifest::from_text(&body) {
        Ok(m) => {
            checks.push(Check {
                name: "valid World Manifest",
                pass: true,
                detail: format!("'{}'", m.world.title),
            });
            let bad: Vec<_> = m
                .portals
                .iter()
                .filter(|p| Locator::parse(&p.to).is_none())
                .map(|p| p.id.clone())
                .collect();
            checks.push(Check {
                name: "portals address the Thread",
                pass: bad.is_empty(),
                detail: if bad.is_empty() {
                    format!("{} portal(s) OK", m.portals.len())
                } else {
                    format!("invalid: {}", bad.join(", "))
                },
            });
        }
        Err(e) => checks.push(Check {
            name: "valid World Manifest",
            pass: false,
            detail: e.to_string(),
        }),
    }
    checks
}

// --- preview ------------------------------------------------------------------

fn cmd_preview(path: Option<&String>) -> ExitCode {
    let path = resolve_manifest_path(path);
    let abs = std::fs::canonicalize(&path).unwrap_or(path.clone());
    match find_browser() {
        Some(bin) => {
            println!("opening {} in {}…", abs.display(), bin.display());
            match std::process::Command::new(&bin).arg(&abs).status() {
                Ok(s) if s.success() => ExitCode::SUCCESS,
                Ok(_) => ExitCode::FAILURE,
                Err(e) => {
                    eprintln!("could not launch the browser: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        None => {
            println!("The Thread browser isn't installed. To preview this world:");
            println!("  cargo run --bin infinite-wgpu --no-default-features --features wgpu-backend -- {}", abs.display());
            println!("or download it and run:  infinite-thread {}", abs.display());
            ExitCode::SUCCESS
        }
    }
}

/// Find a Thread browser binary: env override, on PATH, or a local release build.
fn find_browser() -> Option<PathBuf> {
    if let Ok(b) = std::env::var("INFINITE_BROWSER") {
        let p = PathBuf::from(b);
        if p.exists() {
            return Some(p);
        }
    }
    for name in ["infinite-thread", "infinite-wgpu"] {
        if let Ok(paths) = std::env::var("PATH") {
            for dir in std::env::split_paths(&paths) {
                let cand = dir.join(name);
                if cand.is_file() {
                    return Some(cand);
                }
            }
        }
    }
    let local = PathBuf::from("target/release/infinite-wgpu");
    local.is_file().then_some(local)
}

// --- helpers ------------------------------------------------------------------

fn resolve_manifest_path(path: Option<&String>) -> PathBuf {
    match path {
        Some(p) => {
            let pb = PathBuf::from(p);
            if pb.is_dir() {
                // A directory holds either source form; JSON wins when both exist.
                let json = pb.join("world.json");
                let thread = pb.join("world.thread");
                if !json.exists() && thread.exists() {
                    thread
                } else {
                    json
                }
            } else {
                pb
            }
        }
        None => PathBuf::from("world.json"),
    }
}

/// Split a doctor target into `(host, path)`.
fn split_target(target: &str) -> (String, String) {
    if let Some(loc) = Locator::parse(target) {
        return (loc.host, loc.path);
    }
    let t = target
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');
    match t.split_once('/') {
        Some((h, p)) => (h.to_string(), p.to_string()),
        None => (t.to_string(), String::new()),
    }
}

fn titleize(name: &str) -> String {
    name.split(['-', '_', ' '])
        .filter(|s| !s.is_empty())
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn readme(name: &str, title: &str, file: &str) -> String {
    let ctype = if file.ends_with(".thread") {
        "text/plain"
    } else {
        "application/json"
    };
    format!(
        "# {title}\n\nA world on the Thread.\n\n\
         ## Edit\n\nEdit `{file}` (spec: World Manifest v0.1 / Thread Markup v0.1). `thread validate` checks it.\n\n\
         ## Preview\n\n```\nthread preview {file}\n```\n\n\
         ## Publish\n\nPut this folder at `https://<yourdomain>/.well-known/thread/` on any static host\n\
         (Netlify, S3, GitHub Pages, nginx…). Serve `{file}` with:\n\
         - `Content-Type: {ctype}`\n- `Access-Control-Allow-Origin: *`\n\n\
         Then check it and share:\n\n```\nthread doctor <yourdomain>\n# → walk  thread://<yourdomain>\n```\n\n\
         _World id: `{name}`._\n"
    )
}

/// The `--markup` scaffold — the same starter place as [`template_world`], but
/// authored in Thread markup (the "HTML" form). Behaviors are left out: the
/// markup floor is codeless; interactions come from `<style>` rules.
fn template_world_markup(id: &str, title: &str) -> String {
    format!(
        r##"<!-- {title} — a world on the Thread. This file IS the world:
     browsers compile it on arrival. `thread validate` checks it. -->
<world id="{id}" title="{title}" description="A world on the Thread. Edit me!"
       sky="0.05 0.06 0.12 / 0.18 0.16 0.24">
  <spawn at="0 0 8" yaw="180"/>

  <plane class="floor" at="0 0 0" scale="24 1 24"/>
  <cube id="pedestal" at="0 0.5 0" scale="0.8 1 0.8" codex="the-thread"/>
  <quad id="sign" at="0 1.4 -6" scale="5 2 0.2"
        url="https://example.com" data-tagline="Your site, inside the Thread."/>

  <portal to="thread://pixygon.io#entry" at="-8 1.2 0" scale="2 3 0.2" label="The Nexus"/>
</world>
<style>
  .floor {{ color: 0.14 0.13 0.18; roughness: 0.9 }}
  #pedestal {{ color: 0.45 0.5 0.7; roughness: 0.5; metallic: 0.1 }}
  #sign {{ color: 0.2 0.25 0.35; roughness: 0.8 }}
</style>
"##
    )
}

fn template_world(id: &str, title: &str) -> String {
    format!(
        r##"{{
  "thread": "thread/0.1",
  "world": {{
    "id": "{id}",
    "title": "{title}",
    "description": "A world on the Thread. Edit me!",
    "author": {{ "id": "did:web:example.com", "name": "You" }},
    "license": "CC-BY-4.0"
  }},
  "environment": {{
    "year": 0,
    "sky": {{ "zenith": [0.05, 0.06, 0.12], "horizon": [0.18, 0.16, 0.24], "sun_dir": [0.3, 0.7, 0.2] }}
  }},
  "spawns": [{{ "name": "entry", "position": [0, 0, 8], "yaw": 3.14159 }}],
  "prefabs": [
    {{ "id": "60000001", "mesh": {{ "builtin": "plane" }}, "material": {{ "base_color": [0.14, 0.13, 0.18, 1], "roughness": 0.9 }} }},
    {{ "id": "60000002", "mesh": {{ "builtin": "cube" }}, "material": {{ "base_color": [0.45, 0.5, 0.7, 1], "roughness": 0.5, "metallic": 0.1 }} }},
    {{ "id": "60000003", "mesh": {{ "builtin": "quad" }}, "material": {{ "base_color": [0.2, 0.25, 0.35, 1], "roughness": 0.8 }} }}
  ],
  "placements": [
    {{ "prefab": "60000001", "name": "floor", "position": [0, 0, 0], "scale": [24, 1, 24] }},
    {{ "prefab": "60000002", "name": "pedestal", "position": [0, 0.5, 0], "scale": [0.8, 1, 0.8], "codex": "the-thread", "behavior": "codex-viewer" }},
    {{ "prefab": "60000003", "name": "signboard", "position": [0, 1.4, -6], "scale": [5, 2, 0.2], "behavior": "open-link", "data": {{ "kind": "signboard", "url": "https://example.com", "tagline": "Your site, inside the Thread." }} }}
  ],
  "portals": [
    {{ "id": "to-nexus", "position": [-8, 1.2, 0], "scale": [2, 3, 0.2], "to": "thread://pixygon.io#entry", "label": "The Nexus", "preview": "live" }}
  ],
  "behaviors": [
    {{ "id": "codex-viewer", "wasm": "codex-viewer-wasm", "on": ["interact"] }},
    {{ "id": "open-link", "wasm": "open-link-wasm", "on": ["interact"] }}
  ],
  "assets": [
    {{ "id": "codex-viewer-wasm", "uri": "behaviors/codex-viewer.wasm", "kind": "wasm" }},
    {{ "id": "open-link-wasm", "uri": "behaviors/open-link.wasm", "kind": "wasm" }}
  ],
  "presence": {{ "relay": null, "max_occupants": 32, "voice": true }}
}}
"##
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaffold_template_is_conformant() {
        let w = template_world("my-world", "My World");
        WorldManifest::from_json(&w).expect("scaffolded world must validate");
    }

    #[test]
    fn markup_scaffold_is_conformant() {
        let w = template_world_markup("my-world", "My World");
        let m = WorldManifest::from_text(&w).expect("markup scaffold must compile + validate");
        assert_eq!(m.world.id, "my-world");
        assert!(!m.spawns.is_empty());
        assert_eq!(m.portals.len(), 1);
    }

    #[test]
    fn titleizes_names() {
        assert_eq!(titleize("the-grand-hall"), "The Grand Hall");
        assert_eq!(titleize("gallery"), "Gallery");
    }

    #[test]
    fn splits_doctor_targets() {
        assert_eq!(
            split_target("example.com"),
            ("example.com".into(), String::new())
        );
        assert_eq!(
            split_target("example.com/gallery"),
            ("example.com".into(), "gallery".into())
        );
        assert_eq!(
            split_target("thread://studio.io/room#a"),
            ("studio.io".into(), "room".into())
        );
        assert_eq!(
            split_target("https://x.org/y"),
            ("x.org".into(), "y".into())
        );
    }

    /// The primitives themselves are measured where they are made
    /// (`chisel::builtin`); what this pins is that the CLI *asks* for them
    /// rather than carrying a second copy that could drift from the mesher's.
    #[test]
    fn the_cli_takes_its_primitives_from_the_mesher() {
        for name in chisel::builtin::NAMES {
            let ours = builtin_meshdata(name).expect(name);
            let theirs = chisel::builtin::mesh(name).expect(name);
            assert_eq!(ours.positions, theirs.positions, "{name}");
            assert_eq!(ours.indices, theirs.indices, "{name}");
        }
        assert!(builtin_meshdata("dodecahedron").is_none());
    }
}
