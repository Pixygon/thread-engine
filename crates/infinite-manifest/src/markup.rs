//! Thread markup — the HTML/CSS authoring surface for worlds.
//!
//! A `.thread` file reads like HTML + CSS and **compiles to a World Manifest**
//! (the JSON "DOM" browsers fetch) — the same source→DOM split the web has. The
//! goal is the low floor: a handful of tags and a `<style>` block get you a
//! walkable place, no JSON by hand. Browsers accept `.thread` files directly
//! (see [`WorldManifest::from_text`]); serving markup at `.well-known/thread/`
//! is as valid as serving JSON — exactly like serving HTML.
//!
//! ```html
//! <world id="grove" title="The Grove" description="A quiet clearing"
//!        sky="0.1 0.1 0.2 / 0.5 0.3 0.2" rules="gathering">
//!   <spawn at="0 0 8" yaw="180"/>
//!   <tree class="harvestable" at="3 0 -2" scale="1 3 1"/>
//!   <cube at="0 0 -5" rot="0 45 0" codex="the-thread">
//!     <sphere at="0 1 0"/>
//!   </cube>
//!   <quad id="sign" at="0 1.4 -6" scale="5 2 0.2" url="https://example.com"/>
//!   <portal to="thread://market" at="0 0 -8" label="Market"/>
//! </world>
//! <style>
//!   tree { mesh: "trees/oak.glb" }
//!   .harvestable {
//!     interaction: "Chop";
//!     interaction-hits: 3;
//!     interaction-gives: 20100001 3;
//!     interaction-message: "You chopped the old oak.";
//!     interaction-despawns: true;
//!   }
//!   #sign { color: 0.2 0.25 0.35; emissive: 0.4 }
//! </style>
//! ```
//!
//! Grammar summary:
//! - `<world id title description sky year rules>` — the root. `sky` is a named
//!   preset (`dawn`/`day`/`dusk`/`night`/`void`) or explicit gradients
//!   `"zr zg zb / hr hg hb [/ sx sy sz]"` (the third segment aims the sun).
//!   `rules` opts into game mechanics (`survival gathering combat`), default none.
//! - `<spawn at name yaw>` — an arrival point (`yaw` in degrees). A world with
//!   no `<spawn>` gets a default one so every compiled world is enterable.
//! - Any builtin tag (`cube`/`sphere`/`cylinder`/`capsule`/`plane`/`quad`) or a
//!   tag given a `mesh:` style rule places an object. Attributes: `id`, `class`,
//!   `at`, `rot` (Euler degrees, applied yaw→pitch→roll), `scale`,
//!   `codex="slug"` (inspect opens its lore), `url="…"` (signboard → the web
//!   reader), and `data-*` (arbitrary per-object data, numbers auto-typed).
//!   Children nest — child transforms are relative to the parent.
//! - `<portal to at rot scale label>` — a veil to another world.
//! - **Structure vocabulary**: `<room at r h gates columns gate-width>` — a
//!   round walled colonnaded chamber (gates are azimuths in degrees, 180 =
//!   toward the default spawn; children are authored relative to the room's
//!   centre); `<wall from="x z" to="x z" h thick>` — a straight wall between
//!   two ground points. Architecture as elements, not hand-mathed cubes.
//! - `light="r g b [intensity] [range]"` (attr or style property) — the
//!   placement emits a point light; lamps are content, not configuration.
//! - `<style>` properties: `mesh`, `color` (named or `r g b [a]`), `metallic`,
//!   `roughness`, `emissive` (glow, blooms), `light`, room's `floor-color` /
//!   `accent-color`, `interaction` (the verb) with longhands
//!   `interaction-hits`, `interaction-gives` (`<item-id> [count]`),
//!   `interaction-message`, `interaction-effect`, `interaction-despawns`.

use std::collections::BTreeMap;

use crate::{
    Animate, Asset, AssetKind, Environment, Interaction, InteractionEffect, LightEmitter,
    MaterialRef, MeshRef, Placement, Portal, Prefab, Sky, Spawn, StructuredId, StyleRule,
    WorldManifest, WorldMeta, WorldRules,
};

/// The builtin-primitive mesh names a tag may resolve to directly (a bare
/// `<cube>` needs no style rule); any other mesh comes from a `mesh:` cascade rule.
const BUILTINS: &[&str] = &["cube", "sphere", "cylinder", "capsule", "plane", "quad"];

/// A parsed markup element: a tag, its attributes, and its children.
#[derive(Debug)]
struct Element {
    tag: String,
    attrs: BTreeMap<String, String>,
    children: Vec<Element>,
}

/// Compile Thread markup into a validated [`WorldManifest`].
pub fn compile(src: &str) -> Result<WorldManifest, String> {
    // Pull the <style> block out first (its body is CSS-ish, not markup).
    let (markup, style_body) = extract_style(src);
    let decls = parse_style(&style_body)?;
    // `interaction` is a *runtime* cascade property → it rides along in the
    // manifest's `styles`. Appearance (mesh/color/…) is resolved at compile time.
    let styles = interaction_styles(&decls)?;

    let roots = Parser::new(&markup).parse_all()?;
    let world_el = roots
        .iter()
        .find(|e| e.tag == "world")
        .ok_or("markup must contain a <world> root element")?;

    let mut manifest = WorldManifest {
        thread: crate::THREAD_VERSION.to_string(),
        world: WorldMeta {
            id: world_el
                .attrs
                .get("id")
                .cloned()
                .unwrap_or_else(|| "world".into()),
            title: world_el
                .attrs
                .get("title")
                .cloned()
                .unwrap_or_else(|| "Untitled".into()),
            description: world_el
                .attrs
                .get("description")
                .cloned()
                .unwrap_or_default(),
            author: None,
            codex: vec![],
            license: None,
            extra: Default::default(),
        },
        environment: Environment {
            sky: parse_sky(world_el.attrs.get("sky")),
            year: world_el.attrs.get("year").and_then(|y| y.parse().ok()),
            rules: parse_rules(world_el.attrs.get("rules"))?,
            ..Default::default()
        },
        spawns: vec![],
        assets: vec![],
        prefabs: vec![],
        placements: vec![],
        portals: vec![],
        behaviors: vec![],
        styles,
        presence: parse_presence(world_el.attrs.get("presence"))?,
        extra: Default::default(),
    };

    // Walk the world's children into placements/portals, synthesizing a prefab
    // (and, for a mesh URL, an asset entry) per unique mesh the cascade resolves.
    // A pre-pass collects <model> definitions — reusable composite parts that
    // <use> stamps into place (the markup's "make a thing once" primitive).
    let mut models: std::collections::HashMap<String, &Element> = Default::default();
    for child in &world_el.children {
        if child.tag == "model" {
            let name = child
                .attrs
                .get("name")
                .cloned()
                .ok_or("<model> needs a `name` to be <use>d by")?;
            models.insert(name, child);
        }
    }
    let mut looks = LookRegistry::new();
    for child in &world_el.children {
        if child.tag == "shape" {
            let name = child
                .attrs
                .get("name")
                .cloned()
                .ok_or("<shape> needs a `name` to be used by shape=\"…\"")?;
            let resolution = child.attrs.get("resolution").and_then(|v| v.parse().ok());
            let tree = parse_shape_children(&child.children)?;
            tree.validate()?;
            looks.shapes.insert(name, (tree, resolution));
        }
        if child.tag == "texture" {
            let name = child
                .attrs
                .get("name")
                .cloned()
                .ok_or("<texture> needs a `name` to be used by texture=\"…\"")?;
            let recipe = parse_texture_def(child, &looks.textures)?;
            recipe.validate()?;
            looks.textures.insert(name, recipe);
        }
    }
    for child in &world_el.children {
        if child.tag == "model" || child.tag == "shape" || child.tag == "texture" {
            continue; // definitions emit nothing themselves
        }
        emit(child, &mut manifest, &decls, &mut looks, &models, 0)?;
    }

    // Every compiled world is enterable: a world that authored no <spawn> gets
    // a default arrival point a few steps back from the origin, facing it.
    if manifest.spawns.is_empty() {
        manifest.spawns.push(Spawn {
            name: "entry".into(),
            position: [0.0, 0.0, 6.0],
            yaw: std::f32::consts::PI,
        });
    }

    manifest.validate().map_err(|e| e.to_string())?;
    Ok(manifest)
}

/// Turn one markup element into a placement (or portal/spawn), recursing into
/// children.
fn emit(
    el: &Element,
    manifest: &mut WorldManifest,
    decls: &[StyleDecl],
    looks: &mut LookRegistry,
    models: &std::collections::HashMap<String, &Element>,
    depth: usize,
) -> Result<(), String> {
    if depth > 16 {
        return Err("model expansion deeper than 16 — a <model> is <use>ing itself".into());
    }
    if el.tag == "spawn" {
        manifest.spawns.push(Spawn {
            name: el
                .attrs
                .get("name")
                .cloned()
                .unwrap_or_else(|| "entry".into()),
            position: vec3(el.attrs.get("at"))?,
            yaw: el
                .attrs
                .get("yaw")
                .and_then(|y| y.parse::<f32>().ok())
                .map(|deg| deg.to_radians())
                .unwrap_or(0.0),
        });
        return Ok(());
    }
    // Structure vocabulary — architecture as elements, not hand-mathed cubes.
    if el.tag == "room" {
        return emit_room(el, manifest, decls, looks, models, depth);
    }
    if el.tag == "wall" {
        return emit_wall(el, manifest, decls, looks);
    }
    if el.tag == "use" {
        return emit_use(el, manifest, decls, looks, models, depth);
    }
    if el.tag == "ring" {
        return emit_ring(el, manifest, decls, looks, models, depth);
    }
    if el.tag == "row" {
        return emit_row(el, manifest, decls, looks, models, depth);
    }
    if el.tag == "lamp" {
        return emit_lamp(el, manifest, decls, looks);
    }
    if el.tag == "model" {
        return Err("<model> definitions live at the top level of <world>".into());
    }
    if el.tag == "portal" {
        manifest.portals.push(Portal {
            id: el
                .attrs
                .get("id")
                .cloned()
                .unwrap_or_else(|| "portal".into()),
            position: vec3(el.attrs.get("at"))?,
            rotation: rot_attr(el)?,
            scale: scale_attr(el)?,
            to: el.attrs.get("to").cloned().ok_or("<portal> needs a `to`")?,
            label: el.attrs.get("label").cloned().unwrap_or_default(),
            preview: Default::default(),
            extra: Default::default(),
        });
        return Ok(());
    }

    // The element's mesh: a carved `shape="name"` wins; else a cascade `mesh:`
    // rule; else a builtin-named tag (`<cube>`). Anything else needs a rule.
    let mesh = if let Some(name) = el.attrs.get("shape") {
        if !looks.shapes.contains_key(name) {
            return Err(format!("shape=\"{name}\" — no such <shape> defined"));
        }
        format!("shape:{name}")
    } else {
        resolve_prop(el, decls, "mesh")
            .or_else(|| BUILTINS.iter().find(|t| **t == el.tag).map(|t| (*t).to_string()))
            .ok_or_else(|| {
                format!(
                    "no mesh for <{0}> — add a `{0} {{ mesh: \"…\" }}` style rule, or use a builtin tag",
                    el.tag
                )
            })?
    };
    // The rest of the appearance cascade — each property resolved independently.
    let texture = resolve_prop(el, decls, "texture");
    if let Some(t) = &texture {
        if !looks.textures.contains_key(t) {
            return Err(format!("texture=\"{t}\" — no such <texture> defined"));
        }
    }
    let look = Look {
        mesh,
        base_color: resolve_prop(el, decls, "color")
            .and_then(|c| parse_color(&c))
            .unwrap_or([1.0, 1.0, 1.0, 1.0]),
        metallic: resolve_prop(el, decls, "metallic")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.0),
        roughness: resolve_prop(el, decls, "roughness")
            .and_then(|v| v.parse().ok())
            .unwrap_or(1.0),
        emissive: resolve_prop(el, decls, "emissive")
            .map(|v| parse_emissive(&v))
            .unwrap_or(0.0),
        texture,
    };
    let prefab = looks.ensure(&look, manifest);

    let mut children = Vec::new();
    for c in &el.children {
        // A child placement is authored relative to its parent (the tree model).
        let before = manifest.placements.len();
        emit(c, manifest, decls, looks, models, depth + 1)?;
        // Move any placement `emit` pushed for the child under this one instead.
        children.extend(manifest.placements.drain(before..));
    }

    let behavior = weft_binding(el, manifest);
    manifest.placements.push(Placement {
        prefab,
        name: el.attrs.get("id").cloned().unwrap_or_default(),
        kind: Some(el.tag.clone()),
        class: el
            .attrs
            .get("class")
            .map(|c| c.split_whitespace().map(str::to_string).collect())
            .unwrap_or_default(),
        position: vec3(el.attrs.get("at"))?,
        rotation: rot_attr(el)?,
        scale: scale_attr(el)?,
        codex: el.attrs.get("codex").cloned(),
        text: el.attrs.get("text").map(|t| {
            let (content, links) = parse_text_links(&t.replace("\\n", "\n"));
            crate::TextPanel {
                content,
                size: el
                    .attrs
                    .get("text-size")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or_else(default_panel_size),
                color: default_panel_ink(),
                background: default_panel_paper(),
                links,
            }
        }),
        animate: parse_animate(resolve_prop(el, decls, "animate")),
        solid: el
            .attrs
            .get("solid")
            .map(|v| v != "false")
            .or_else(|| resolve_prop(el, decls, "solid").map(|v| v != "false")),
        light: el
            .attrs
            .get("light")
            .cloned()
            .or_else(|| resolve_prop(el, decls, "light"))
            .and_then(|v| parse_light(&v)),
        behavior,
        interaction: None,
        data: data_attrs(el),
        children,
        extra: Default::default(),
    });
    Ok(())
}

/// Inline hyperlinks in a `text` attribute — the markup form of the Thread's
/// `<a>`: `[[thread://host/path|phrase]]` renders `phrase` as a link to the
/// Locator; `[[thread://host/path]]` uses the Locator itself as the phrase.
/// Returns the cleaned content plus the extracted [`crate::TextLink`]s.
fn parse_text_links(raw: &str) -> (String, Vec<crate::TextLink>) {
    let mut content = String::with_capacity(raw.len());
    let mut links = Vec::new();
    let mut rest = raw;
    while let Some(open) = rest.find("[[") {
        content.push_str(&rest[..open]);
        let after = &rest[open + 2..];
        let Some(close) = after.find("]]") else {
            content.push_str(&rest[open..]);
            rest = "";
            break;
        };
        let inner = &after[..close];
        let (to, label) = match inner.split_once('|') {
            Some((to, label)) => (to.trim(), label.trim()),
            None => (inner.trim(), inner.trim()),
        };
        if !to.is_empty() && !label.is_empty() {
            links.push(crate::TextLink {
                text: label.to_string(),
                to: to.to_string(),
            });
            content.push_str(label);
        }
        rest = &after[close + 2..];
    }
    content.push_str(rest);
    (content, links)
}

/// The `light` attribute / style property → a [`LightEmitter`]. Forms:
/// `light="true"` (default warm lamplight), `light="r g b"`,
/// `light="r g b intensity"`, `light="r g b intensity range"`.
fn parse_light(v: &str) -> Option<LightEmitter> {
    if v.trim() == "true" || v.trim().is_empty() {
        return Some(LightEmitter {
            color: [1.0, 0.85, 0.6],
            intensity: 1.0,
            range: 10.0,
        });
    }
    let n: Vec<f32> = v
        .split_whitespace()
        .filter_map(|t| t.parse().ok())
        .collect();
    match n.as_slice() {
        [r, g, b] => Some(LightEmitter {
            color: [*r, *g, *b],
            intensity: 1.0,
            range: 10.0,
        }),
        [r, g, b, i] => Some(LightEmitter {
            color: [*r, *g, *b],
            intensity: *i,
            range: 10.0,
        }),
        [r, g, b, i, ra] => Some(LightEmitter {
            color: [*r, *g, *b],
            intensity: *i,
            range: *ra,
        }),
        _ => None,
    }
}

/// Minimal angular distance between two azimuths, degrees.
fn ang_dist(a: f32, b: f32) -> f32 {
    let mut d = (a - b).abs() % 360.0;
    if d > 180.0 {
        d = 360.0 - d;
    }
    d
}

/// Push a bare structural placement (no text/behavior/children).
#[allow(clippy::too_many_arguments)]
// ── The carving language: <shape> subtrees → shape::Shape ────────────────
//
// Inside a <shape name="…"> definition, elements are SDF nodes, not
// placements: primitives (<sphere> <box> <cylinder> <capsule> <cone>
// <torus>), combiners (<blend k> <cut> <union> <intersect>), and <lathe
// profile="r y, r y, …"> revolutions. The browser meshes the tree at load.

fn parse_shape_children(children: &[Element]) -> Result<crate::shape::Shape, String> {
    let mut parts = Vec::new();
    for c in children {
        parts.push(parse_shape_node(c)?);
    }
    match parts.len() {
        0 => Err("<shape> needs at least one part".into()),
        1 => Ok(parts.pop().unwrap()),
        _ => Ok(crate::shape::Shape::Group(crate::shape::Group {
            op: "union".into(),
            k: 0.25,
            at: [0.0; 3],
            rot: 0.0,
            parts,
        })),
    }
}

fn parse_shape_node(el: &Element) -> Result<crate::shape::Shape, String> {
    use crate::shape::{Group, Lathe, Prim, Shape};
    let f = |k: &str, d: f32| {
        el.attrs
            .get(k)
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(d)
    };
    let at = if el.attrs.contains_key("at") {
        vec3(el.attrs.get("at"))?
    } else {
        [0.0; 3]
    };
    match el.tag.as_str() {
        "sphere" | "cylinder" | "capsule" | "cone" | "torus" | "box" => {
            let size = if el.tag == "box" {
                Some(vec3(Some(
                    el.attrs.get("size").ok_or("shape <box> needs a `size`")?,
                ))?)
            } else {
                None
            };
            Ok(Shape::Prim(Prim {
                prim: el.tag.clone(),
                at,
                rot: f("rot", 0.0),
                r: f("r", 0.5),
                size,
                h: f("h", 1.0),
                r2: f("r2", if el.tag == "torus" { 0.15 } else { 0.0 }),
                rounded: f("rounded", 0.0),
                axis: el.attrs.get("axis").cloned().unwrap_or_else(|| "y".into()),
            }))
        }
        "blend" | "cut" | "union" | "intersect" => {
            let mut parts = Vec::new();
            for c in &el.children {
                parts.push(parse_shape_node(c)?);
            }
            if parts.is_empty() {
                return Err(format!("<{}> needs children", el.tag));
            }
            Ok(Shape::Group(Group {
                op: el.tag.clone(),
                k: f("k", 0.25),
                at,
                rot: f("rot", 0.0),
                parts,
            }))
        }
        "lathe" => {
            let profile = el.attrs.get("profile").ok_or("<lathe> needs a `profile`")?;
            let pts: Result<Vec<[f32; 2]>, String> = profile
                .split(',')
                .map(|pair| {
                    let mut it = pair
                        .split_whitespace()
                        .filter_map(|t| t.parse::<f32>().ok());
                    match (it.next(), it.next()) {
                        (Some(r), Some(y)) => Ok([r, y]),
                        _ => Err(format!("lathe profile point '{pair}' — want `r y`")),
                    }
                })
                .collect();
            Ok(Shape::Lathe(Lathe { lathe: pts?, at }))
        }
        other => Err(format!(
            "'{other}' is not a shape word — prims {:?}, ops {:?}, or lathe",
            crate::shape::PRIMS,
            crate::shape::OPS
        )),
    }
}

/// `<texture name kind [scale octaves seed size] [colors="r g b | r g b …"]
/// [rough="a b"] [metal="a b"] [height] [ao]/>` — one tag, a whole PBR
/// material: the browser bakes albedo + normal + occlusion-roughness-metallic
/// from the recipe. See texture.rs for the semantics.
fn parse_texture_def(
    el: &Element,
    defined: &BTreeMap<String, crate::texture::TextureRecipe>,
) -> Result<crate::texture::TextureRecipe, String> {
    let f = |k: &str, d: f32| {
        el.attrs
            .get(k)
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(d)
    };
    let pair = |k: &str, d: [f32; 2]| -> Result<[f32; 2], String> {
        match el.attrs.get(k) {
            None => Ok(d),
            Some(v) => {
                let nums: Vec<f32> = v
                    .split_whitespace()
                    .filter_map(|t| t.parse().ok())
                    .collect();
                match nums.as_slice() {
                    [a] => Ok([*a, *a]),
                    [a, b] => Ok([*a, *b]),
                    _ => Err(format!("<texture> {k}=\"{v}\" — want one or two numbers")),
                }
            }
        }
    };
    let colors = match el.attrs.get("colors") {
        None => vec![[0.6, 0.6, 0.6]],
        Some(v) => v
            .split('|')
            .map(|c| {
                let nums: Vec<f32> = c
                    .split_whitespace()
                    .filter_map(|t| t.parse().ok())
                    .collect();
                match nums.as_slice() {
                    [r, g, b] => Ok([*r, *g, *b]),
                    _ => Err(format!("<texture> color '{c}' — want `r g b`")),
                }
            })
            .collect::<Result<Vec<_>, _>>()?,
    };
    // `over="name"` layers a previously defined texture on top (moss over
    // granite) — defined earlier in the document, one level deep.
    let over = match el.attrs.get("over") {
        None => None,
        Some(name) => Some(Box::new(defined.get(name).cloned().ok_or_else(|| {
            format!("over=\"{name}\" — no such <texture> defined yet (define it first)")
        })?)),
    };
    Ok(crate::texture::TextureRecipe {
        kind: el
            .attrs
            .get("kind")
            .cloned()
            .ok_or("<texture> needs a `kind`")?,
        scale: f("scale", 4.0),
        octaves: f("octaves", 4.0) as u32,
        seed: f("seed", 0.0) as u32,
        colors,
        roughness: pair("rough", [0.9, 0.9])?,
        smoothness: match el.attrs.get("smooth") {
            Some(_) => Some(pair("smooth", [0.1, 0.1])?),
            None => None,
        },
        metallic: pair("metal", [0.0, 0.0])?,
        height: f("height", 0.0),
        ao: f("ao", 0.0),
        size: f("size", 256.0) as u32,
        over,
        mix: f("mix", 0.5),
        mask_scale: f("mask-scale", 3.0),
        mask_seed: f("mask-seed", 0.0) as u32,
        triplanar: f("triplanar", 0.0),
    })
}

// ── The stamping engine: compile-time instancing ─────────────────────────
//
// <use>, <ring> and <row> all work the same way: emit a template's children
// into the manifest, then TRANSFORM everything that was just produced
// (translate + yaw + uniform scale). The manifest stays plain placements —
// no runtime grouping, nothing new for browsers to learn — and the trig
// lives here once instead of in every author's head.

/// Emit `children` and transform the produced placements + portals by
/// (`offset`, `yaw_deg`, uniform `scale`).
fn stamp_children(
    children: &[Element],
    manifest: &mut WorldManifest,
    decls: &[StyleDecl],
    looks: &mut LookRegistry,
    models: &std::collections::HashMap<String, &Element>,
    depth: usize,
    offset: [f32; 3],
    yaw_deg: f32,
    scale: f32,
) -> Result<(), String> {
    let p_before = manifest.placements.len();
    let g_before = manifest.portals.len();
    for c in children {
        emit(c, manifest, decls, looks, models, depth + 1)?;
    }
    let q = euler_deg_to_quat(0.0, yaw_deg, 0.0);
    let apply = |pos: &mut [f32; 3], rot: &mut [f32; 4], scl: Option<&mut [f32; 3]>| {
        let scaled = [pos[0] * scale, pos[1] * scale, pos[2] * scale];
        let turned = quat_rotate(q, scaled);
        *pos = [
            turned[0] + offset[0],
            turned[1] + offset[1],
            turned[2] + offset[2],
        ];
        *rot = quat_mul(q, *rot);
        if let Some(s) = scl {
            *s = [s[0] * scale, s[1] * scale, s[2] * scale];
        }
    };
    for p in &mut manifest.placements[p_before..] {
        apply(&mut p.position, &mut p.rotation, Some(&mut p.scale));
        // Children of a stamped placement are parent-relative — untouched.
    }
    for portal in &mut manifest.portals[g_before..] {
        apply(
            &mut portal.position,
            &mut portal.rotation,
            Some(&mut portal.scale),
        );
    }
    Ok(())
}

/// `<use model="name" at="x y z" [yaw=deg] [scale=s]/>` — stamp a `<model>`.
fn emit_use(
    el: &Element,
    manifest: &mut WorldManifest,
    decls: &[StyleDecl],
    looks: &mut LookRegistry,
    models: &std::collections::HashMap<String, &Element>,
    depth: usize,
) -> Result<(), String> {
    let name = el.attrs.get("model").ok_or("<use> needs a `model` name")?;
    let model = *models
        .get(name)
        .ok_or_else(|| format!("<use model=\"{name}\"> — no such <model> defined"))?;
    let at = vec3(el.attrs.get("at"))?;
    let yaw = el
        .attrs
        .get("yaw")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0);
    let scale = el
        .attrs
        .get("scale")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1.0);
    stamp_children(
        &model.children,
        manifest,
        decls,
        looks,
        models,
        depth,
        at,
        yaw,
        scale,
    )
}

/// `<ring n=8 r=10 [at] [start=deg] [face="in|out|none"]>…</ring>` — stamp the
/// children at `n` evenly-spaced points of a circle, each turned to face the
/// centre (the default), outward, or not at all. The radial-repetition
/// primitive: lamps around a plaza, stones around a hall, veils around a home.
fn emit_ring(
    el: &Element,
    manifest: &mut WorldManifest,
    decls: &[StyleDecl],
    looks: &mut LookRegistry,
    models: &std::collections::HashMap<String, &Element>,
    depth: usize,
) -> Result<(), String> {
    let c = el
        .attrs
        .get("at")
        .map(|_| vec3(el.attrs.get("at")))
        .transpose()?
        .unwrap_or([0.0; 3]);
    let n = el
        .attrs
        .get("n")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(8)
        .max(1);
    let r = el
        .attrs
        .get("r")
        .and_then(|v| v.parse::<f32>().ok())
        .unwrap_or(8.0);
    let start = el
        .attrs
        .get("start")
        .and_then(|v| v.parse::<f32>().ok())
        .unwrap_or(0.0);
    let face = el.attrs.get("face").map(String::as_str).unwrap_or("in");
    for k in 0..n {
        let az = start + k as f32 * 360.0 / n as f32;
        let (dx, dz) = crate::arch::az_dir(az);
        let at = [c[0] + dx * r, c[1], c[2] + dz * r];
        // Placement yaw ψ maps +Z (a quad's normal) to (sin ψ, cos ψ):
        // facing in = toward the centre = −az_dir(az) = (−sin az, cos az) → ψ = −az
        // (the same sign law as arch::Corridor::yaw_across — derived once, reused).
        let yaw = match face {
            "out" => 180.0 - az,
            "none" => 0.0,
            _ => -az,
        };
        stamp_children(
            &el.children,
            manifest,
            decls,
            looks,
            models,
            depth,
            at,
            yaw,
            1.0,
        )?;
    }
    Ok(())
}

/// `<row n=5 from="x y z" to="x y z" [yaw=deg]>…</row>` — stamp the children
/// at `n` points evenly spaced from `from` to `to` (endpoints included). The
/// linear-repetition primitive: a colonnade, a fence, a row of stalls.
fn emit_row(
    el: &Element,
    manifest: &mut WorldManifest,
    decls: &[StyleDecl],
    looks: &mut LookRegistry,
    models: &std::collections::HashMap<String, &Element>,
    depth: usize,
) -> Result<(), String> {
    let a = vec3(el.attrs.get("from"))?;
    let b = vec3(el.attrs.get("to"))?;
    let n = el
        .attrs
        .get("n")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(2)
        .max(1);
    let yaw = el
        .attrs
        .get("yaw")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0);
    for k in 0..n {
        let t = if n == 1 {
            0.5
        } else {
            k as f32 / (n - 1) as f32
        };
        let at = [
            a[0] + (b[0] - a[0]) * t,
            a[1] + (b[1] - a[1]) * t,
            a[2] + (b[2] - a[2]) * t,
        ];
        stamp_children(
            &el.children,
            manifest,
            decls,
            looks,
            models,
            depth,
            at,
            yaw,
            1.0,
        )?;
    }
    Ok(())
}

/// `<lamp at="x y z" [h=2.2] [color="r g b"]/>` — a post, a warm head, and the
/// light itself, in one element. Light is half of beauty; make it one tag.
fn emit_lamp(
    el: &Element,
    manifest: &mut WorldManifest,
    _decls: &[StyleDecl],
    looks: &mut LookRegistry,
) -> Result<(), String> {
    let at = vec3(el.attrs.get("at"))?;
    let h = el
        .attrs
        .get("h")
        .and_then(|v| v.parse::<f32>().ok())
        .unwrap_or(2.2);
    let light_c = el
        .attrs
        .get("color")
        .and_then(|v| parse_color(v))
        .map(|c| [c[0], c[1], c[2]])
        .unwrap_or([1.0, 0.78, 0.45]);
    let post = looks.ensure(
        &Look {
            mesh: "cylinder".into(),
            base_color: [0.20, 0.19, 0.22, 1.0],
            metallic: 0.6,
            roughness: 0.5,
            emissive: 0.0,
            texture: None,
        },
        manifest,
    );
    push_part(
        manifest,
        post,
        "lamp post",
        "lamp-post",
        [at[0], at[1] + h / 2.0, at[2]],
        0.0,
        [0.16, h, 0.16],
    );
    let head = looks.ensure(
        &Look {
            mesh: "cube".into(),
            base_color: [light_c[0], light_c[1] * 0.95, light_c[2] * 0.85, 1.0],
            metallic: 0.0,
            roughness: 0.2,
            emissive: 0.6,
            texture: None,
        },
        manifest,
    );
    let head_i = manifest.placements.len();
    push_part(
        manifest,
        head,
        "lamp",
        "lamp-head",
        [at[0], at[1] + h + 0.15, at[2]],
        0.0,
        [0.32, 0.32, 0.32],
    );
    manifest.placements[head_i].solid = Some(false);
    manifest.placements[head_i].light = Some(crate::LightEmitter {
        color: light_c,
        intensity: el
            .attrs
            .get("intensity")
            .and_then(|v| v.parse().ok())
            .unwrap_or(1.3),
        range: el
            .attrs
            .get("range")
            .and_then(|v| v.parse().ok())
            .unwrap_or(8.0),
    });
    Ok(())
}

/// Rotate vector `v` by quaternion `q` (`[x y z w]`).
fn quat_rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let [qx, qy, qz, qw] = q;
    // t = 2 q × v ; v' = v + w t + q × t
    let t = [
        2.0 * (qy * v[2] - qz * v[1]),
        2.0 * (qz * v[0] - qx * v[2]),
        2.0 * (qx * v[1] - qy * v[0]),
    ];
    [
        v[0] + qw * t[0] + qy * t[2] - qz * t[1],
        v[1] + qw * t[1] + qz * t[0] - qx * t[2],
        v[2] + qw * t[2] + qx * t[1] - qy * t[0],
    ]
}

fn push_part(
    manifest: &mut WorldManifest,
    prefab: StructuredId,
    name: &str,
    kind: &str,
    position: [f32; 3],
    yaw_deg: f32,
    scale: [f32; 3],
) {
    manifest.placements.push(Placement {
        prefab,
        name: name.into(),
        kind: Some(kind.into()),
        class: vec![],
        position,
        rotation: euler_deg_to_quat(0.0, yaw_deg, 0.0),
        scale,
        codex: None,
        text: None,
        animate: None,
        solid: None,
        light: None,
        behavior: None,
        interaction: None,
        data: serde_json::Value::Null,
        children: vec![],
        extra: Default::default(),
    });
}

/// `<room at r h gates columns gate-width>` — a round, walled, colonnaded
/// chamber: floor disc + wall segments (gapped at each gate azimuth) + columns
/// with capitals at every joint. **Azimuth convention:** degrees where 0 faces
/// −z (away from the default spawn) and 180 faces +z — so `gates="180"` opens
/// the room toward arrivals. Style properties: `color` (walls), `floor-color`,
/// `accent-color` (capitals). Children are authored relative to the room's
/// centre — a room is a coordinate frame, like any parent.
fn emit_room(
    el: &Element,
    manifest: &mut WorldManifest,
    decls: &[StyleDecl],
    looks: &mut LookRegistry,
    models: &std::collections::HashMap<String, &Element>,
    depth: usize,
) -> Result<(), String> {
    let c = vec3(el.attrs.get("at"))?;
    let attr_f = |k: &str| el.attrs.get(k).and_then(|v| v.parse::<f32>().ok());
    let r = attr_f("r").unwrap_or(12.0);
    let h = attr_f("h").unwrap_or(3.6);
    let n = attr_f("columns").map(|v| v as usize).unwrap_or(16).max(6);
    let gate_w = attr_f("gate-width").unwrap_or(14.0);
    let gates: Vec<f32> = el
        .attrs
        .get("gates")
        .map(|g| {
            g.split_whitespace()
                .filter_map(|t| t.parse().ok())
                .collect()
        })
        .unwrap_or_else(|| vec![180.0]);
    let wall_c = resolve_prop(el, decls, "color")
        .and_then(|v| parse_color(&v))
        .unwrap_or([0.62, 0.58, 0.52, 1.0]);
    let floor_c = resolve_prop(el, decls, "floor-color")
        .and_then(|v| parse_color(&v))
        .unwrap_or([0.50, 0.47, 0.42, 1.0]);
    let accent_c = resolve_prop(el, decls, "accent-color")
        .and_then(|v| parse_color(&v))
        .unwrap_or([0.85, 0.68, 0.28, 1.0]);

    let floor = looks.ensure(
        &Look {
            mesh: "cylinder".into(),
            base_color: floor_c,
            metallic: 0.0,
            roughness: 0.9,
            emissive: 0.0,
            texture: None,
        },
        manifest,
    );
    push_part(
        manifest,
        floor,
        "room floor",
        "room-floor",
        [c[0], c[1] + 0.03, c[2]],
        0.0,
        [r * 2.0, 0.06, r * 2.0],
    );
    let wall_pf = looks.ensure(
        &Look {
            mesh: "cube".into(),
            base_color: wall_c,
            metallic: 0.0,
            roughness: 0.85,
            emissive: 0.0,
            texture: None,
        },
        manifest,
    );
    let col_c = [wall_c[0] * 1.15, wall_c[1] * 1.15, wall_c[2] * 1.15, 1.0];
    let col_pf = looks.ensure(
        &Look {
            mesh: "cylinder".into(),
            base_color: col_c,
            metallic: 0.05,
            roughness: 0.55,
            emissive: 0.0,
            texture: None,
        },
        manifest,
    );
    let cap_pf = looks.ensure(
        &Look {
            mesh: "cylinder".into(),
            base_color: accent_c,
            metallic: 0.8,
            roughness: 0.35,
            emissive: 0.1,
            texture: None,
        },
        manifest,
    );

    // One geometry for every builder: the shared architecture math (arch).
    let reach = crate::arch::gate_reach(n, gate_w);
    for s in crate::arch::ring_segments(r, n, &gates, reach) {
        push_part(
            manifest,
            wall_pf,
            "wall",
            "room-wall",
            [c[0] + s.x, c[1] + h / 2.0, c[2] + s.z],
            s.yaw_deg,
            [s.len + 0.2, h, 0.45],
        );
    }
    for (jx, jz) in crate::arch::ring_joints(r, n) {
        let col_h = h * 1.35;
        push_part(
            manifest,
            col_pf,
            "column",
            "room-column",
            [c[0] + jx, c[1] + col_h / 2.0, c[2] + jz],
            0.0,
            [0.5, col_h, 0.5],
        );
        push_part(
            manifest,
            cap_pf,
            "capital",
            "room-capital",
            [c[0] + jx, c[1] + col_h + 0.09, c[2] + jz],
            0.0,
            [0.7, 0.18, 0.7],
        );
    }

    // Children live in the room's frame: emit, then translate to its centre
    // (portals and spawns authored inside a room move with it too).
    let (p0, po0, s0) = (
        manifest.placements.len(),
        manifest.portals.len(),
        manifest.spawns.len(),
    );
    for child in &el.children {
        emit(child, manifest, decls, looks, models, depth + 1)?;
    }
    for p in &mut manifest.placements[p0..] {
        for i in 0..3 {
            p.position[i] += c[i];
        }
    }
    for p in &mut manifest.portals[po0..] {
        for i in 0..3 {
            p.position[i] += c[i];
        }
    }
    for s in &mut manifest.spawns[s0..] {
        for i in 0..3 {
            s.position[i] += c[i];
        }
    }
    Ok(())
}

/// `<wall from="x z" to="x z" h thick>` — a straight wall between two ground
/// points; height and thickness as attributes, colour from the cascade. The
/// piece missing from every blockout: rooms that aren't round.
fn emit_wall(
    el: &Element,
    manifest: &mut WorldManifest,
    decls: &[StyleDecl],
    looks: &mut LookRegistry,
) -> Result<(), String> {
    let two = |k: &str| -> Result<[f32; 2], String> {
        let s = el
            .attrs
            .get(k)
            .ok_or_else(|| format!("<wall> needs `{k}`"))?;
        let n: Vec<f32> = s
            .split_whitespace()
            .filter_map(|t| t.parse().ok())
            .collect();
        match n.as_slice() {
            [x, z] => Ok([*x, *z]),
            _ => Err(format!("<wall {k}> expects 2 numbers, got '{s}'")),
        }
    };
    let (from, to) = (two("from")?, two("to")?);
    let attr_f = |k: &str| el.attrs.get(k).and_then(|v| v.parse::<f32>().ok());
    let h = attr_f("h").unwrap_or(3.0);
    let thick = attr_f("thick").unwrap_or(0.4);
    let y = attr_f("y").unwrap_or(0.0);
    let (dx, dz) = (to[0] - from[0], to[1] - from[1]);
    let len = (dx * dx + dz * dz).sqrt().max(0.01);
    let yaw_deg = (-dz).atan2(dx).to_degrees();
    let color = resolve_prop(el, decls, "color")
        .and_then(|v| parse_color(&v))
        .unwrap_or([0.62, 0.58, 0.52, 1.0]);
    let pf = looks.ensure(
        &Look {
            mesh: "cube".into(),
            base_color: color,
            metallic: 0.0,
            roughness: 0.85,
            emissive: 0.0,
            texture: None,
        },
        manifest,
    );
    push_part(
        manifest,
        pf,
        &el.attrs.get("id").cloned().unwrap_or_else(|| "wall".into()),
        "wall",
        [
            (from[0] + to[0]) / 2.0,
            y + h / 2.0,
            (from[1] + to[1]) / 2.0,
        ],
        yaw_deg,
        [len, h, thick],
    );
    Ok(())
}

/// Per-object data from `url="…"` and `data-*` attributes. Values that parse as
/// numbers or booleans are typed as such (a `data-price="250"` is a number the
/// commerce layer can use); everything else stays a string.
fn data_attrs(el: &Element) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    if let Some(url) = el.attrs.get("url") {
        map.insert("url".into(), serde_json::Value::String(url.clone()));
    }
    for (k, v) in &el.attrs {
        if let Some(key) = k.strip_prefix("data-") {
            map.insert(key.to_string(), typed_value(v));
        }
    }
    if map.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::Value::Object(map)
    }
}

/// Type a raw attribute value: bool, number, or string.
fn typed_value(v: &str) -> serde_json::Value {
    match v {
        "true" => return serde_json::Value::Bool(true),
        "false" => return serde_json::Value::Bool(false),
        _ => {}
    }
    if let Ok(n) = v.parse::<i64>() {
        return serde_json::Value::Number(n.into());
    }
    if let Ok(f) = v.parse::<f64>() {
        if let Some(n) = serde_json::Number::from_f64(f) {
            return serde_json::Value::Number(n);
        }
    }
    serde_json::Value::String(v.to_string())
}

/// The `rules="survival gathering combat"` attribute → [`WorldRules`]. Unknown
/// tokens are an error (a typo silently defaulting to "off" would be cruel).
/// The `<world presence="…">` attribute — the whole of a creator's presence
/// setup, one word (or one address):
///
/// - absent / `"none"` — people off (Tier 0, solo). The default.
/// - `"p2p"` — participants host each other; the first traveler whose browser
///   allows hosting becomes the room's host (presence-topology §3).
/// - a `wss://` (or `ws://`) relay URL — hosted: the creator runs (or rents)
///   a relay and names it. Comma-separate for ordered fallbacks.
fn parse_presence(s: Option<&String>) -> Result<Option<crate::Presence>, String> {
    let Some(s) = s else { return Ok(None) };
    let v = s.trim();
    match v {
        "" | "none" | "off" | "solo" => Ok(None),
        "p2p" => Ok(Some(crate::Presence {
            mode: Some("p2p".into()),
            relay: None,
            relays: vec![],
            rendezvous: None,
            max_occupants: None,
            voice: true,
            owner_required: false,
            extra: Default::default(),
        })),
        urls if urls.starts_with("wss://") || urls.starts_with("ws://") => {
            let relays: Vec<String> = urls
                .split(',')
                .map(str::trim)
                .filter(|u| !u.is_empty())
                .map(String::from)
                .collect();
            Ok(Some(crate::Presence {
                mode: Some("relay".into()),
                relay: None,
                relays,
                rendezvous: None,
                max_occupants: None,
                voice: true,
                owner_required: false,
                extra: Default::default(),
            }))
        }
        other => Err(format!(
            "world presence '{other}' — use \"none\", \"p2p\", or a wss:// relay URL"
        )),
    }
}

fn parse_rules(s: Option<&String>) -> Result<WorldRules, String> {
    let mut rules = WorldRules::default();
    let Some(s) = s else { return Ok(rules) };
    for tok in s.split([' ', ',']).filter(|t| !t.is_empty()) {
        match tok {
            "survival" => rules.survival = true,
            "gathering" => rules.gathering = true,
            "combat" => rules.combat = true,
            other => {
                return Err(format!(
                    "unknown rule '{other}' (survival|gathering|combat)"
                ))
            }
        }
    }
    Ok(rules)
}

/// A parsed `<style>` rule: a selector + its declared properties (raw values) —
/// `mesh`, `color`, `metallic`, `roughness`, `emissive`, `interaction[-…]`.
struct StyleDecl {
    select: String,
    props: BTreeMap<String, String>,
}

/// Build the manifest's runtime [`StyleRule`]s from the declarations: every rule
/// that declares an `interaction` verb becomes a full [`Interaction`], with the
/// `interaction-*` longhands filling hits + effects.
fn interaction_styles(decls: &[StyleDecl]) -> Result<Vec<StyleRule>, String> {
    let mut styles = Vec::new();
    for d in decls {
        let Some(label) = d.props.get("interaction") else {
            // Longhands without a verb are a mistake worth catching early.
            if d.props.keys().any(|k| k.starts_with("interaction-")) {
                return Err(format!(
                    "style rule '{}' has interaction-* longhands but no `interaction:` verb",
                    d.select
                ));
            }
            continue;
        };
        let hits = match d.props.get("interaction-hits") {
            Some(h) => h
                .parse()
                .map_err(|_| format!("bad interaction-hits '{h}'"))?,
            None => 1,
        };
        let mut effects = Vec::new();
        if let Some(gives) = d.props.get("interaction-gives") {
            effects.push(parse_gives(gives)?);
        }
        if let Some(fx) = d.props.get("interaction-effect") {
            effects.push(InteractionEffect::Effect(fx.clone()));
        }
        if let Some(msg) = d.props.get("interaction-message") {
            effects.push(InteractionEffect::Message(msg.clone()));
        }
        if d.props
            .get("interaction-despawns")
            .is_some_and(|v| v != "false")
        {
            effects.push(InteractionEffect::Despawn);
        }
        styles.push(StyleRule {
            select: d.select.clone(),
            interaction: Some(Interaction {
                label: label.clone(),
                hits,
                effects,
            }),
        });
    }
    Ok(styles)
}

/// `interaction-gives: <item-id> [count]` — an item StructuredId plus a count.
fn parse_gives(v: &str) -> Result<InteractionEffect, String> {
    let mut toks = v.split_whitespace();
    let id: u32 = toks
        .next()
        .and_then(|t| t.parse().ok())
        .ok_or_else(|| format!("interaction-gives needs an item id, got '{v}'"))?;
    let count = match toks.next() {
        Some(c) => c
            .parse()
            .map_err(|_| format!("bad interaction-gives count in '{v}'"))?,
        None => 1,
    };
    Ok(InteractionEffect::GiveItem {
        item: StructuredId(id),
        count,
    })
}

/// Match an element against a simple selector, returning CSS specificity if it
/// matches (id 2 > class 1 > type 0).
fn sel_specificity(el: &Element, select: &str) -> Option<u8> {
    let s = select.trim();
    if let Some(id) = s.strip_prefix('#') {
        return (el.attrs.get("id").map(String::as_str) == Some(id)).then_some(2);
    }
    if let Some(c) = s.strip_prefix('.') {
        let classes = el.attrs.get("class").map(String::as_str).unwrap_or("");
        return classes.split_whitespace().any(|x| x == c).then_some(1);
    }
    (el.tag == s).then_some(0)
}

/// The value a property cascades to for this element: highest-specificity rule
/// that declares it (later rules break ties). Independent per property, like CSS.
fn resolve_prop(el: &Element, decls: &[StyleDecl], prop: &str) -> Option<String> {
    // Inline beats the cascade — exactly the web's rule. (This was broken
    // once: inline `color=` silently dropped and whole worlds rendered white;
    // the snapshot loop caught it. Keep this line first.)
    if let Some(v) = el.attrs.get(prop) {
        return Some(v.clone());
    }
    let mut best: Option<(u8, &str)> = None;
    for d in decls {
        if let (Some(spec), Some(v)) = (sel_specificity(el, &d.select), d.props.get(prop)) {
            if best.is_none_or(|(s, _)| spec >= s) {
                best = Some((spec, v));
            }
        }
    }
    best.map(|(_, v)| v.to_string())
}

/// The resolved appearance of an element: a mesh plus its PBR material scalars.
struct Look {
    mesh: String,
    base_color: [f32; 4],
    metallic: f32,
    roughness: f32,
    /// >0 renders unlit, glowing its base colour (blooms) — `emissive: true` = 1.
    emissive: f32,
    /// Named procedural material (`texture="granite"`), baked by the browser.
    texture: Option<String>,
}

impl Look {
    /// A stable key so identical looks share one synthesized prefab.
    fn key(&self) -> String {
        format!(
            "{}|{:?}|{}|{}|{}|{:?}",
            self.mesh, self.base_color, self.metallic, self.roughness, self.emissive, self.texture
        )
    }
    fn is_plain(&self) -> bool {
        self.base_color == [1.0, 1.0, 1.0, 1.0]
            && self.metallic == 0.0
            && self.roughness == 1.0
            && self.emissive == 0.0
            && self.texture.is_none()
    }
}

/// `emissive: true` (full glow) or a strength number.
fn parse_emissive(v: &str) -> f32 {
    match v {
        "true" => 1.0,
        _ => v.parse().unwrap_or(0.0),
    }
}

/// Named colours the `color:` property accepts, besides `r g b [a]` floats.
fn parse_color(s: &str) -> Option<[f32; 4]> {
    let named = match s.trim() {
        "white" => [1.0, 1.0, 1.0, 1.0],
        "black" => [0.0, 0.0, 0.0, 1.0],
        "red" => [0.8, 0.15, 0.15, 1.0],
        "green" => [0.2, 0.6, 0.2, 1.0],
        "blue" => [0.2, 0.4, 0.8, 1.0],
        "yellow" => [0.9, 0.85, 0.2, 1.0],
        "orange" => [0.9, 0.5, 0.15, 1.0],
        "brown" => [0.4, 0.26, 0.13, 1.0],
        "gray" | "grey" => [0.5, 0.5, 0.5, 1.0],
        _ => {
            let n: Vec<f32> = s
                .split_whitespace()
                .filter_map(|t| t.parse().ok())
                .collect();
            return match n.as_slice() {
                [r, g, b] => Some([*r, *g, *b, 1.0]),
                [r, g, b, a] => Some([*r, *g, *b, *a]),
                _ => None,
            };
        }
    };
    Some(named)
}

/// Synthesizes one prefab per unique *look* (mesh + material), plus one `assets[]`
/// entry per unique mesh URL — so cascaded appearance renders through the normal
/// prefab pipeline. URLs may be local paths or CDN/ipfs URIs.
struct LookRegistry {
    prefabs: BTreeMap<String, StructuredId>,
    assets: BTreeMap<String, String>,
    /// Named `<shape>` recipes (tree + meshing resolution), by name.
    shapes: BTreeMap<String, (crate::shape::Shape, Option<u32>)>,
    /// Named `<texture>` recipes, by name.
    textures: BTreeMap<String, crate::texture::TextureRecipe>,
    next_prefab: u32,
    next_asset: u32,
}

impl LookRegistry {
    fn new() -> Self {
        Self {
            prefabs: BTreeMap::new(),
            assets: BTreeMap::new(),
            shapes: BTreeMap::new(),
            textures: BTreeMap::new(),
            next_prefab: 60910001,
            next_asset: 0,
        }
    }

    fn ensure(&mut self, look: &Look, manifest: &mut WorldManifest) -> StructuredId {
        if let Some(id) = self.prefabs.get(&look.key()) {
            return *id;
        }
        let mesh_ref = if BUILTINS.contains(&look.mesh.as_str()) {
            MeshRef {
                asset: None,
                builtin: Some(look.mesh.clone()),
                shape: None,
                resolution: None,
            }
        } else if let Some(name) = look.mesh.strip_prefix("shape:") {
            // A carved shape: the recipe rides in the prefab itself. The name
            // was checked against the registry at the usage site.
            let (tree, resolution) = self
                .shapes
                .get(name)
                .cloned()
                .expect("shape checked at usage site");
            MeshRef {
                asset: None,
                builtin: None,
                shape: Some(tree),
                resolution,
            }
        } else {
            // Dedup the asset URL across looks that share the mesh but differ in colour.
            let asset_id = match self.assets.get(&look.mesh) {
                Some(a) => a.clone(),
                None => {
                    let aid = format!("mesh-{}", self.next_asset);
                    self.next_asset += 1;
                    manifest.assets.push(Asset {
                        id: aid.clone(),
                        uri: look.mesh.clone(),
                        kind: AssetKind::Gltf,
                    });
                    self.assets.insert(look.mesh.clone(), aid.clone());
                    aid
                }
            };
            MeshRef {
                asset: Some(asset_id),
                builtin: None,
                shape: None,
                resolution: None,
            }
        };
        let material = (!look.is_plain()).then(|| MaterialRef {
            base_color: look.base_color,
            metallic: look.metallic,
            roughness: look.roughness,
            emissive: look.emissive,
            texture: look
                .texture
                .as_ref()
                .and_then(|t| self.textures.get(t).cloned()),
            ..Default::default()
        });
        let id = StructuredId(self.next_prefab);
        self.next_prefab += 1;
        manifest.prefabs.push(Prefab {
            id,
            mesh: mesh_ref,
            material,
            extra: Default::default(),
        });
        self.prefabs.insert(look.key(), id);
        id
    }
}

fn default_panel_size() -> f32 {
    0.09
}
fn default_panel_ink() -> [f32; 3] {
    [0.13, 0.11, 0.10]
}
fn default_panel_paper() -> [f32; 3] {
    [0.93, 0.89, 0.80]
}

/// The `weft="<uri>"` attribute — bind a **Weft** module (the Thread's native
/// code, weft-v0.1) to this object: synthesizes the asset + behavior
/// declarations and returns the binding id. One line of markup = verified,
/// fuel-metered interactivity.
fn weft_binding(el: &Element, manifest: &mut WorldManifest) -> Option<String> {
    // `weft-on="interact tick"` picks the events; default stays interact-only.
    // A `tick` subscription gives the module the world's heartbeat.
    let on: Vec<String> = el
        .attrs
        .get("weft-on")
        .map(|v| v.split_whitespace().map(str::to_string).collect())
        .filter(|v: &Vec<String>| !v.is_empty())
        .unwrap_or_else(|| vec!["interact".into()]);
    let bid = format!("weft-b{}", manifest.behaviors.len());
    // `weft-use="<uri>#<export>"` — bind a **published package's export**
    // (weft-pack-v0.1): one line of markup consumes the ecosystem.
    if let Some(spec) = el.attrs.get("weft-use") {
        let (uri, export) = spec.split_once('#')?;
        if uri.is_empty() || export.is_empty() {
            return None;
        }
        let aid = format!("weft-a{}", manifest.assets.len());
        manifest.assets.push(Asset {
            id: aid.clone(),
            uri: uri.to_string(),
            kind: AssetKind::Weft,
        });
        manifest.behaviors.push(crate::Behavior {
            id: bid.clone(),
            wasm: String::new(),
            weft: None,
            weft_pack: Some(aid),
            weft_export: Some(export.to_string()),
            on,
        });
        return Some(bid);
    }
    let uri = el.attrs.get("weft")?;
    let aid = format!("weft-a{}", manifest.assets.len());
    manifest.assets.push(Asset {
        id: aid.clone(),
        uri: uri.clone(),
        kind: AssetKind::Weft,
    });
    manifest.behaviors.push(crate::Behavior {
        id: bid.clone(),
        wasm: String::new(),
        weft: Some(aid),
        weft_pack: None,
        weft_export: None,
        on,
    });
    Some(bid)
}

/// The `animate` style property: `animate: spin [speed]` or
/// `animate: bob [speed] [amp]` — data-driven idle motion (browser-executed).
fn parse_animate(v: Option<String>) -> Option<Animate> {
    let v = v?;
    let mut toks = v.split_whitespace();
    let kind = toks.next()?.to_string();
    let speed = toks.next().and_then(|t| t.parse().ok()).unwrap_or(1.0);
    let amp = toks.next().and_then(|t| t.parse().ok()).unwrap_or(0.25);
    Some(Animate {
        kind,
        speed,
        amp,
        points: vec![],
    })
}

// --- attribute parsing ---

fn vec3(s: Option<&String>) -> Result<[f32; 3], String> {
    let s = s.map(String::as_str).unwrap_or("0 0 0");
    let nums: Vec<f32> = s
        .split_whitespace()
        .filter_map(|t| t.parse().ok())
        .collect();
    match nums.as_slice() {
        [x, y, z] => Ok([*x, *y, *z]),
        _ => Err(format!("expected 3 numbers, got '{s}'")),
    }
}

/// The `scale` attribute (default unit).
fn scale_attr(el: &Element) -> Result<[f32; 3], String> {
    match el.attrs.get("scale") {
        Some(_) => vec3(el.attrs.get("scale")),
        None => Ok([1.0, 1.0, 1.0]),
    }
}

/// The `rot="x y z"` attribute — Euler angles in **degrees**, applied yaw (Y),
/// then pitch (X), then roll (Z) — as a unit quaternion `[x y z w]`.
fn rot_attr(el: &Element) -> Result<[f32; 4], String> {
    match el.attrs.get("rot") {
        None => Ok([0.0, 0.0, 0.0, 1.0]),
        Some(_) => {
            let [x, y, z] = vec3(el.attrs.get("rot"))?;
            Ok(euler_deg_to_quat(x, y, z))
        }
    }
}

/// Euler degrees (x pitch, y yaw, z roll) → quaternion `[x y z w]`, composed
/// `qy * qx * qz` (yaw, then pitch, then roll — the common authoring intuition).
fn euler_deg_to_quat(x: f32, y: f32, z: f32) -> [f32; 4] {
    let (hx, hy, hz) = (
        x.to_radians() / 2.0,
        y.to_radians() / 2.0,
        z.to_radians() / 2.0,
    );
    let qx = [hx.sin(), 0.0, 0.0, hx.cos()];
    let qy = [0.0, hy.sin(), 0.0, hy.cos()];
    let qz = [0.0, 0.0, hz.sin(), hz.cos()];
    quat_mul(quat_mul(qy, qx), qz)
}

/// Hamilton product of two `[x y z w]` quaternions.
fn quat_mul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let [ax, ay, az, aw] = a;
    let [bx, by, bz, bw] = b;
    [
        aw * bx + bw * ax + ay * bz - az * by,
        aw * by + bw * ay + az * bx - ax * bz,
        aw * bz + bw * az + ax * by - ay * bx,
        aw * bw - ax * bx - ay * by - az * bz,
    ]
}

/// `sky` accepts a named preset (`dawn`/`day`/`dusk`/`night`/`void`) — a tuned
/// zenith + horizon + sun that just looks good — or explicit gradients:
/// `"zr zg zb / hr hg hb"`, with an optional third segment for the sun
/// direction (`"… / … / sx sy sz"`).
fn parse_sky(s: Option<&String>) -> Option<Sky> {
    let s = s?;
    if let Some(preset) = sky_preset(s.trim()) {
        return Some(preset);
    }
    let mut segs = s.split('/');
    let z: Vec<f32> = segs
        .next()?
        .split_whitespace()
        .filter_map(|t| t.parse().ok())
        .collect();
    let h: Vec<f32> = segs
        .next()?
        .split_whitespace()
        .filter_map(|t| t.parse().ok())
        .collect();
    let sun: Vec<f32> = segs
        .next()
        .map(|seg| {
            seg.split_whitespace()
                .filter_map(|t| t.parse().ok())
                .collect()
        })
        .unwrap_or_default();
    if let ([zx, zy, zz], [hx, hy, hz]) = (z.as_slice(), h.as_slice()) {
        let sun_dir = match sun.as_slice() {
            [sx, sy, sz] => Some([*sx, *sy, *sz]),
            _ => None,
        };
        Some(Sky {
            zenith: [*zx, *zy, *zz],
            horizon: [*hx, *hy, *hz],
            sun_dir,
        })
    } else {
        None
    }
}

/// The named sky presets — each a complete atmosphere (gradient + sun angle)
/// tuned for the standard rendering loop, so a world looks good by naming a
/// time of day instead of tuning numbers.
fn sky_preset(name: &str) -> Option<Sky> {
    let (zenith, horizon, sun) = match name {
        "dawn" => ([0.30, 0.38, 0.60], [0.95, 0.62, 0.42], [0.75, 0.22, 0.35]),
        "day" | "noon" => ([0.24, 0.48, 0.85], [0.72, 0.82, 0.92], [0.30, 0.85, 0.25]),
        "dusk" => ([0.22, 0.16, 0.38], [0.92, 0.44, 0.28], [-0.70, 0.16, 0.40]),
        "night" => ([0.02, 0.03, 0.08], [0.07, 0.09, 0.16], [0.40, 0.20, 0.50]),
        "void" => ([0.05, 0.06, 0.12], [0.18, 0.16, 0.24], [0.30, 0.70, 0.20]),
        _ => return None,
    };
    Some(Sky {
        zenith,
        horizon,
        sun_dir: Some(sun),
    })
}

// --- <style> block ---

/// Split out the `<style>…</style>` body (if any) from the markup.
fn extract_style(src: &str) -> (String, String) {
    if let (Some(open), Some(close)) = (src.find("<style>"), src.find("</style>")) {
        if close > open {
            let body = src[open + "<style>".len()..close].to_string();
            let markup = format!("{}{}", &src[..open], &src[close + "</style>".len()..]);
            return (markup, body);
        }
    }
    (src.to_string(), String::new())
}

/// Parse a `<style>` body into per-selector property maps (raw string values).
/// A block is `selector { prop: value; prop: value }`; empty blocks are skipped.
fn parse_style(body: &str) -> Result<Vec<StyleDecl>, String> {
    let mut decls = Vec::new();
    for block in body.split('}') {
        let block = block.trim();
        if block.is_empty() {
            continue;
        }
        let (select, body) = block
            .split_once('{')
            .ok_or_else(|| format!("style rule missing '{{': {block}"))?;
        let mut props = BTreeMap::new();
        for decl in body.split(';') {
            if let Some((prop, val)) = decl.trim().split_once(':') {
                let val = val.trim().trim_matches('"').to_string();
                if !val.is_empty() {
                    props.insert(prop.trim().to_string(), val);
                }
            }
        }
        if !props.is_empty() {
            decls.push(StyleDecl {
                select: select.trim().to_string(),
                props,
            });
        }
    }
    Ok(decls)
}

// --- markup parser (a tiny XML-ish recursive descent) ---

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Parser<'a> {
    fn new(s: &'a str) -> Self {
        Self {
            s: s.as_bytes(),
            i: 0,
        }
    }

    fn parse_all(&mut self) -> Result<Vec<Element>, String> {
        self.parse_children(None)
    }

    /// Parse sibling elements until EOF, or until the closing tag `until`.
    fn parse_children(&mut self, until: Option<&str>) -> Result<Vec<Element>, String> {
        let mut out = Vec::new();
        loop {
            self.skip_trivia();
            if self.i >= self.s.len() {
                return match until {
                    Some(u) => Err(format!("unclosed <{u}>")),
                    None => Ok(out),
                };
            }
            if self.starts_with("</") {
                self.i += 2;
                let name = self.read_name();
                self.expect('>')?;
                return match until {
                    Some(u) if u == name => Ok(out),
                    Some(u) => Err(format!("mismatched close </{name}> (wanted </{u}>)")),
                    None => Err(format!("unexpected close </{name}>")),
                };
            }
            if self.peek() == Some(b'<') {
                out.push(self.parse_element()?);
            } else {
                // Text between tags is ignored — worlds are element trees.
                while self.i < self.s.len() && self.s[self.i] != b'<' {
                    self.i += 1;
                }
            }
        }
    }

    fn parse_element(&mut self) -> Result<Element, String> {
        self.expect('<')?;
        let tag = self.read_name();
        if tag.is_empty() {
            return Err("empty tag name".into());
        }
        let mut attrs = BTreeMap::new();
        loop {
            self.skip_ws();
            match self.peek() {
                Some(b'/') => {
                    self.i += 1;
                    self.expect('>')?;
                    return Ok(Element {
                        tag,
                        attrs,
                        children: vec![],
                    });
                }
                Some(b'>') => {
                    self.i += 1;
                    let children = self.parse_children(Some(&tag))?;
                    return Ok(Element {
                        tag,
                        attrs,
                        children,
                    });
                }
                Some(_) => {
                    let (k, v) = self.read_attr()?;
                    attrs.insert(k, v);
                }
                None => return Err(format!("unclosed <{tag}>")),
            }
        }
    }

    fn read_attr(&mut self) -> Result<(String, String), String> {
        let name = self.read_name();
        if name.is_empty() {
            return Err("expected attribute name".into());
        }
        self.skip_ws();
        if self.peek() != Some(b'=') {
            return Ok((name, String::new())); // valueless attribute
        }
        self.i += 1;
        self.skip_ws();
        let quote = self
            .peek()
            .filter(|c| *c == b'"' || *c == b'\'')
            .ok_or("attribute value must be quoted")?;
        self.i += 1;
        let start = self.i;
        while self.i < self.s.len() && self.s[self.i] != quote {
            self.i += 1;
        }
        let val = std::str::from_utf8(&self.s[start..self.i])
            .unwrap_or_default()
            .to_string();
        self.expect(quote as char)?;
        Ok((name, val))
    }

    fn read_name(&mut self) -> String {
        let start = self.i;
        while self.i < self.s.len() {
            let c = self.s[self.i];
            if c.is_ascii_alphanumeric() || c == b'-' || c == b'_' {
                self.i += 1;
            } else {
                break;
            }
        }
        std::str::from_utf8(&self.s[start..self.i])
            .unwrap_or_default()
            .to_string()
    }

    fn skip_ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    /// Skip whitespace and `<!-- comments -->`.
    fn skip_trivia(&mut self) {
        loop {
            self.skip_ws();
            if self.starts_with("<!--") {
                if let Some(end) = self.find_from(self.i, "-->") {
                    self.i = end + 3;
                    continue;
                }
            }
            break;
        }
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }
    fn starts_with(&self, p: &str) -> bool {
        self.s[self.i..].starts_with(p.as_bytes())
    }
    fn find_from(&self, from: usize, p: &str) -> Option<usize> {
        self.s[from..]
            .windows(p.len())
            .position(|w| w == p.as_bytes())
            .map(|k| from + k)
    }
    fn expect(&mut self, c: char) -> Result<(), String> {
        if self.peek() == Some(c as u8) {
            self.i += 1;
            Ok(())
        } else {
            Err(format!("expected '{c}' at byte {}", self.i))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_stamp_with_transform_and_recursion_is_bounded() {
        let m = compile(
            r#"<world id="w" title="W">
                 <model name="lantern">
                   <cube id="base" at="0 0.5 0" scale="1 1 1"/>
                   <cube id="top" at="0 1.25 0" scale="0.5 0.5 0.5"/>
                 </model>
                 <use model="lantern" at="10 0 0"/>
                 <use model="lantern" at="0 0 -10" yaw="90" scale="2"/>
               </world>"#,
        )
        .unwrap();
        // Two stamps × two parts; the definition itself emitted nothing.
        let parts: Vec<_> = m
            .placements
            .iter()
            .filter(|p| p.name == "base" || p.name == "top")
            .collect();
        assert_eq!(parts.len(), 4);
        // Stamp 1: plain translation.
        let b1 = m
            .placements
            .iter()
            .find(|p| p.name == "base" && p.position[0] > 5.0)
            .unwrap();
        assert_eq!(b1.position, [10.0, 0.5, 10.0 * 0.0]);
        // Stamp 2: scaled ×2 (top sits at 2×1.25 above) and yawed 90°.
        let t2 = m
            .placements
            .iter()
            .find(|p| p.name == "top" && p.position[2] < -5.0)
            .expect("second stamp's top");
        assert!(
            (t2.position[1] - 2.5).abs() < 1e-4,
            "scaled height: {:?}",
            t2.position
        );
        assert_eq!(t2.scale, [1.0, 1.0, 1.0], "0.5 × 2");
        // Unknown model and self-use both fail loudly.
        assert!(
            compile(r#"<world id="w" title="W"><use model="ghost" at="0 0 0"/></world>"#).is_err()
        );
        assert!(compile(
            r#"<world id="w" title="W">
                 <model name="ouro"><use model="ouro" at="0 0 0"/></model>
                 <use model="ouro" at="0 0 0"/>
               </world>"#
        )
        .is_err());
    }

    #[test]
    fn ring_places_n_facing_the_centre_and_row_interpolates() {
        let m = compile(
            r#"<world id="w" title="W">
                 <ring n="4" r="10">
                   <quad id="board" at="0 1.5 0" scale="2 1 1"/>
                 </ring>
                 <row n="3" from="-4 0 5" to="4 0 5">
                   <cube id="post" at="0 1 0" scale="0.2 2 0.2"/>
                 </row>
               </world>"#,
        )
        .unwrap();
        let boards: Vec<_> = m.placements.iter().filter(|p| p.name == "board").collect();
        assert_eq!(boards.len(), 4);
        for b in &boards {
            let r = (b.position[0].powi(2) + b.position[2].powi(2)).sqrt();
            assert!((r - 10.0).abs() < 1e-3, "on the circle: {:?}", b.position);
            // Facing in: the quad's +Z normal (rotated) points at the centre.
            let n = quat_rotate(b.rotation, [0.0, 0.0, 1.0]);
            let toward = [-b.position[0] / r, 0.0, -b.position[2] / r];
            let dot = n[0] * toward[0] + n[2] * toward[2];
            assert!(
                dot > 0.999,
                "board faces centre (dot {dot}) at {:?}",
                b.position
            );
        }
        let posts: Vec<_> = m.placements.iter().filter(|p| p.name == "post").collect();
        assert_eq!(posts.len(), 3);
        let xs: Vec<f32> = posts.iter().map(|p| p.position[0]).collect();
        assert!(
            xs.contains(&-4.0) && xs.contains(&0.0) && xs.contains(&4.0),
            "{xs:?}"
        );
    }

    #[test]
    fn lamp_is_geometry_plus_light_in_one_tag() {
        let m = compile(r#"<world id="w" title="W"><lamp at="3 0 -2" h="2.0"/></world>"#).unwrap();
        let head = m
            .placements
            .iter()
            .find(|p| p.kind.as_deref() == Some("lamp-head"))
            .unwrap();
        let l = head.light.as_ref().expect("the lamp lights");
        assert!(l.intensity > 0.0);
        assert!(
            head.position[1] > 2.0,
            "head atop the post: {:?}",
            head.position
        );
        assert!(m
            .placements
            .iter()
            .any(|p| p.kind.as_deref() == Some("lamp-post")));
    }

    #[test]
    fn presence_attr_is_the_whole_setup() {
        // The default: people off.
        let m = compile(r#"<world id="w" title="W"></world>"#).unwrap();
        assert!(m.presence.is_none());
        let m = compile(r#"<world id="w" title="W" presence="none"></world>"#).unwrap();
        assert!(m.presence.is_none());
        // One word: p2p.
        let m = compile(r#"<world id="w" title="W" presence="p2p"></world>"#).unwrap();
        let p = m.presence.unwrap();
        assert_eq!(p.mode.as_deref(), Some("p2p"));
        assert!(p.relays.is_empty());
        // One address (plus a fallback): hosted.
        let m = compile(
            r#"<world id="w" title="W" presence="wss://a.example/thread/w, wss://b.example/thread/w"></world>"#,
        )
        .unwrap();
        let p = m.presence.unwrap();
        assert_eq!(p.mode.as_deref(), Some("relay"));
        assert_eq!(p.relays.len(), 2);
        // A typo is a compile error, never a silently-solo world.
        assert!(compile(r#"<world id="w" title="W" presence="p2"></world>"#).is_err());
    }

    #[test]
    fn compiles_a_small_world() {
        let src = r##"
            <world id="grove" title="The Grove" sky="0.1 0.1 0.2 / 0.5 0.3 0.2">
              <cube class="harvestable" at="3 0 -2" scale="1 3 1"/>
              <cube at="0 0 -5">
                <sphere at="0 1 0"/>
              </cube>
              <portal to="thread://market" at="0 0 -8" label="Market"/>
            </world>
            <style>
              .harvestable { interaction: "Chop" }
            </style>
        "##;
        let m = compile(src).expect("compiles");
        assert_eq!(m.world.id, "grove");
        assert_eq!(m.world.title, "The Grove");
        assert!(m.environment.sky.is_some());
        // Two top-level cubes; the sphere is nested under the second cube.
        assert_eq!(m.placements.len(), 2);
        let nested = &m.placements[1];
        assert_eq!(nested.children.len(), 1);
        assert_eq!(nested.children[0].kind.as_deref(), Some("sphere"));
        // The harvestable cube got its class; the cascade attaches the interaction.
        let harvest = m
            .placements
            .iter()
            .find(|p| p.class.iter().any(|c| c == "harvestable"))
            .unwrap();
        assert_eq!(m.computed_interaction(harvest).unwrap().label, "Chop");
        // One portal, prefabs synthesized for the used builtins (cube + sphere).
        assert_eq!(m.portals.len(), 1);
        assert_eq!(m.prefabs.len(), 2);
        // The whole thing is a conformant manifest.
        assert!(m.validate().is_ok());
    }

    #[test]
    fn self_closing_and_valued_attrs_parse() {
        let m = compile(r#"<world id="w" title="W"><cube at="0 0 0"/></world>"#).unwrap();
        assert_eq!(m.placements.len(), 1);
        assert_eq!(m.placements[0].position, [0.0, 0.0, 0.0]);
    }

    #[test]
    fn missing_world_is_an_error() {
        assert!(compile(r#"<cube at="0 0 0"/>"#).is_err());
    }

    #[test]
    fn cascade_mesh_synthesizes_prefab_and_asset() {
        // `tree { mesh: "…" }` gives an arbitrary <tree> tag its glTF — the
        // `tree { mesh: tree.glb }` vision. cube falls back to its builtin.
        let m = compile(
            r#"
            <world id="w" title="W">
              <tree at="0 0 0"/>
              <tree at="5 0 0"/>
              <cube at="1 0 0"/>
            </world>
            <style>
              tree { mesh: "trees/oak.glb" }
            </style>
        "#,
        )
        .unwrap();
        // One prefab for tree (asset) + one for cube (builtin), deduped across the
        // two trees; one assets[] entry for the glTF url.
        assert_eq!(m.prefabs.len(), 2);
        assert_eq!(m.assets.len(), 1);
        assert_eq!(m.assets[0].uri, "trees/oak.glb");
        let tree = m
            .placements
            .iter()
            .find(|p| p.kind.as_deref() == Some("tree"))
            .unwrap();
        let prefab = m.prefabs.iter().find(|p| p.id == tree.prefab).unwrap();
        assert_eq!(prefab.mesh.asset.as_deref(), Some("mesh-0"));
        assert!(prefab.mesh.builtin.is_none());
    }

    #[test]
    fn unknown_tag_without_a_mesh_rule_errors() {
        let err = compile(r#"<world id="w" title="W"><dragon at="0 0 0"/></world>"#).unwrap_err();
        assert!(err.contains("no mesh for <dragon>"), "got: {err}");
    }

    #[test]
    fn cascade_color_makes_a_distinct_material() {
        let m = compile(
            r#"
            <world id="w" title="W">
              <cube class="gold" at="0 0 0"/>
              <cube at="1 0 0"/>
            </world>
            <style>
              .gold { color: yellow; metallic: 1 }
            </style>
        "#,
        )
        .unwrap();
        // Same mesh (cube), but the gold one has a material → two distinct prefabs.
        assert_eq!(m.prefabs.len(), 2);
        let gold = m
            .placements
            .iter()
            .find(|p| p.class.iter().any(|c| c == "gold"))
            .unwrap();
        let mat = m
            .prefabs
            .iter()
            .find(|p| p.id == gold.prefab)
            .unwrap()
            .material
            .as_ref()
            .expect("gold cube has a material");
        assert_eq!(mat.base_color, [0.9, 0.85, 0.2, 1.0]); // named "yellow"
        assert_eq!(mat.metallic, 1.0);
        // The plain cube shares the mesh but has no material block.
        let plain = m.placements.iter().find(|p| p.class.is_empty()).unwrap();
        assert!(m
            .prefabs
            .iter()
            .find(|p| p.id == plain.prefab)
            .unwrap()
            .material
            .is_none());
    }

    #[test]
    fn spawn_element_and_default_spawn() {
        // An authored <spawn> lands verbatim (yaw degrees → radians)…
        let m = compile(
            r#"<world id="w" title="W"><spawn at="0 0 8" name="gate" yaw="180"/><cube/></world>"#,
        )
        .unwrap();
        assert_eq!(m.spawns.len(), 1);
        assert_eq!(m.spawns[0].name, "gate");
        assert_eq!(m.spawns[0].position, [0.0, 0.0, 8.0]);
        assert!((m.spawns[0].yaw - std::f32::consts::PI).abs() < 1e-5);
        // …and a world with none still gets a default arrival point.
        let d = compile(r#"<world id="w" title="W"><cube/></world>"#).unwrap();
        assert_eq!(d.spawns.len(), 1);
        assert_eq!(d.spawns[0].name, "entry");
    }

    #[test]
    fn rot_attr_becomes_a_quaternion() {
        let m = compile(r#"<world id="w" title="W"><cube rot="0 90 0"/></world>"#).unwrap();
        let q = m.placements[0].rotation;
        let s45 = (std::f32::consts::FRAC_PI_4).sin();
        assert!(
            (q[1] - s45).abs() < 1e-5 && (q[3] - s45).abs() < 1e-5,
            "got {q:?}"
        );
        assert!(q[0].abs() < 1e-6 && q[2].abs() < 1e-6);
        // Unit length.
        let len: f32 = q.iter().map(|c| c * c).sum::<f32>().sqrt();
        assert!((len - 1.0).abs() < 1e-5);
    }

    #[test]
    fn codex_url_and_data_attrs_flow_to_the_placement() {
        let m = compile(
            r#"<world id="w" title="W">
                 <cube codex="the-thread" url="https://example.com"
                       data-price="250" data-item="20100001" data-featured="true" data-note="hi"/>
               </world>"#,
        )
        .unwrap();
        let p = &m.placements[0];
        assert_eq!(p.codex.as_deref(), Some("the-thread"));
        assert_eq!(p.data["url"], "https://example.com");
        assert_eq!(p.data["price"], 250);
        assert_eq!(p.data["item"], 20100001);
        assert_eq!(p.data["featured"], true);
        assert_eq!(p.data["note"], "hi");
    }

    #[test]
    fn world_rules_year_and_description_parse() {
        let m = compile(
            r#"<world id="w" title="W" description="A test" year="-2400" rules="survival, gathering"><cube/></world>"#,
        )
        .unwrap();
        assert_eq!(m.world.description, "A test");
        assert_eq!(m.environment.year, Some(-2400));
        assert!(m.environment.rules.survival);
        assert!(m.environment.rules.gathering);
        assert!(!m.environment.rules.combat);
        // A typo'd rule is an error, not a silent no-op.
        assert!(compile(r#"<world id="w" title="W" rules="surival"><cube/></world>"#).is_err());
    }

    #[test]
    fn interaction_longhands_build_full_effects() {
        let m = compile(
            r#"
            <world id="w" title="W"><tree class="harvestable"/></world>
            <style>
              tree { mesh: "oak.glb" }
              .harvestable {
                interaction: "Chop";
                interaction-hits: 3;
                interaction-gives: 20100001 3;
                interaction-message: "You chopped the old oak.";
                interaction-despawns: true;
              }
            </style>
        "#,
        )
        .unwrap();
        let i = m.computed_interaction(&m.placements[0]).expect("cascades");
        assert_eq!(i.label, "Chop");
        assert_eq!(i.hits, 3);
        assert_eq!(
            i.effects,
            vec![
                InteractionEffect::GiveItem {
                    item: StructuredId(20100001),
                    count: 3
                },
                InteractionEffect::Message("You chopped the old oak.".into()),
                InteractionEffect::Despawn,
            ]
        );
        // Longhands without a verb are caught at compile time.
        let err = compile(
            r#"<world id="w" title="W"><cube/></world><style>cube { interaction-hits: 2 }</style>"#,
        )
        .unwrap_err();
        assert!(err.contains("no `interaction:` verb"), "got: {err}");
    }

    #[test]
    fn sky_presets_and_explicit_sun_parse() {
        // A named preset gives a full atmosphere, sun included.
        let m = compile(r#"<world id="w" title="W" sky="dusk"><cube/></world>"#).unwrap();
        let sky = m.environment.sky.expect("preset resolves");
        assert!(sky.sun_dir.is_some(), "presets set a sun");
        // Explicit gradients still work, now with an optional sun segment.
        let m = compile(
            r#"<world id="w" title="W" sky="0.1 0.1 0.2 / 0.5 0.3 0.2 / 0.7 0.2 0.3"><cube/></world>"#,
        )
        .unwrap();
        let sky = m.environment.sky.unwrap();
        assert_eq!(sky.zenith, [0.1, 0.1, 0.2]);
        assert_eq!(sky.sun_dir, Some([0.7, 0.2, 0.3]));
        // Two-segment form keeps its old meaning (no sun declared).
        let m =
            compile(r#"<world id="w" title="W" sky="0.1 0.1 0.2 / 0.5 0.3 0.2"><cube/></world>"#)
                .unwrap();
        assert_eq!(m.environment.sky.unwrap().sun_dir, None);
    }

    #[test]
    fn room_compiles_to_floor_walls_columns_with_gates() {
        let m = compile(
            r#"<world id="w" title="W">
                 <room at="0 0 0" r="10" h="3" columns="12" gates="180 0" gate-width="15">
                   <cube id="exhibit" at="0 1 0"/>
                 </room>
               </world>"#,
        )
        .unwrap();
        let walls = m
            .placements
            .iter()
            .filter(|p| p.kind.as_deref() == Some("room-wall"))
            .count();
        let cols = m
            .placements
            .iter()
            .filter(|p| p.kind.as_deref() == Some("room-column"))
            .count();
        // 12 segments minus one gate at 180 and one at 0 (15° half-width covers
        // the two segments straddling each gate azimuth = 12 − 4).
        assert_eq!(cols, 12);
        assert!(walls < 12 && walls >= 8, "gates opened: {walls} walls");
        assert_eq!(
            m.placements
                .iter()
                .filter(|p| p.kind.as_deref() == Some("room-floor"))
                .count(),
            1
        );
        // The child rode along in the room's frame (translated by its centre).
        let ex = m.placements.iter().find(|p| p.name == "exhibit").unwrap();
        assert_eq!(ex.position, [0.0, 1.0, 0.0]);
        assert!(m.validate().is_ok());
    }

    #[test]
    fn room_children_translate_with_the_room() {
        let m = compile(
            r#"<world id="w" title="W">
                 <room at="10 0 -20" r="8">
                   <cube id="pedestal" at="0 0.5 0"/>
                   <portal to="thread://elsewhere" at="0 1.4 -6" label="On"/>
                 </room>
               </world>"#,
        )
        .unwrap();
        let p = m.placements.iter().find(|p| p.name == "pedestal").unwrap();
        assert_eq!(p.position, [10.0, 0.5, -20.0]);
        assert_eq!(m.portals[0].position, [10.0, 1.4, -26.0]);
    }

    #[test]
    fn wall_element_spans_two_points() {
        let m = compile(r#"<world id="w" title="W"><wall from="-5 0" to="5 0" h="2.5"/></world>"#)
            .unwrap();
        let w = m
            .placements
            .iter()
            .find(|p| p.kind.as_deref() == Some("wall"))
            .unwrap();
        assert_eq!(w.position, [0.0, 1.25, 0.0]);
        assert!((w.scale[0] - 10.0).abs() < 1e-4, "length {:?}", w.scale);
        assert_eq!(w.scale[1], 2.5);
    }

    #[test]
    fn text_attrs_carry_inline_hyperlinks() {
        let m = compile(
            r#"<world id="w" title="W">
                 <quad at="0 1 0" text="He founded [[thread://wiki.pixygon.io/wiki/kyuss|Kyuss]] and later [[thread://wiki.pixygon.io/wiki/qotsa]]."/>
               </world>"#,
        )
        .unwrap();
        let t = m.placements[0].text.as_ref().unwrap();
        assert_eq!(
            t.content,
            "He founded Kyuss and later thread://wiki.pixygon.io/wiki/qotsa."
        );
        assert_eq!(t.links.len(), 2);
        assert_eq!(t.links[0].text, "Kyuss");
        assert_eq!(t.links[0].to, "thread://wiki.pixygon.io/wiki/kyuss");
        assert_eq!(t.links[1].to, "thread://wiki.pixygon.io/wiki/qotsa");
    }

    #[test]
    fn weft_use_binds_a_package_export() {
        let m = compile(
            r#"<world id="w" title="W">
                 <cube id="clock" weft-use="weft-clock.weftpack.json#clock" weft-on="tick"/>
               </world>"#,
        )
        .unwrap();
        let b = &m.behaviors[0];
        assert!(b.weft.is_none());
        assert_eq!(b.weft_export.as_deref(), Some("clock"));
        assert_eq!(b.on, vec!["tick".to_string()]);
        let aid = b.weft_pack.as_ref().unwrap();
        let asset = m.assets.iter().find(|a| &a.id == aid).unwrap();
        assert_eq!(asset.uri, "weft-clock.weftpack.json");
        assert_eq!(m.placements[0].behavior.as_deref(), Some(b.id.as_str()));
    }

    #[test]
    fn light_attr_and_style_become_emitters() {
        let m = compile(
            r#"
            <world id="w" title="W">
              <sphere id="lamp" at="0 2 0" light="1 0.9 0.6 2 12"/>
              <sphere class="stage" at="3 2 0"/>
              <cube at="1 0 0"/>
            </world>
            <style>.stage { light: 0.5 0.85 1 }</style>
        "#,
        )
        .unwrap();
        let lamp = m.placements.iter().find(|p| p.name == "lamp").unwrap();
        let e = lamp.light.as_ref().expect("attr light");
        assert_eq!(e.color, [1.0, 0.9, 0.6]);
        assert_eq!(e.intensity, 2.0);
        assert_eq!(e.range, 12.0);
        let stage = m
            .placements
            .iter()
            .find(|p| p.class.iter().any(|c| c == "stage"))
            .unwrap();
        assert_eq!(stage.light.as_ref().unwrap().color, [0.5, 0.85, 1.0]);
        assert!(m
            .placements
            .iter()
            .find(|p| p.kind.as_deref() == Some("cube"))
            .unwrap()
            .light
            .is_none());
    }

    #[test]
    fn emissive_cascades_into_the_material() {
        let m = compile(
            r#"
            <world id="w" title="W"><cube class="lamp"/><cube/></world>
            <style>.lamp { color: 1 0.9 0.6; emissive: true }</style>
        "#,
        )
        .unwrap();
        let lamp = m
            .placements
            .iter()
            .find(|p| p.class.iter().any(|c| c == "lamp"))
            .unwrap();
        let mat = m
            .prefabs
            .iter()
            .find(|p| p.id == lamp.prefab)
            .unwrap()
            .material
            .as_ref()
            .unwrap();
        assert_eq!(mat.emissive, 1.0);
        // Emissive alone distinguishes the prefab from the plain cube.
        assert_eq!(m.prefabs.len(), 2);
    }
}
