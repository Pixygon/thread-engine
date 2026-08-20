//! # World lint — the quality eye that runs before any screenshot
//!
//! Conformance (C1–C6) answers "is this world *valid*?"; lint answers "will
//! this world *look and feel* right?" Every rule here is a bug class that
//! actually shipped and was caught by a human staring at a screenshot —
//! floating furniture, a spawn that walks straight into a veil, a text panel
//! spilling off its board, a portal buried in a wall. The linter is how an
//! agent authoring a world catches them before the first render.
//!
//! Geometry is approximated by axis-aligned boxes from `position`/`scale`
//! (rotation is ignored — findings are advisory, never build-breaking). Every
//! finding names its placement so the fix is one search away.

use crate::{Placement, WorldManifest};

/// One advisory finding. Lint never fails a build — it points.
#[derive(Debug, Clone)]
pub struct Finding {
    /// Stable rule id (`L1`…) for filtering and docs.
    pub rule: &'static str,
    /// What's wrong and where, human-first.
    pub message: String,
}

impl std::fmt::Display for Finding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.rule, self.message)
    }
}

/// A veil fires within this distance (mirrors the browser's enter radius);
/// spawning closer than this to a portal veilwalks the visitor instantly.
const PORTAL_ENTER_RADIUS: f32 = 2.2;

/// How much unsupported air under a placement's bottom counts as "floating".
const FLOAT_TOLERANCE: f32 = 0.25;

/// Run every rule over a manifest.
pub fn lint(m: &WorldManifest) -> Vec<Finding> {
    let mut out = Vec::new();
    // Carved prefabs know their true extents — use them instead of assuming
    // unit-sized geometry (a lathed 5m column at scale 1 is 5m tall).
    let shape_bounds: std::collections::HashMap<u32, ([f32; 3], [f32; 3])> = m
        .prefabs
        .iter()
        .filter_map(|p| p.mesh.shape.as_ref().map(|sh| (p.id.0, sh.bounds())))
        .collect();
    let boxes: Vec<Aabb> = m
        .placements
        .iter()
        .map(|p| Aabb::of(p, shape_bounds.get(&p.prefab.0)))
        .collect();
    floating_placements(m, &boxes, &mut out);
    spawn_hazards(m, &boxes, &mut out);
    portal_hazards(m, &boxes, &mut out);
    text_overflow(m, &mut out);
    darkness(m, &mut out);
    degenerate_scales(m, &mut out);
    doubled_placements(m, &mut out);
    presence_claim(m, &mut out);
    out
}

/// L9 — `presence.mode` claims a tier the addresses don't support. The spec
/// says the addresses win, so this never breaks a world; it exists because the
/// disagreement is always a mistake in one of the two, and the author is the
/// only one who knows which.
fn presence_claim(m: &WorldManifest, out: &mut Vec<Finding>) {
    let Some(p) = &m.presence else { return };
    if p.mode_disagrees() {
        out.push(Finding {
            rule: "L9",
            message: format!(
                "presence says mode '{}' but names {} — a browser will treat this world as '{}'",
                p.mode.as_deref().unwrap_or_default(),
                if p.relay_list().is_empty() {
                    "no relay".to_string()
                } else {
                    format!("{} relay(s)", p.relay_list().len())
                },
                p.effective_mode()
            ),
        });
    }
}

/// Axis-aligned box approximation of a placement (rotation ignored).
#[derive(Clone, Copy)]
struct Aabb {
    min: [f32; 3],
    max: [f32; 3],
    solid: bool,
    /// Whether the box is (near-)axis-aligned — containment tests are only
    /// trustworthy when it is.
    axis_aligned: bool,
}

impl Aabb {
    fn of(p: &Placement, shape_bounds: Option<&([f32; 3], [f32; 3])>) -> Aabb {
        // Unit-primitive assumption (centre-origin, extent = scale) — replaced
        // by the recipe's true bounds for carved prefabs.
        if let Some((bmin, bmax)) = shape_bounds {
            let lo = [
                p.position[0] + bmin[0] * p.scale[0],
                p.position[1] + bmin[1] * p.scale[1],
                p.position[2] + bmin[2] * p.scale[2],
            ];
            let hi = [
                p.position[0] + bmax[0] * p.scale[0],
                p.position[1] + bmax[1] * p.scale[1],
                p.position[2] + bmax[2] * p.scale[2],
            ];
            let [qx, qy, qz, qw] = p.rotation;
            let upright =
                qx.abs() < 0.02 && qz.abs() < 0.02 && (qy.abs() < 0.04 || qw.abs() < 0.04);
            return Aabb {
                min: lo,
                max: hi,
                solid: p.solid.unwrap_or(true),
                axis_aligned: upright,
            };
        }
        let half = [
            p.scale[0].abs() / 2.0,
            p.scale[1].abs() / 2.0,
            p.scale[2].abs() / 2.0,
        ];
        // A meaningfully rotated box is NOT its AABB — containment checks on
        // it would cry wolf (a ring's rotated wall segments "swallowing" the
        // veils in their gates). Mark it so point-in-box rules skip it.
        let [qx, qy, qz, qw] = p.rotation;
        let upright = qx.abs() < 0.02 && qz.abs() < 0.02 && (qy.abs() < 0.04 || qw.abs() < 0.04);
        Aabb {
            min: [
                p.position[0] - half[0],
                p.position[1] - half[1],
                p.position[2] - half[2],
            ],
            max: [
                p.position[0] + half[0],
                p.position[1] + half[1],
                p.position[2] + half[2],
            ],
            solid: p.solid.unwrap_or(true),
            axis_aligned: upright,
        }
    }

    fn footprint_overlaps(&self, other: &Aabb) -> bool {
        self.min[0] < other.max[0]
            && self.max[0] > other.min[0]
            && self.min[2] < other.max[2]
            && self.max[2] > other.min[2]
    }

    fn contains(&self, p: [f32; 3]) -> bool {
        (0..3).all(|i| p[i] > self.min[i] && p[i] < self.max[i])
    }
}

fn name_of(p: &Placement, i: usize) -> String {
    if !p.name.is_empty() {
        format!("'{}'", p.name)
    } else if let Some(k) = &p.kind {
        format!("<{k}> #{i}")
    } else {
        format!("placement #{i} (prefab {})", p.prefab.0)
    }
}

/// L1 — a solid placement hanging in the air with nothing under it. Text
/// panels, lights-only pieces and non-solids are exempt (they hover by
/// design); anything at ground contact (bottom ≈ 0) is grounded by the floor.
fn floating_placements(m: &WorldManifest, boxes: &[Aabb], out: &mut Vec<Finding>) {
    for (i, p) in m.placements.iter().enumerate() {
        if p.solid == Some(false) || p.text.is_some() || p.light.is_some() {
            continue;
        }
        let b = &boxes[i];
        if !b.axis_aligned {
            continue; // a rotated box is not its AABB — don't cry wolf
        }
        if b.min[1] <= FLOAT_TOLERANCE {
            continue; // resting on (or in) the ground plane
        }
        let supported = boxes.iter().enumerate().any(|(j, other)| {
            j != i
                && other.footprint_overlaps(b)
                && other.max[1] >= b.min[1] - FLOAT_TOLERANCE
                && other.min[1] < b.min[1] + FLOAT_TOLERANCE
        });
        // A child placement is positioned relative to its parent — skip those
        // (they were already lifted into their parent's frame).
        if !supported {
            out.push(Finding {
                rule: "L1",
                message: format!(
                    "{} floats {:.2}m above ground with nothing under it (at {:?})",
                    name_of(p, i),
                    b.min[1],
                    p.position
                ),
            });
        }
    }
}

/// L2 — a spawn buried inside solid geometry, or with no ground beneath it.
fn spawn_hazards(m: &WorldManifest, boxes: &[Aabb], out: &mut Vec<Finding>) {
    for s in &m.spawns {
        let eye = [s.position[0], s.position[1] + 1.0, s.position[2]];
        for (i, b) in boxes.iter().enumerate() {
            if b.solid && b.axis_aligned && b.contains(eye) {
                out.push(Finding {
                    rule: "L2",
                    message: format!(
                        "spawn '{}' is inside solid {} — visitors arrive buried",
                        s.name,
                        name_of(&m.placements[i], i)
                    ),
                });
            }
        }
    }
}

/// L3 — portal hazards: a veil so close to a spawn that arrival instantly
/// fires it (the walk-in bug), or a veil buried inside solid geometry.
fn portal_hazards(m: &WorldManifest, boxes: &[Aabb], out: &mut Vec<Finding>) {
    for portal in &m.portals {
        for s in &m.spawns {
            let d = dist(portal.position, s.position);
            if d < PORTAL_ENTER_RADIUS + 0.3 {
                out.push(Finding {
                    rule: "L3",
                    message: format!(
                        "veil '{}' is {:.1}m from spawn '{}' — inside the trigger radius: visitors arrive standing IN the doorway (it can't fire until they step away and return). Keep veils > {:.1}m from spawns",
                        portal.id, d, s.name, PORTAL_ENTER_RADIUS
                    ),
                });
            }
        }
        for (i, b) in boxes.iter().enumerate() {
            if b.solid && b.axis_aligned && b.contains(portal.position) {
                out.push(Finding {
                    rule: "L3",
                    message: format!(
                        "veil '{}' sits inside solid {} — the doorway is walled off",
                        portal.id,
                        name_of(&m.placements[i], i)
                    ),
                });
            }
        }
    }
}

/// L4 — text that will not fit its panel. Mirrors the SDF layout's real
/// numbers (line height ≈ 1.4 × the size fraction, 0.8-glyph margins): copy
/// past capacity is clipped at the paper's edge — better to know now.
fn text_overflow(m: &WorldManifest, out: &mut Vec<Finding>) {
    for (i, p) in m.placements.iter().enumerate() {
        let Some(t) = &p.text else { continue };
        let lines = t.content.lines().count() as f32;
        let capacity = (1.0 - 1.6 * t.size) / (1.4 * t.size);
        if lines > capacity.max(1.0) {
            out.push(Finding {
                rule: "L4",
                message: format!(
                    "{}'s text is {} lines but the panel fits ~{:.0} at size {} — shrink the size or trim the copy",
                    name_of(p, i),
                    lines as usize,
                    capacity,
                    t.size
                ),
            });
        }
    }
}

/// L5 — a world with no light source at all: no sky, no placement lights, and
/// nothing emissive renders as a black hole.
fn darkness(m: &WorldManifest, out: &mut Vec<Finding>) {
    let has_sky = m.environment.sky.is_some();
    let has_light = m.placements.iter().any(|p| p.light.is_some());
    let has_emissive = m
        .prefabs
        .iter()
        .any(|pf| pf.material.as_ref().is_some_and(|mat| mat.emissive > 0.0));
    if !has_sky && !has_light && !has_emissive {
        out.push(Finding {
            rule: "L5",
            message: "no sky, no lights, nothing emissive — this world renders dark".to_string(),
        });
    }
}

/// L6 — zero, negative, or absurd scales (a 0-thin wall flickers; a 900 m cube
/// is almost always a typo'd unit).
fn degenerate_scales(m: &WorldManifest, out: &mut Vec<Finding>) {
    for (i, p) in m.placements.iter().enumerate() {
        if p.scale.iter().any(|s| *s <= 0.0) {
            out.push(Finding {
                rule: "L6",
                message: format!("{} has a zero/negative scale {:?}", name_of(p, i), p.scale),
            });
        } else if p.scale.iter().any(|s| *s > 500.0) {
            out.push(Finding {
                rule: "L6",
                message: format!(
                    "{} is over 500m on a side ({:?}) — metres, not centimetres?",
                    name_of(p, i),
                    p.scale
                ),
            });
        }
    }
}

/// L8 — two placements at the identical position (a copy-paste double: they
/// z-fight and double colliders).
fn doubled_placements(m: &WorldManifest, out: &mut Vec<Finding>) {
    for i in 0..m.placements.len() {
        for j in (i + 1)..m.placements.len() {
            let (a, b) = (&m.placements[i], &m.placements[j]);
            if a.prefab == b.prefab && a.position == b.position && a.scale == b.scale {
                out.push(Finding {
                    rule: "L8",
                    message: format!(
                        "{} and {} are identical twins at {:?} — one is probably a paste",
                        name_of(a, i),
                        name_of(b, j),
                        a.position
                    ),
                });
            }
        }
    }
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::markup;

    fn world(src: &str) -> WorldManifest {
        markup::compile(src).unwrap()
    }

    #[test]
    fn a_clean_world_lints_clean() {
        let m = world(
            r#"<world id="w" title="W" sky="dusk">
                 <spawn at="0 0 8"/>
                 <plane at="0 0 0" scale="30 1 30"/>
                 <cube at="3 0.5 0" scale="1 1 1"/>
                 <portal at="0 1.4 -6" to="thread://pixygon.io" label="Out"/>
               </world>"#,
        );
        let findings = lint(&m);
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn floating_buried_and_walkin_are_caught() {
        let m = world(
            r#"<world id="w" title="W" sky="dusk">
                 <spawn at="0 0 -5"/>
                 <plane at="0 0 0" scale="30 1 30"/>
                 <cube id="hover" at="5 4 5" scale="1 1 1"/>
                 <cube id="tomb" at="0 1 -5" scale="4 4 4"/>
                 <portal id="door" at="0 1.4 -6" to="thread://pixygon.io" label="Out"/>
               </world>"#,
        );
        let f = lint(&m);
        let rules: Vec<&str> = f.iter().map(|x| x.rule).collect();
        assert!(rules.contains(&"L1"), "hovering cube: {f:?}");
        assert!(rules.contains(&"L2"), "buried spawn: {f:?}");
        assert!(rules.contains(&"L3"), "walk-in veil: {f:?}");
    }

    #[test]
    fn supported_stacks_do_not_flag() {
        let m = world(
            r#"<world id="w" title="W" sky="dusk">
                 <spawn at="0 0 8"/>
                 <plane at="0 0 0" scale="30 1 30"/>
                 <cube id="plinth" at="0 0.5 0" scale="1 1 1"/>
                 <cube id="statue" at="0 1.5 0" scale="0.8 1 0.8"/>
               </world>"#,
        );
        let f: Vec<_> = lint(&m).into_iter().filter(|x| x.rule == "L1").collect();
        assert!(f.is_empty(), "stacked statue is supported: {f:?}");
    }

    #[test]
    fn overflow_darkness_scale_and_twins_are_caught() {
        let mut m = world(
            r#"<world id="w" title="W">
                 <spawn at="0 0 8"/>
                 <plane at="0 0 0" scale="30 1 30"/>
                 <cube at="4 0.5 4"/>
                 <cube at="4 0.5 4"/>
                 <cube id="mega" at="9 0.5 0" scale="900 1 1"/>
               </world>"#,
        );
        m.placements[0].text = Some(crate::TextPanel {
            content: (0..40)
                .map(|i| format!("line {i}"))
                .collect::<Vec<_>>()
                .join("\n"),
            size: 0.05,
            color: [0.0; 3],
            background: [1.0; 3],
            links: vec![],
        });
        let f = lint(&m);
        let rules: Vec<&str> = f.iter().map(|x| x.rule).collect();
        assert!(rules.contains(&"L4"), "overflowing text: {f:?}");
        assert!(rules.contains(&"L5"), "dark world: {f:?}");
        assert!(rules.contains(&"L6"), "typo scale: {f:?}");
        assert!(rules.contains(&"L8"), "twins: {f:?}");
    }

    /// L9. The world stays valid either way — this fires so the author finds
    /// out which of the two lines they got wrong.
    #[test]
    fn a_presence_claim_the_addresses_cannot_keep_is_reported() {
        let world = |presence: &str| -> WorldManifest {
            serde_json::from_str(&format!(
                r#"{{ "thread": "thread/0.1", "world": {{ "id": "w", "title": "W" }},
                      "spawns": [{{ "name": "e", "position": [0,0,0] }}],
                      "placements": [{{ "prefab": 60930001, "position": [0,0,0], "scale": [1,1,1],
                                        "light": {{ "color": [1,1,1], "intensity": 1, "range": 8 }} }}],
                      "presence": {presence} }}"#
            ))
            .expect("manifest")
        };
        let claims_a_relay_it_hasnt_got = lint(&world(r#"{ "mode": "relay" }"#));
        assert!(
            claims_a_relay_it_hasnt_got.iter().any(|f| f.rule == "L9"),
            "{claims_a_relay_it_hasnt_got:?}"
        );
        assert!(lint(&world(r#"{ "mode": "solo" }"#))
            .iter()
            .all(|f| f.rule != "L9"));
        assert!(
            lint(&world(r#"{ "mode": "relay", "relays": ["wss://a"] }"#))
                .iter()
                .all(|f| f.rule != "L9")
        );
        assert!(lint(&world(r#"{ "relays": ["wss://a"] }"#))
            .iter()
            .all(|f| f.rule != "L9"));
    }
}
