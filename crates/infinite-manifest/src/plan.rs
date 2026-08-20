//! # Plan — a level design, before it knows what it is made of
//!
//! The output of a layout program, and the input to the binder. The whole
//! idea is in one sentence: **a plan never names a file.** It says *"here,
//! facing this way, I need a column about 5.2 m tall, classical, and it must
//! be load-bearing"* — and what satisfies that is a separate, cacheable
//! question, answered later by [the Quarry](https://quarry.pixygon.io) or by
//! commissioning a new model.
//!
//! That separation is what makes layout reproducible. The plan is computed
//! by a verified program from a brief and a seed, so the same brief rebuilds
//! the same place forever; the *assets* it resolves to may improve over time
//! without the design changing at all.
//!
//! Like [`crate::model`], a plan is a flat list of uniform records — the only
//! shape a non-recursive value language can express, and (again) the shape
//! that turned out to read better anyway: a plan is a bill of requirements
//! you can hand to someone.

use serde::{Deserialize, Serialize};

/// A laid-out place: its ground, what it needs, and how you get in and out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    #[serde(default = "untitled")]
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// The style and material family the whole place keeps to. Commissions
    /// inherit it; off-palette matches are allowed but never preferred.
    #[serde(default)]
    pub palette: Palette,
    /// Geometry the browser draws **directly** — ground, walls, copings,
    /// plinths. Cheap stone should not cost a model fetch: a wall segment is
    /// a box, and asking a store for one (or commissioning it!) would be
    /// silly. Only things worth *being* a model go in [`Plan::needs`].
    #[serde(default, alias = "regions")]
    pub builds: Vec<Build>,
    /// The bill of requirements: what goes where, described by what it must
    /// BE rather than which file it is.
    #[serde(default)]
    pub needs: Vec<Need>,
    /// Doorways out of this place.
    #[serde(default)]
    pub veils: Vec<Veil>,
    #[serde(default)]
    pub spawns: Vec<PlanSpawn>,
    #[serde(default)]
    pub lights: Vec<PlanLight>,
    /// Named boards — the place's own words.
    #[serde(default)]
    pub signs: Vec<Sign>,
}

fn untitled() -> String {
    "place".to_string()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Palette {
    /// `classical` · `rustic` · `industrial` …
    #[serde(default)]
    pub style: String,
    /// Material petnames the binder prefers for stone/wood/metal work.
    #[serde(default)]
    pub stone: String,
    #[serde(default)]
    pub wood: String,
    #[serde(default)]
    pub metal: String,
    /// Sky preset for the world this plan becomes.
    #[serde(default)]
    pub sky: String,
}

/// One piece of built geometry — the browser draws it from a primitive.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Build {
    /// `disc` (a round floor) · `slab` (a rectangular floor) · `box` (walls,
    /// copings, lintels, plinths) · `cylinder` (posts, drums).
    #[serde(default = "slab")]
    pub shape: String,
    #[serde(default)]
    pub at: [f32; 3],
    /// Facing, degrees — a wall segment is a box turned tangent to its ring.
    #[serde(default)]
    pub yaw: f32,
    /// Radius (disc / cylinder).
    #[serde(default)]
    pub r: f32,
    #[serde(default)]
    pub w: f32,
    #[serde(default)]
    pub d: f32,
    #[serde(default = "tenth")]
    pub h: f32,
    /// Which palette material: `stone` · `wood` · `metal` · `accent`, or a
    /// literal colour `"r g b"`.
    #[serde(default)]
    pub material: String,
    /// Blocks walking. Floors are solid; copings and trim usually are not.
    #[serde(default = "yes")]
    pub solid: bool,
    #[serde(default)]
    pub name: String,
}

fn slab() -> String {
    "slab".to_string()
}
fn tenth() -> f32 {
    0.1
}

/// One thing the place needs, described so a store can answer it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Need {
    /// What it is: `column` · `arch` · `stairs` · `table` · `rock` …
    pub kind: String,
    #[serde(default)]
    pub at: [f32; 3],
    /// Facing, degrees (the manifest's yaw convention).
    #[serde(default)]
    pub yaw: f32,
    /// Wanted size in metres; 0 means "don't care about this axis".
    #[serde(default)]
    pub w: f32,
    #[serde(default)]
    pub h: f32,
    #[serde(default)]
    pub d: f32,
    /// How far off the wanted size is still acceptable.
    #[serde(default = "quarter")]
    pub tol: f32,
    /// Style hint, else the plan's palette.
    #[serde(default)]
    pub style: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// If nothing fits, may the binder commission one? (Almost always yes;
    /// `false` means "skip it rather than invent it".)
    #[serde(default = "yes")]
    pub commission: bool,
    /// Does the place fail without it? Reported by the binder.
    #[serde(default = "yes")]
    pub must: bool,
    /// Blocks walking (the manifest's `solid`).
    #[serde(default = "yes")]
    pub solid: bool,
    #[serde(default)]
    pub name: String,
}

fn quarter() -> f32 {
    0.25
}
fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Veil {
    #[serde(default)]
    pub at: [f32; 3],
    #[serde(default)]
    pub yaw: f32,
    /// Locator; empty means "the brief will fill this in".
    #[serde(default)]
    pub to: String,
    #[serde(default)]
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanSpawn {
    #[serde(default)]
    pub at: [f32; 3],
    #[serde(default)]
    pub yaw: f32,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanLight {
    #[serde(default)]
    pub at: [f32; 3],
    #[serde(default = "one_f")]
    pub warm: f32,
    #[serde(default = "eight")]
    pub range: f32,
    #[serde(default = "one_f")]
    pub intensity: f32,
    /// Draw a lamp post under it (else the light just is).
    #[serde(default = "yes")]
    pub fixture: bool,
}

fn one_f() -> f32 {
    1.0
}
fn eight() -> f32 {
    8.0
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sign {
    #[serde(default)]
    pub at: [f32; 3],
    #[serde(default)]
    pub yaw: f32,
    #[serde(default)]
    pub text: String,
    #[serde(default = "two")]
    pub w: f32,
    #[serde(default = "one_and_a_half")]
    pub h: f32,
}

fn two() -> f32 {
    2.0
}
fn one_and_a_half() -> f32 {
    1.5
}

/// Metric discipline, named. A layout that ignores these reads wrong before
/// anyone can say why, so they live in one place and get referenced rather
/// than retyped.
pub mod metric {
    /// A door a person walks through without ducking or feeling lost.
    pub const DOOR_W: f32 = 1.1;
    pub const DOOR_H: f32 = 2.3;
    /// A corridor two people pass in.
    pub const CORRIDOR_W: f32 = 1.8;
    /// Ceilings read generous at about 1.6 × the door.
    pub const CEILING_MIN: f32 = DOOR_H * 1.6;
    /// A veil is 2 × 3 m and fires within this radius — a spawn any closer
    /// puts the visitor inside the doorway (world lint L3).
    pub const VEIL_CLEAR: f32 = 3.0;
    /// A plaza that feels like a room, not a field.
    pub const PLAZA_MIN: f32 = 12.0;
    pub const PLAZA_MAX: f32 = 20.0;
    /// Lamps this far apart leave no dark ground at range 8–9.
    pub const LAMP_SPACING: f32 = 7.0;
}

impl Plan {
    /// Check the things that make a layout walkable rather than merely
    /// present. Advisory like the world linter — findings name the fault.
    pub fn check(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.spawns.is_empty() {
            out.push("no spawn: nobody can arrive".into());
        }
        for s in &self.spawns {
            for v in &self.veils {
                let d = dist(s.at, v.at);
                if d < metric::VEIL_CLEAR {
                    out.push(format!(
                        "spawn '{}' is {d:.1} m from a veil — arrivals land inside the doorway (keep {:.1} m)",
                        s.name, metric::VEIL_CLEAR
                    ));
                }
            }
        }
        if !self
            .builds
            .iter()
            .any(|b| matches!(b.shape.as_str(), "disc" | "slab"))
        {
            out.push("no ground: the place has no floor".into());
        }
        if self.lights.is_empty() && self.palette.sky.is_empty() {
            out.push("no lights and no sky — this place renders dark".into());
        }
        // Needs that would sit inside each other.
        for (i, a) in self.needs.iter().enumerate() {
            for b in self.needs.iter().skip(i + 1) {
                let clearance = ((a.w.max(a.d) + b.w.max(b.d)) / 2.0) * 0.6;
                if a.solid && b.solid && dist(a.at, b.at) < clearance {
                    out.push(format!(
                        "'{}' and '{}' overlap at {:?}",
                        label(a, i),
                        label(b, i + 1),
                        a.at
                    ));
                }
            }
        }
        out
    }
}

fn label(n: &Need, i: usize) -> String {
    if n.name.is_empty() {
        format!("{} #{i}", n.kind)
    } else {
        n.name.clone()
    }
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plan_is_a_bill_of_requirements_and_checks_itself() {
        let p: Plan = serde_json::from_str(
            r#"{ "name": "hall",
                 "palette": { "style": "classical", "stone": "marble", "sky": "dusk" },
                 "builds": [ { "shape": "disc", "r": 12, "material": "stone" } ],
                 "needs": [
                   { "kind": "column", "at": [8,0,0], "h": 5.2, "w": 0.9, "d": 0.9 },
                   { "kind": "column", "at": [-8,0,0], "h": 5.2, "w": 0.9, "d": 0.9 }
                 ],
                 "spawns": [ { "at": [0,0,10] } ],
                 "veils": [ { "at": [0,1.4,-11], "label": "Onward" } ],
                 "lights": [ { "at": [0,3,0] } ] }"#,
        )
        .unwrap();
        assert_eq!(p.needs.len(), 2);
        assert!(p.needs[0].commission, "commissioning is the default");
        assert!(p.check().is_empty(), "a sound plan: {:?}", p.check());

        // A veil on top of the spawn, two columns in the same spot, no floor.
        let bad: Plan = serde_json::from_str(
            r#"{ "needs": [ { "kind": "column", "at": [0,0,0], "w": 1, "d": 1 },
                            { "kind": "column", "at": [0.2,0,0], "w": 1, "d": 1 } ],
                 "spawns": [ { "at": [0,0,0] } ],
                 "veils": [ { "at": [0,1.4,1] } ] }"#,
        )
        .unwrap();
        let found = bad.check();
        assert!(found.iter().any(|f| f.contains("doorway")), "{found:?}");
        assert!(found.iter().any(|f| f.contains("overlap")), "{found:?}");
        assert!(found.iter().any(|f| f.contains("floor")), "{found:?}");
        assert!(found.iter().any(|f| f.contains("dark")), "{found:?}");
    }
}
