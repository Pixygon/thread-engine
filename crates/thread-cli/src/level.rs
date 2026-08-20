//! `thread level` — turn a *plan of requirements* into a world.
//!
//! The drafting library ([`weft::draft_lib`]) computes what a place needs
//! without naming a single file. This is the part that goes shopping: for
//! every need it asks [the Quarry](https://quarry.pixygon.io) for something
//! that fits, **commissions** what the shelf lacks (evaluating the modeling
//! library, then publishing the result so the next place gets it for free),
//! and emits a conformant World Manifest.
//!
//! The division of labour is the point. Layout is a verified program: same
//! brief, same place, forever. Shopping is an effect, and effects live out
//! here — with **no discretion**: the binder may match, it may scale a model
//! by up to 15 %, and it may commission at the exact size. It may not decide
//! anything about the place itself. Everything that shapes a world belongs
//! in the draft, where it can be checked.

use std::collections::BTreeMap;

use infinite_manifest::plan::{Need, Plan};
use infinite_manifest::{
    Asset, AssetKind, Environment, MaterialRef, MeshRef, Placement, Portal, Prefab, PreviewPolicy,
    Sky, Spawn, StructuredId, WorldManifest, WorldMeta, THREAD_VERSION,
};

/// How a need was satisfied — reported, because "where did this come from?"
/// should never be a mystery.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Source {
    /// Found on the shelf at the right size.
    Stock,
    /// Found close enough, used at a uniform scale (never squashed).
    Scaled,
    /// Nothing fit: made to measure, and published back to the store.
    Commissioned,
    /// Nothing fit and nothing could be made — the need is unmet.
    Unmet,
}

pub struct Bound {
    pub source: Source,
    pub design: String,
    pub title: String,
    pub url: String,
    /// Uniform scale to apply (1.0 unless `Scaled`).
    pub scale: f32,
    pub note: String,
}

pub struct Options {
    pub quarry: String,
    /// Publish commissions back to the store (the flywheel).
    pub publish: bool,
    pub verbose: bool,
    /// Presence relay for the emitted world. Empty = solo — which is still
    /// **written down**: a world with no `presence` block reads as an author
    /// who forgot, and every consumer then has to decide what silence meant.
    pub relay: String,
    /// Build without a store: make every model here, write it beside the world,
    /// and reference it by a relative path. `Some(dir)` means **no network at
    /// all** — not "try the Quarry and fall back", because a build that
    /// sometimes reaches the network is not a reproducible build.
    pub local: Option<std::path::PathBuf>,
}

/// Bind every need, then emit the world.
pub fn build(plan: &Plan, opts: &Options) -> Result<(WorldManifest, Vec<(Need, Bound)>), String> {
    let catalog = load_catalog()?;
    let mut bound: Vec<(Need, Bound)> = Vec::new();
    // One design per (kind, rounded size, style): a place with sixteen
    // identical columns asks the store once and reuses the answer — which is
    // also what stops the shelf filling with near-duplicates.
    let mut memo: BTreeMap<String, Bound> = BTreeMap::new();

    for need in &plan.needs {
        let key = format!(
            "{}|{:.2}|{:.2}|{:.2}|{}",
            need.kind,
            quantize(need.w),
            quantize(need.h),
            quantize(need.d),
            if need.style.is_empty() {
                &plan.palette.style
            } else {
                &need.style
            }
        );
        if let Some(b) = memo.get(&key) {
            bound.push((need.clone(), b.clone()));
            continue;
        }
        let b = bind_one(need, plan, &catalog, opts);
        memo.insert(key, b.clone());
        bound.push((need.clone(), b));
    }
    let manifest = emit(plan, &bound, &opts.relay)?;
    Ok((manifest, bound))
}

impl Clone for Bound {
    fn clone(&self) -> Self {
        Bound {
            source: self.source,
            design: self.design.clone(),
            title: self.title.clone(),
            url: self.url.clone(),
            scale: self.scale,
            note: self.note.clone(),
        }
    }
}

/// Round a wanted dimension to 5 cm. Two needs that differ by a millimetre
/// are the same need; treating them otherwise is how a store silts up.
fn quantize(v: f32) -> f32 {
    (v / 0.05).round() * 0.05
}

/// One need: shop, then commission.
fn bind_one(need: &Need, plan: &Plan, catalog: &[CatalogEntry], opts: &Options) -> Bound {
    let style = if need.style.is_empty() {
        plan.palette.style.clone()
    } else {
        need.style.clone()
    };
    // 0. Building locally: there is no shelf to ask.
    if opts.local.is_some() {
        return match commission(need, plan, catalog, opts, &style) {
            Ok(b) => b,
            Err(e) => Bound {
                source: Source::Unmet,
                design: String::new(),
                title: String::new(),
                url: String::new(),
                scale: 1.0,
                note: e,
            },
        };
    }
    // 1. Ask the shelf.
    match search(&opts.quarry, need, &style) {
        Ok(Some((hit, fit))) => {
            let scale = if fit == "scale" {
                let have = hit["facts"]["size"][1].as_f64().unwrap_or(0.0) as f32;
                if need.h > 0.0 && have > 0.0 {
                    need.h / have
                } else {
                    1.0
                }
            } else {
                1.0
            };
            return Bound {
                source: if fit == "scale" {
                    Source::Scaled
                } else {
                    Source::Stock
                },
                design: hit["design"].as_str().unwrap_or_default().to_string(),
                title: hit["title"].as_str().unwrap_or_default().to_string(),
                url: format!(
                    "{}{}",
                    opts.quarry.trim_end_matches('/'),
                    hit["artifact"]["url"].as_str().unwrap_or_default()
                ),
                scale,
                note: if fit == "scale" {
                    format!("scaled ×{scale:.2}")
                } else {
                    String::new()
                },
            };
        }
        Ok(None) => {}
        Err(e) if opts.verbose => eprintln!("  (store unreachable: {e})"),
        Err(_) => {}
    }
    // 2. Nothing fit — commission it, at exactly the wanted size.
    if !need.commission {
        return Bound {
            source: Source::Unmet,
            design: String::new(),
            title: String::new(),
            url: String::new(),
            scale: 1.0,
            note: "nothing in stock and commissioning was declined".into(),
        };
    }
    match commission(need, plan, catalog, opts, &style) {
        Ok(b) => b,
        Err(e) => Bound {
            source: Source::Unmet,
            design: String::new(),
            title: String::new(),
            url: String::new(),
            scale: 1.0,
            note: e,
        },
    }
}

/// Ask the Quarry for something that fits. Returns the best hit, unless the
/// best is a poor fit — in which case the honest answer is "nothing".
fn search(
    quarry: &str,
    need: &Need,
    style: &str,
) -> Result<Option<(serde_json::Value, String)>, String> {
    let url = format!(
        "{}/models/search?kind={}&w={}&h={}&d={}&tol={}&style={}&limit=3",
        quarry.trim_end_matches('/'),
        urlencode(&need.kind),
        need.w,
        need.h,
        need.d,
        need.tol,
        urlencode(style)
    );
    let body = crate::http_get(&url)?;
    let parsed: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("bad search response: {e}"))?;
    let empty = Vec::new();
    let models = parsed["models"].as_array().unwrap_or(&empty);
    for m in models {
        let fit = m["fit"].as_str().unwrap_or("poor").to_string();
        if fit == "exact" || fit == "scale" {
            return Ok(Some((m.clone(), fit)));
        }
    }
    Ok(None)
}

struct CatalogEntry {
    kind: String,
    export: String,
    args: Vec<String>,
    material: String,
}

/// What the modeling library can make, straight from its own `catalog`.
fn load_catalog() -> Result<Vec<CatalogEntry>, String> {
    let lib = chisel::weft_model::standard_library();
    let raw = chisel::weft_model::eval_export(&lib, "catalog", &[])?;
    let arr = raw.as_array().ok_or("catalog is not a list")?;
    Ok(arr
        .iter()
        .map(|e| CatalogEntry {
            kind: e["kind"].as_str().unwrap_or_default().to_string(),
            export: e["export"].as_str().unwrap_or_default().to_string(),
            args: e["args"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            material: e["material"].as_str().unwrap_or_default().to_string(),
        })
        .collect())
}

/// Make one to measure, and (by default) put it on the shelf so the next
/// place finds it in stock. That is the whole flywheel: the first temple
/// commissions a dozen pieces and thereby stocks them.
fn commission(
    need: &Need,
    plan: &Plan,
    catalog: &[CatalogEntry],
    opts: &Options,
    style: &str,
) -> Result<Bound, String> {
    let entry = catalog
        .iter()
        .find(|c| c.kind == need.kind)
        .ok_or_else(|| format!("no supplier makes a '{}'", need.kind))?;
    let args: Vec<serde_json::Value> = entry
        .args
        .iter()
        .map(|a| serde_json::json!(eval_arg(a, need)))
        .collect();
    let material = match entry.material.as_str() {
        "stone" if !plan.palette.stone.is_empty() => plan.palette.stone.clone(),
        "wood" if !plan.palette.wood.is_empty() => plan.palette.wood.clone(),
        "metal" if !plan.palette.metal.is_empty() => plan.palette.metal.clone(),
        other => other.to_string(),
    };
    let title = format!(
        "{} {}",
        title_case(&need.kind),
        args.iter()
            .filter_map(|a| a.as_f64())
            .map(|f| format!("{f:.2}").trim_end_matches(['0', '.']).to_string())
            .collect::<Vec<_>>()
            .join(" × ")
    );
    let mut tags = need.tags.clone();
    tags.push(need.kind.clone());
    if !style.is_empty() {
        tags.push(style.to_string());
    }
    let submission = serde_json::json!({
        "title": title.trim(),
        "description": format!("Commissioned for '{}'.", plan.name),
        "kind": need.kind,
        "style": style,
        "tags": tags,
        "package": "weft-model",
        "export": entry.export,
        "args": args,
        "material": material,
        "origin": "commissioned",
    });
    if let Some(dir) = &opts.local {
        // Make it here and put it beside the world. The name comes from the
        // recipe, not from a hash of the bytes: the same brief must produce the
        // same filenames on every machine and in every release, so a rebuild is
        // a diff of geometry rather than a diff of names.
        let lib = chisel::weft_model::standard_library();
        let mat = (!material.is_empty()).then_some(material.as_str());
        let model = chisel::weft_model::eval_model_or_part(&lib, &entry.export, &args, mat)?;
        let built = chisel::model::build(&model)?;
        let glb = chisel::model::export_glb(&built)?;
        let file = local_name(&need.kind, style, &args, &material);
        let models = dir.join("models");
        std::fs::create_dir_all(&models)
            .map_err(|e| format!("cannot make {}: {e}", models.display()))?;
        let path = models.join(&file);
        std::fs::write(&path, &glb)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        return Ok(Bound {
            source: Source::Commissioned,
            design: file.trim_end_matches(".glb").to_string(),
            title,
            url: format!("models/{file}"),
            scale: 1.0,
            note: format!("made here — {} KB", glb.len() / 1024),
        });
    }
    if !opts.publish {
        // Dry run: prove it can be made, but leave the shelf alone.
        let lib = chisel::weft_model::standard_library();
        let mat = (!material.is_empty()).then_some(material.as_str());
        let model = chisel::weft_model::eval_model_or_part(&lib, &entry.export, &args, mat)?;
        chisel::model::build(&model)?;
        return Ok(Bound {
            source: Source::Commissioned,
            design: String::new(),
            title,
            url: String::new(),
            scale: 1.0,
            note: "made, not published (--publish to stock it)".into(),
        });
    }
    let body = crate::post_json(
        &format!("{}/publish", opts.quarry.trim_end_matches('/')),
        &submission,
    )?;
    let v: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("publish: {e} ({body})"))?;
    let design = v["design"].as_str().unwrap_or_default().to_string();
    if design.is_empty() {
        return Err(format!("the store refused the commission: {body}"));
    }
    Ok(Bound {
        source: Source::Commissioned,
        design: design.clone(),
        title,
        url: format!("{}/models/{design}.glb", opts.quarry.trim_end_matches('/')),
        scale: 1.0,
        note: "made to measure and stocked".into(),
    })
}

/// A local model's filename: readable, deterministic, and unique to the
/// recipe. `column-classical-marble-5.40x0.92.glb` tells you what it is at a
/// glance, which a content hash never does — and an author who opens the
/// folder is the whole point of building without a store.
fn local_name(kind: &str, style: &str, args: &[serde_json::Value], material: &str) -> String {
    let dims: Vec<String> =
        args.iter().filter_map(|a| a.as_f64()).map(|f| format!("{f:.2}")).collect();
    let mut name = [kind, style, material]
        .iter()
        .filter(|p| !p.is_empty())
        .map(|p| p.to_lowercase())
        .collect::<Vec<_>>()
        .join("-");
    if !dims.is_empty() {
        name.push('-');
        name.push_str(&dims.join("x"));
    }
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c } else { '-' })
        .collect();
    format!("{cleaned}.glb")
}

/// A catalog argument: `h`, `w`, `d`, a number, or one of those scaled
/// (`w*0.5`). Small on purpose — a supplier declares how to be asked, not a
/// program.
fn eval_arg(expr: &str, need: &Need) -> f32 {
    let (base, factor) = match expr.split_once('*') {
        Some((b, f)) => (b.trim(), f.trim().parse::<f32>().unwrap_or(1.0)),
        None => (expr.trim(), 1.0),
    };
    let v = match base {
        "h" => need.h,
        "w" => need.w,
        "d" => need.d,
        lit => lit.parse::<f32>().unwrap_or(0.0),
    };
    v * factor
}

fn title_case(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// Turn the plan plus its bindings into a walkable world.
fn emit(plan: &Plan, bound: &[(Need, Bound)], relay: &str) -> Result<WorldManifest, String> {
    let mut m = WorldManifest {
        thread: THREAD_VERSION.to_string(),
        world: WorldMeta {
            id: slug(&plan.name),
            title: plan.name.clone(),
            description: plan.description.clone(),
            author: None,
            codex: vec![],
            license: None,
            extra: Default::default(),
        },
        environment: Environment {
            sky: sky_for(&plan.palette.sky),
            ..Default::default()
        },
        spawns: Vec::new(),
        assets: Vec::new(),
        prefabs: Vec::new(),
        placements: Vec::new(),
        portals: Vec::new(),
        behaviors: Vec::new(),
        styles: Vec::new(),
        // Solo unless told otherwise, and said out loud either way. A place
        // built from a brief is usually meant to have people in it.
        presence: Some(infinite_manifest::Presence {
            mode: Some(if relay.is_empty() {
                "solo".into()
            } else {
                "relay".into()
            }),
            relays: if relay.is_empty() {
                Vec::new()
            } else {
                vec![relay.to_string()]
            },
            // Voice rides the relay when there is one: proximity audio is how a
            // room full of people sounds like a room.
            voice: !relay.is_empty(),
            ..Default::default()
        }),
        extra: Default::default(),
    };
    let mut next_prefab = 60_930_001u32;
    let mut next_asset = 0usize;
    let mut by_design: BTreeMap<String, StructuredId> = BTreeMap::new();

    // Built geometry first: ground, walls, copings — drawn from primitives,
    // one prefab per (primitive, material) pair so a hundred wall segments
    // are one upload and one instanced draw.
    let mut built_prefabs: BTreeMap<String, StructuredId> = BTreeMap::new();
    for (i, b) in plan.builds.iter().enumerate() {
        let builtin = match b.shape.as_str() {
            "disc" | "cylinder" => "cylinder",
            "slab" => "plane",
            _ => "cube",
        };
        let color = palette_color(&b.material, &plan.palette);
        let key = format!("{builtin}|{}|{color:?}", b.material);
        let prefab = *built_prefabs.entry(key).or_insert_with(|| {
            let id = StructuredId(next_prefab);
            next_prefab += 1;
            m.prefabs.push(Prefab {
                id,
                mesh: MeshRef {
                    asset: None,
                    builtin: Some(builtin.into()),
                    shape: None,
                    resolution: None,
                },
                material: Some(MaterialRef {
                    base_color: color,
                    roughness: 0.9,
                    texture: surface_for(&b.material, color),
                    ..Default::default()
                }),
                extra: Default::default(),
            });
            id
        });
        let scale = match b.shape.as_str() {
            "disc" => [b.r * 2.0, b.h.max(0.05), b.r * 2.0],
            "cylinder" => [b.r * 2.0, b.h.max(0.05), b.r * 2.0],
            "slab" => [b.w.max(0.5), 1.0, b.d.max(0.5)],
            _ => [b.w.max(0.05), b.h.max(0.05), b.d.max(0.05)],
        };
        m.placements.push(place(
            prefab,
            if b.name.is_empty() {
                format!("built-{i}")
            } else {
                format!("{}-{i}", b.name)
            },
            b.at,
            b.yaw,
            scale,
            (!b.solid).then_some(false),
        ));
    }

    // Then everything the plan asked for.
    for (i, (need, b)) in bound.iter().enumerate() {
        if b.source == Source::Unmet || b.url.is_empty() {
            continue;
        }
        let prefab = *by_design.entry(b.design.clone()).or_insert_with(|| {
            let asset_id = format!("model-{next_asset}");
            next_asset += 1;
            m.assets.push(Asset {
                id: asset_id.clone(),
                uri: b.url.clone(),
                kind: AssetKind::Gltf,
            });
            let id = StructuredId(next_prefab);
            next_prefab += 1;
            m.prefabs.push(Prefab {
                id,
                mesh: MeshRef {
                    asset: Some(asset_id),
                    builtin: None,
                    shape: None,
                    resolution: None,
                },
                material: None, // the glTF brings its own, PBR-complete
                extra: Default::default(),
            });
            id
        });
        let s = b.scale;
        m.placements.push(place(
            prefab,
            if need.name.is_empty() {
                format!("{}-{i}", need.kind)
            } else {
                format!("{}-{i}", need.name)
            },
            need.at,
            need.yaw,
            [s, s, s],
            (!need.solid).then_some(false),
        ));
    }

    // Lights, with a post under them when the plan wants a fixture.
    if !plan.lights.is_empty() {
        let lamp_pf = StructuredId(next_prefab);
        next_prefab += 1;
        m.prefabs.push(Prefab {
            id: lamp_pf,
            mesh: MeshRef {
                asset: None,
                builtin: Some("cube".into()),
                shape: None,
                resolution: None,
            },
            material: Some(MaterialRef {
                base_color: [1.0, 0.86, 0.6, 1.0],
                emissive: 0.7,
                ..Default::default()
            }),
            extra: Default::default(),
        });
        let post_pf = StructuredId(next_prefab);
        next_prefab += 1;
        m.prefabs.push(Prefab {
            id: post_pf,
            mesh: MeshRef {
                asset: None,
                builtin: Some("cylinder".into()),
                shape: None,
                resolution: None,
            },
            material: Some(MaterialRef {
                base_color: [0.2, 0.19, 0.22, 1.0],
                metallic: 0.6,
                roughness: 0.5,
                ..Default::default()
            }),
            extra: Default::default(),
        });
        for (i, l) in plan.lights.iter().enumerate() {
            if l.fixture {
                // A standard from the ground to the light, and no further. It
                // was twice that tall once — 5 m black posts, at eye level, in
                // the entrance view. A lamp is furniture; if you notice it
                // before you notice the room, it is wrong.
                let h = l.at[1].max(0.4);
                m.placements.push(place(
                    post_pf,
                    format!("lamp-post-{i}"),
                    [l.at[0], h / 2.0, l.at[2]],
                    0.0,
                    [0.09, h, 0.09],
                    None,
                ));
            }
            let mut head = place(
                lamp_pf,
                format!("lamp-{i}"),
                l.at,
                0.0,
                [0.22, 0.22, 0.22],
                Some(false),
            );
            head.light = Some(infinite_manifest::LightEmitter {
                color: [
                    1.0,
                    0.78 * l.warm.clamp(0.2, 1.5),
                    0.45 * l.warm.clamp(0.2, 1.5),
                ],
                intensity: l.intensity,
                range: l.range,
            });
            m.placements.push(head);
        }
    }

    for (i, s) in plan.spawns.iter().enumerate() {
        m.spawns.push(Spawn {
            name: if s.name.is_empty() {
                format!("spawn-{i}")
            } else {
                s.name.clone()
            },
            position: s.at,
            yaw: s.yaw.to_radians(),
        });
    }
    for (i, v) in plan.veils.iter().enumerate() {
        m.portals.push(Portal {
            id: format!("veil-{i}"),
            position: v.at,
            rotation: yaw_quat(v.yaw),
            scale: [2.0, 3.0, 0.2],
            to: if v.to.is_empty() {
                "thread://pixygon.io#entry".into()
            } else {
                v.to.clone()
            },
            label: if v.label.is_empty() {
                "Onward".into()
            } else {
                v.label.clone()
            },
            preview: PreviewPolicy::Live,
            extra: Default::default(),
        });
    }
    if !plan.signs.is_empty() {
        let quad = StructuredId(next_prefab);
        next_prefab += 1;
        m.prefabs.push(Prefab {
            id: quad,
            mesh: MeshRef {
                asset: None,
                builtin: Some("quad".into()),
                shape: None,
                resolution: None,
            },
            material: Some(MaterialRef {
                base_color: [0.96, 0.93, 0.86, 1.0],
                roughness: 0.9,
                ..Default::default()
            }),
            extra: Default::default(),
        });
        for s in &plan.signs {
            let mut p = place(
                quad,
                if s.text.is_empty() {
                    "sign".into()
                } else {
                    s.text.clone()
                },
                s.at,
                s.yaw,
                [s.w, s.h, 1.0],
                Some(false),
            );
            // Type as large as the panel will hold. This was capped at 0.06
            // — a legible size for a wall of prose, and utterly invisible for
            // a two-word gate title, which is what signs mostly are.
            let lines = s.text.lines().count().max(1) as f32;
            p.text = Some(infinite_manifest::TextPanel {
                content: s.text.clone(),
                size: 1.0 / (1.4 * lines + 1.6),
                color: [0.13, 0.11, 0.09],
                background: [0.96, 0.93, 0.86],
                links: vec![],
            });
            m.placements.push(p);
        }
    }
    Ok(m)
}

fn place(
    prefab: StructuredId,
    name: String,
    position: [f32; 3],
    yaw: f32,
    scale: [f32; 3],
    solid: Option<bool>,
) -> Placement {
    Placement {
        prefab,
        name,
        kind: None,
        class: vec![],
        position,
        rotation: yaw_quat(yaw),
        scale,
        codex: None,
        animate: None,
        solid,
        light: None,
        text: None,
        behavior: None,
        interaction: None,
        data: serde_json::Value::Null,
        children: vec![],
        extra: Default::default(),
    }
}

fn yaw_quat(deg: f32) -> [f32; 4] {
    let h = deg.to_radians() / 2.0;
    [0.0, h.sin(), 0.0, h.cos()]
}

/// A palette slot (`stone`, `wood`, `metal`, `accent`) or a literal `"r g b"`
/// → a colour. The palette names materials by petname (`marble`,
/// `sandstone`), which also tints the built stone so a drafted place reads as
/// one place rather than a kit.
fn palette_color(material: &str, palette: &infinite_manifest::plan::Palette) -> [f32; 4] {
    // `ground` is the palette's stone, taken down to earth: open ground next
    // to dressed stone at the same value gives a white void with edges in it,
    // which is what the first drafted museum looked like.
    if let Some(k) = match material {
        "ground" => Some(0.46),
        // Floor a touch below wall: without it a marble hall is one flat
        // value from your feet to the sky, and reads as fog with edges.
        "floor" => Some(0.9),
        _ => None,
    } {
        let s = palette_color("stone", palette);
        return [s[0] * k, s[1] * k * 0.99, s[2] * k * 0.96, 1.0];
    }
    let name = match material {
        "stone" => palette.stone.as_str(),
        "wood" => palette.wood.as_str(),
        "metal" => palette.metal.as_str(),
        other => other,
    };
    let literal: Vec<f32> = name
        .split_whitespace()
        .filter_map(|t| t.parse().ok())
        .collect();
    if literal.len() >= 3 {
        return [literal[0], literal[1], literal[2], 1.0];
    }
    match name {
        "marble" => [0.86, 0.85, 0.82, 1.0],
        "granite" => [0.47, 0.47, 0.50, 1.0],
        "sandstone" => [0.78, 0.69, 0.52, 1.0],
        "plaster" => [0.85, 0.83, 0.79, 1.0],
        "terracotta" => [0.62, 0.38, 0.24, 1.0],
        "wood" | "oak" => [0.45, 0.31, 0.18, 1.0],
        "iron" => [0.26, 0.25, 0.28, 1.0],
        "brass" | "accent" => [0.78, 0.63, 0.28, 1.0],
        _ => [0.55, 0.53, 0.50, 1.0],
    }
}

/// A procedural surface for built stone. Flat colour is what made the first
/// drafted museum read as fog with edges in it: every plane the same value,
/// nothing for the eye to measure the room by. These are the same recipes the
/// hand-written hall used, chosen by *role* rather than by name, and tinted
/// from the palette so the place still keeps its own colour.
fn surface_for(
    material: &str,
    color: [f32; 4],
) -> Option<infinite_manifest::texture::TextureRecipe> {
    use infinite_manifest::texture::TextureRecipe;
    let shade = |k: f32| [color[0] * k, color[1] * k, color[2] * k];
    // `triplanar` is not a nicety here: the builtin primitives carry planar
    // XZ uvs, which is right for a floor and nonsense on a wall — the first
    // textured draft grew vertical stripes, a brick bond smeared up a face
    // that had no v to speak of. World-space sampling (uv repeats per metre)
    // both fixes that and makes a ring of rotated segments seamless.
    let (kind, scale, height, ao, triplanar, colors) = match material {
        // dressed masonry: courses you can read the scale of a wall from
        "stone" | "" => (
            "bricks",
            4.0,
            0.55,
            0.55,
            0.55,
            vec![shade(0.86), shade(1.0), shade(0.93)],
        ),
        // a floor is one surface; it wants figure, not joints
        "floor" => (
            "veins",
            3.0,
            0.10,
            0.15,
            0.5,
            vec![shade(1.06), shade(0.86), shade(0.98)],
        ),
        "ground" => (
            "fbm",
            4.0,
            0.35,
            0.45,
            0.12,
            vec![shade(0.8), shade(1.0), shade(0.88)],
        ),
        "wood" => ("wood", 4.0, 0.3, 0.3, 0.5, vec![shade(0.8), shade(1.0)]),
        _ => return None,
    };
    Some(TextureRecipe {
        kind: kind.into(),
        scale,
        octaves: 4,
        seed: 7,
        colors,
        roughness: [0.95, 0.8],
        smoothness: None,
        metallic: [0.0, 0.0],
        height,
        ao,
        size: 256,
        over: None,
        mix: 0.0,
        mask_scale: 4.0,
        mask_seed: 0,
        triplanar,
    })
}

fn sky_for(name: &str) -> Option<Sky> {
    let (z, h, s) = match name {
        "dawn" => ([0.10, 0.13, 0.26], [0.86, 0.52, 0.32], [0.30, 0.35, 0.25]),
        "day" => ([0.24, 0.45, 0.78], [0.72, 0.82, 0.92], [0.35, 0.80, 0.30]),
        "dusk" => ([0.07, 0.09, 0.20], [0.80, 0.42, 0.24], [0.30, 0.35, 0.25]),
        "night" => ([0.02, 0.03, 0.08], [0.10, 0.12, 0.22], [0.20, 0.60, 0.30]),
        "" => return None,
        _ => ([0.10, 0.12, 0.22], [0.55, 0.35, 0.28], [0.35, 0.45, 0.30]),
    };
    Some(Sky {
        zenith: z,
        horizon: h,
        sun_dir: Some(s),
    })
}

fn slug(name: &str) -> String {
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

fn urlencode(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => c.to_string(),
            ' ' => "+".to_string(),
            other => format!("%{:02X}", other as u32),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The filename IS the reproducibility guarantee for a store-less build: a
    /// rebuild on another machine has to overwrite the same files, or the world
    /// accumulates near-duplicate models and its asset list churns. Derived
    /// from the recipe, so it is stable across releases of the mesher — a
    /// changed model is a changed *file*, never a changed name.
    #[test]
    fn a_local_model_is_named_by_its_recipe() {
        let args = vec![serde_json::json!(5.2), serde_json::json!(0.44)];
        let name = local_name("column", "classical", &args, "marble");
        assert_eq!(name, "column-classical-marble-5.20x0.44.glb");
        assert_eq!(local_name("column", "classical", &args, "marble"), name, "stable");

        // Every part of the recipe separates it from its neighbours.
        assert_ne!(name, local_name("column", "rustic", &args, "marble"));
        assert_ne!(name, local_name("column", "classical", &args, "granite"));
        assert_ne!(
            name,
            local_name("column", "classical", &vec![serde_json::json!(5.3), serde_json::json!(0.44)], "marble")
        );

        // Nothing a palette can hold may escape into the path.
        let hostile = local_name("../etc/pass wd", "a/b", &[], "c:d");
        assert!(!hostile.contains('/') && !hostile.contains(' '), "{hostile}");
        assert!(hostile.ends_with(".glb"));
    }
}
