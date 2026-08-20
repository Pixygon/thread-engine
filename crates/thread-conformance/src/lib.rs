//! # The Thread conformance suite
//!
//! A standard is only real if it can be checked *without* the reference browser.
//! This crate is that check: point it at a corpus of worlds (a directory of
//! `world.json`s) and it reports, browser-independently, whether they honour the
//! spec — manifests validate, worlds are enterable, links are well-formed, and the
//! constellation hangs together. It's the artifact that lets a third party prove
//! their worlds (or their own engine's output) conform, with zero contact.
//!
//! Clauses are either **Error** (a real conformance violation — fails the suite)
//! or **Warn** (a quality signal that doesn't break interop — reported, but the
//! suite still passes). [`run`] returns a [`Report`]; [`Report::passed`] is true
//! when no Error clause failed.

use std::path::{Path, PathBuf};

use infinite_manifest::{Locator, StructuredId, WorldManifest};

pub mod relay;
pub mod rendezvous;

/// Whether a failed clause breaks conformance or is merely a quality signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// A real spec violation — fails the suite.
    Error,
    /// A quality signal — reported, but interop still holds.
    Warn,
}

/// The outcome of one conformance clause.
#[derive(Debug, Clone)]
pub struct Clause {
    pub name: &'static str,
    pub severity: Severity,
    pub pass: bool,
    /// Human-readable specifics (offending worlds, external links, orphans…).
    pub notes: Vec<String>,
}

/// The full conformance report over a corpus.
#[derive(Debug, Clone)]
pub struct Report {
    pub clauses: Vec<Clause>,
    /// How many worlds loaded and validated.
    pub worlds: usize,
}

impl Report {
    /// The suite passes when no **Error**-severity clause failed. Failed **Warn**
    /// clauses are reported but do not break conformance.
    pub fn passed(&self) -> bool {
        self.clauses
            .iter()
            .all(|c| c.pass || c.severity == Severity::Warn)
    }
}

/// A world that loaded and validated.
pub struct World {
    /// The world's key in the corpus — its directory path under the root, which is
    /// exactly what a Locator's path (or host) resolves to.
    pub name: String,
    pub manifest: WorldManifest,
    /// Where it was loaded from, when it came off a disk. `None` for a world
    /// fetched over the wire — the clauses that read the tree skip those rather
    /// than guess at what a remote host has beside its manifest.
    pub dir: Option<PathBuf>,
}

/// A world that failed to load or validate.
pub struct LoadError {
    pub name: String,
    pub error: String,
}

/// A loaded corpus: the worlds that validated, plus the ones that didn't.
pub struct Corpus {
    pub worlds: Vec<World>,
    pub load_errors: Vec<LoadError>,
}

/// Load every world under `root` (one per immediate subdirectory — a
/// `world.json`, or a `world.thread` markup source when only that exists),
/// parsing and validating each through the reference manifest implementation.
pub fn load_corpus(root: &Path) -> Corpus {
    let mut worlds = Vec::new();
    let mut load_errors = Vec::new();

    let mut entries: Vec<PathBuf> = match std::fs::read_dir(root) {
        Ok(rd) => rd.flatten().map(|e| e.path()).collect(),
        Err(e) => {
            load_errors.push(LoadError {
                name: root.display().to_string(),
                error: e.to_string(),
            });
            return Corpus {
                worlds,
                load_errors,
            };
        }
    };
    entries.sort();

    for dir in entries {
        let mut wj = dir.join("world.json");
        if !wj.exists() {
            wj = dir.join("world.thread");
            if !wj.exists() {
                continue;
            }
        }
        let name = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        match std::fs::read_to_string(&wj) {
            Ok(text) => match WorldManifest::from_text(&text) {
                Ok(manifest) => {
                    worlds.push(World { name, manifest, dir: Some(dir.clone()) })
                }
                Err(e) => load_errors.push(LoadError {
                    name,
                    error: e.to_string(),
                }),
            },
            Err(e) => load_errors.push(LoadError {
                name,
                error: e.to_string(),
            }),
        }
    }
    Corpus {
        worlds,
        load_errors,
    }
}

/// Build a one-world corpus from a single manifest's text (the live-host path:
/// fetch a `thread://host` and run the spec clauses on what it served). `name` is
/// the world's corpus key (its Locator path/host).
pub fn single_corpus(name: &str, manifest_text: &str) -> Corpus {
    match WorldManifest::from_text(manifest_text) {
        Ok(manifest) => Corpus {
            worlds: vec![World {
                dir: None,
                name: name.to_string(),
                manifest,
            }],
            load_errors: Vec::new(),
        },
        Err(e) => Corpus {
            worlds: Vec::new(),
            load_errors: vec![LoadError {
                name: name.to_string(),
                error: e.to_string(),
            }],
        },
    }
}

/// The transport contract a live host must satisfy so *any* browser — including a
/// cross-origin web one — can walk it. Pure over the response facts, so it's
/// testable without a network. `reachable` is whether the GET completed with a 2xx.
pub fn transport_clauses(
    reachable: bool,
    status: u16,
    content_type: &str,
    cors: &str,
) -> Vec<Clause> {
    vec![
        Clause {
            name: "reachable over HTTPS",
            severity: Severity::Error,
            pass: reachable,
            notes: if reachable {
                vec![]
            } else {
                vec![format!("HTTP {status}")]
            },
        },
        Clause {
            name: "serves any-origin CORS",
            severity: Severity::Error,
            pass: cors == "*",
            notes: if cors == "*" {
                vec![]
            } else if cors.is_empty() {
                vec!["missing Access-Control-Allow-Origin (a web browser can't fetch it)".into()]
            } else {
                vec![format!("Access-Control-Allow-Origin: {cors} (not '*')")]
            },
        },
        Clause {
            name: "declares JSON content-type",
            severity: Severity::Warn,
            pass: content_type.contains("json"),
            notes: if content_type.contains("json") {
                vec![]
            } else {
                vec![format!(
                    "Content-Type: '{content_type}' (expected application/json)"
                )]
            },
        },
    ]
}

/// Whether a slice of clauses passes (no failed Error clause).
pub fn clauses_pass(clauses: &[Clause]) -> bool {
    clauses
        .iter()
        .all(|c| c.pass || c.severity == Severity::Warn)
}

/// The corpus key a Locator resolves to — its path, or its host when path-less
/// (mirrors the local resolver: `thread://host/path` → `<root>/<path>`).
fn dest_key(loc: &Locator) -> &str {
    if loc.path.is_empty() {
        &loc.host
    } else {
        &loc.path
    }
}

/// Run the full suite over a loaded corpus.
pub fn run(corpus: &Corpus) -> Report {
    let mut clauses = Vec::new();
    let names: std::collections::HashSet<&str> =
        corpus.worlds.iter().map(|w| w.name.as_str()).collect();

    // C1 — every world manifest validates. (Error)
    clauses.push(Clause {
        name: "world manifests validate",
        severity: Severity::Error,
        pass: corpus.load_errors.is_empty(),
        notes: corpus
            .load_errors
            .iter()
            .map(|e| format!("{}: {}", e.name, e.error))
            .collect(),
    });

    // C2 — every world declares at least one spawn (an arrival point). (Error)
    let no_spawn: Vec<String> = corpus
        .worlds
        .iter()
        .filter(|w| w.manifest.spawns.is_empty())
        .map(|w| w.name.clone())
        .collect();
    clauses.push(Clause {
        name: "worlds declare a spawn",
        severity: Severity::Error,
        pass: no_spawn.is_empty(),
        notes: no_spawn.iter().map(|n| format!("{n}: no spawns")).collect(),
    });

    // C3 — every portal destination is a well-formed Locator. (Error)
    let mut bad_locators = Vec::new();
    for w in &corpus.worlds {
        for p in &w.manifest.portals {
            if Locator::parse(&p.to).is_none() {
                bad_locators.push(format!("{}: portal '{}' → '{}'", w.name, p.id, p.to));
            }
        }
    }
    clauses.push(Clause {
        name: "portal destinations are valid Locators",
        severity: Severity::Error,
        pass: bad_locators.is_empty(),
        notes: bad_locators,
    });

    // C4 — the constellation is connected: from an entry world, every other world
    // is reachable via internal veils. External links (to worlds hosted elsewhere)
    // are fine and simply aren't traversed. Orphans are a quality signal. (Warn)
    let (entry, orphans, internal, external) = analyze_graph(&corpus.worlds, &names);
    let mut notes = vec![format!(
        "entry '{}' · {} internal link(s) · {} external link(s)",
        entry.as_deref().unwrap_or("—"),
        internal,
        external
    )];
    notes.extend(
        orphans
            .iter()
            .map(|o| format!("unreachable from entry: {o}")),
    );
    clauses.push(Clause {
        name: "constellation is connected",
        severity: Severity::Warn,
        pass: orphans.is_empty(),
        notes,
    });

    // C5 — veils carry a human label (so a browser can name the doorway). (Warn)
    let mut unlabeled = Vec::new();
    for w in &corpus.worlds {
        for p in &w.manifest.portals {
            if p.label.trim().is_empty() {
                unlabeled.push(format!("{}: portal '{}' has no label", w.name, p.id));
            }
        }
    }
    clauses.push(Clause {
        name: "veils carry labels",
        severity: Severity::Warn,
        pass: unlabeled.is_empty(),
        notes: unlabeled,
    });

    // C8 — a world's own files are actually there. A **relative** asset URI
    // names something the author was supposed to ship beside the manifest; if
    // it is missing the room loads perfectly and is missing its pillars, with
    // nothing to complain about, because the manifest alone is still valid.
    // (That is not hypothetical: a publisher that staged only the top level of
    // a room directory would have put exactly that on a live domain.)
    //
    // Error, not Warn — unlike a dead veil (C6), which links out to a host that
    // may come back and is the web's normal weather, this is content inside the
    // author's own tree, under their control, missing every single time the
    // world loads. Absolute URIs are links out and are not checked here.
    //
    // **`wasm` is exempt**, and not as a convenience: behaviour-abi §5 says a
    // browser MAY ignore a behavior it cannot sandbox, so a world whose module
    // is absent degrades to exactly what the spec promises — the room is whole,
    // the pedestal simply doesn't do its trick. A missing mesh has no such
    // clause behind it and leaves a hole where the columns were.
    let mut missing = Vec::new();
    for w in &corpus.worlds {
        let Some(dir) = &w.dir else { continue };
        for a in &w.manifest.assets {
            let uri = a.uri.trim();
            if uri.is_empty() || uri.contains("://") || uri.starts_with("//") {
                continue;
            }
            if matches!(a.kind, infinite_manifest::AssetKind::Wasm) {
                continue;
            }
            let rel = uri.split(['?', '#']).next().unwrap_or(uri);
            if !dir.join(rel).exists() {
                missing.push(format!("{}: asset '{}' → {rel} is not there", w.name, a.id));
            }
        }
    }
    clauses.push(Clause {
        name: "declared files are present",
        severity: Severity::Error,
        pass: missing.is_empty(),
        notes: missing,
    });

    // C7 — geometry rests on the ground rather than sinking through it. A
    // centre-origin primitive whose `position.y` is less than half its
    // `scale.y` has its lower half below y = 0: the table is through the
    // floor, the pillar is buried to the knee. Nobody files that as a bug —
    // the room merely looks slightly strange — which is exactly why it wants
    // a clause. (Warn: deliberately half-buried geometry is legitimate.)
    //
    // The check is deliberately narrow. It applies ONLY to the builtins whose
    // origin is their centre and whose extent is exactly `scale`: `cube`,
    // `sphere`, `cylinder`, `capsule`. Not `plane` (flat, its y-scale means
    // nothing), not carved shapes or glTF assets — a model from a store is
    // usually **base**-origin, so the same arithmetic would condemn every
    // correctly-placed one of them.
    //
    // The capsule was excluded here until 0.2.1, because it was three units
    // tall by construction and so its `scale` did not mean what every other
    // primitive's did. It is one metre now, which removed the exception rather
    // than documenting it.
    let mut sunk = Vec::new();
    for w in &corpus.worlds {
        let centre_origin: std::collections::HashSet<StructuredId> = w
            .manifest
            .prefabs
            .iter()
            .filter(|p| {
                matches!(
                    p.mesh.builtin.as_deref(),
                    Some("cube" | "sphere" | "cylinder" | "capsule")
                )
            })
            .map(|p| p.id)
            .collect();
        for pl in &w.manifest.placements {
            if !centre_origin.contains(&pl.prefab) {
                continue;
            }
            // Panels and lights hover by design; so does anything the author
            // marked non-solid (trim, banners, decals).
            if pl.solid == Some(false) || pl.text.is_some() || pl.light.is_some() {
                continue;
            }
            let bottom = pl.position[1] - pl.scale[1].abs() / 2.0;
            if bottom < -GROUND_TOLERANCE {
                sunk.push(format!(
                    "{}: '{}' sits {:.2} m below the ground plane",
                    w.name,
                    if pl.name.is_empty() {
                        "(unnamed)"
                    } else {
                        &pl.name
                    },
                    -bottom
                ));
            }
        }
    }
    let shown: Vec<String> = sunk.iter().take(12).cloned().collect();
    let mut notes = shown;
    if sunk.len() > notes.len() {
        notes.push(format!("… and {} more", sunk.len() - notes.len()));
    }
    clauses.push(Clause {
        name: "geometry rests on the ground plane",
        severity: Severity::Warn,
        pass: sunk.is_empty(),
        notes,
    });

    Report {
        clauses,
        worlds: corpus.worlds.len(),
    }
}

/// How far a piece may sit below y = 0 before it counts as sunk. Five
/// centimetres: enough for a floor slab bedded into the ground, far short of a
/// table through it.
const GROUND_TOLERANCE: f32 = 0.05;

/// Pick an entry world, BFS the internal veil graph, and return
/// `(entry, orphans, internal_link_count, external_link_count)`.
fn analyze_graph(
    worlds: &[World],
    names: &std::collections::HashSet<&str>,
) -> (Option<String>, Vec<String>, usize, usize) {
    if worlds.is_empty() {
        return (None, Vec::new(), 0, 0);
    }

    // Entry: prefer a world named "nexus"; else the hub with the most veils.
    let entry = worlds
        .iter()
        .find(|w| w.name == "nexus")
        .or_else(|| worlds.iter().max_by_key(|w| w.manifest.portals.len()))
        .map(|w| w.name.clone());

    // Adjacency over internal links; tally internal vs external.
    let mut internal = 0usize;
    let mut external = 0usize;
    let mut adj: std::collections::HashMap<&str, Vec<String>> = std::collections::HashMap::new();
    for w in worlds {
        for p in &w.manifest.portals {
            match Locator::parse(&p.to) {
                Some(loc) if names.contains(dest_key(&loc)) => {
                    internal += 1;
                    adj.entry(w.name.as_str())
                        .or_default()
                        .push(dest_key(&loc).to_string());
                }
                _ => external += 1,
            }
        }
    }

    // BFS from the entry.
    let mut seen = std::collections::HashSet::new();
    if let Some(start) = &entry {
        let mut queue = vec![start.clone()];
        seen.insert(start.clone());
        while let Some(cur) = queue.pop() {
            if let Some(nexts) = adj.get(cur.as_str()) {
                for n in nexts {
                    if seen.insert(n.clone()) {
                        queue.push(n.clone());
                    }
                }
            }
        }
    }

    let mut orphans: Vec<String> = worlds
        .iter()
        .map(|w| w.name.clone())
        .filter(|n| !seen.contains(n))
        .collect();
    orphans.sort();
    (entry, orphans, internal, external)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corpus_from(worlds: Vec<(&str, WorldManifest)>) -> Corpus {
        Corpus {
            worlds: worlds
                .into_iter()
                .map(|(n, m)| World {
                    name: n.into(),
                    manifest: m,
                    dir: None,
                })
                .collect(),
            load_errors: Vec::new(),
        }
    }

    fn world(id: &str, spawns: bool, portals: &[(&str, &str, &str)]) -> WorldManifest {
        let spawn_json = if spawns {
            r#""spawns": [{ "name": "entry", "position": [0,0,0] }],"#
        } else {
            ""
        };
        let portal_json: Vec<String> = portals
            .iter()
            .map(|(pid, to, label)| {
                format!(
                    r#"{{ "id": "{pid}", "position": [0,0,0], "to": "{to}", "label": "{label}" }}"#
                )
            })
            .collect();
        let text = format!(
            r#"{{ "thread": "thread/0.1", "world": {{ "id": "{id}", "title": "{id}" }},
                {spawn_json} "portals": [{}] }}"#,
            portal_json.join(",")
        );
        WorldManifest::from_json(&text).unwrap()
    }

    #[test]
    fn a_connected_labeled_corpus_passes_cleanly() {
        let c = corpus_from(vec![
            (
                "nexus",
                world("nexus", true, &[("to-a", "thread://a.io/alpha", "Alpha")]),
            ),
            (
                "alpha",
                world("alpha", true, &[("to-n", "thread://x.io/nexus", "Nexus")]),
            ),
        ]);
        let r = run(&c);
        assert!(r.passed(), "no error clauses should fail");
        assert!(
            r.clauses.iter().all(|cl| cl.pass),
            "and no warnings either: {:?}",
            r.clauses
        );
    }

    #[test]
    fn a_missing_spawn_is_an_error_and_fails_the_suite() {
        let c = corpus_from(vec![("alpha", world("alpha", false, &[]))]);
        let r = run(&c);
        assert!(!r.passed());
        let spawn_clause = r
            .clauses
            .iter()
            .find(|c| c.name == "worlds declare a spawn")
            .unwrap();
        assert!(!spawn_clause.pass);
        assert_eq!(spawn_clause.severity, Severity::Error);
    }

    #[test]
    fn an_orphan_world_warns_but_still_passes() {
        // 'lonely' is in the corpus but nothing links to it.
        let c = corpus_from(vec![
            (
                "nexus",
                world("nexus", true, &[("to-a", "thread://a.io/alpha", "Alpha")]),
            ),
            ("alpha", world("alpha", true, &[])),
            ("lonely", world("lonely", true, &[])),
        ]);
        let r = run(&c);
        assert!(
            r.passed(),
            "an orphan is a warning, not a conformance failure"
        );
        let conn = r
            .clauses
            .iter()
            .find(|c| c.name == "constellation is connected")
            .unwrap();
        assert!(!conn.pass);
        assert_eq!(conn.severity, Severity::Warn);
        assert!(conn.notes.iter().any(|n| n.contains("lonely")));
    }

    /// A corpus sitting beside this crate must have zero **Error**-severity
    /// failures. In the reference implementation's own tree that is the hosted
    /// constellation, and this is the suite guarding it; published on its own,
    /// the crate travels without those worlds, so the test reports that it had
    /// nothing to check rather than failing for the absence of files that were
    /// never its own. (Warn clauses like orphan worlds are allowed; this
    /// asserts real conformance, not full connectivity.)
    #[test]
    fn the_shipped_worlds_corpus_is_conformant() {
        let root = format!("{}/../../worlds", env!("CARGO_MANIFEST_DIR"));
        let root = std::path::Path::new(&root);
        if !root.is_dir() {
            eprintln!("no corpus beside this crate — nothing to guard here");
            return;
        }
        let corpus = load_corpus(root);
        let report = run(&corpus);
        assert!(
            report.worlds >= 8,
            "a corpus is present but nearly empty — found {}",
            report.worlds
        );
        for c in &report.clauses {
            if c.severity == Severity::Error {
                assert!(c.pass, "error clause '{}' failed: {:?}", c.name, c.notes);
            }
        }
        assert!(report.passed());
    }

    #[test]
    fn transport_contract_flags_missing_cors_as_error() {
        let ok = transport_clauses(true, 200, "application/json", "*");
        assert!(clauses_pass(&ok));

        let no_cors = transport_clauses(true, 200, "application/json", "");
        assert!(
            !clauses_pass(&no_cors),
            "missing CORS breaks cross-origin browsers"
        );
        let cors_clause = no_cors
            .iter()
            .find(|c| c.name == "serves any-origin CORS")
            .unwrap();
        assert_eq!(cors_clause.severity, Severity::Error);

        // Wrong content-type is only a warning — the manifest still parses.
        let text_plain = transport_clauses(true, 200, "text/plain", "*");
        assert!(clauses_pass(&text_plain));
        assert!(
            !text_plain
                .iter()
                .find(|c| c.name == "declares JSON content-type")
                .unwrap()
                .pass
        );
    }

    #[test]
    fn single_corpus_runs_the_spec_clauses_on_one_fetched_world() {
        let text = r#"{ "thread": "thread/0.1", "world": { "id": "w", "title": "Live" },
            "spawns": [{ "name": "entry", "position": [0,0,0] }],
            "portals": [{ "id": "p", "position": [0,0,0], "to": "thread://x.io/y", "label": "Y" }] }"#;
        let corpus = single_corpus("myhost.com", text);
        let report = run(&corpus);
        assert_eq!(report.worlds, 1);
        assert!(report.passed());

        // A malformed live manifest surfaces as a C1 error.
        let bad = single_corpus("myhost.com", "{ nope }");
        assert!(!run(&bad).passed());
    }

    #[test]
    fn external_links_are_counted_not_penalized() {
        let c = corpus_from(vec![(
            "nexus",
            world(
                "nexus",
                true,
                &[("out", "thread://someone-elses-host.com/room", "Elsewhere")],
            ),
        )]);
        let r = run(&c);
        assert!(r.passed());
        let conn = r
            .clauses
            .iter()
            .find(|c| c.name == "constellation is connected")
            .unwrap();
        assert!(conn.notes.iter().any(|n| n.contains("1 external link")));
    }

    /// C7. The clause has to be narrow or it is worse than nothing: a
    /// base-origin glTF at y = 0 is *correct*, and condemning it would train
    /// authors to ignore the report. So only centre-origin builtins count.
    #[test]
    fn sunk_geometry_is_reported_and_base_origin_models_are_left_alone() {
        let m: WorldManifest = serde_json::from_str(
            r#"{ "thread": "thread/0.1", "world": { "id": "w", "title": "W" },
                 "spawns": [{ "name": "entry", "position": [0,0,0] }],
                 "assets": [{ "id": "urn", "uri": "https://q/x.glb", "kind": "gltf" }],
                 "prefabs": [
                   { "id": 60930001, "mesh": { "builtin": "cylinder" } },
                   { "id": 60930002, "mesh": { "builtin": "plane" } },
                   { "id": 60930003, "mesh": { "asset": "urn" } }
                 ],
                 "placements": [
                   { "prefab": 60930001, "name": "buried post",
                     "position": [0,0.5,0], "scale": [0.3,1.4,0.3] },
                   { "prefab": 60930001, "name": "standing post",
                     "position": [2,0.7,0], "scale": [0.3,1.4,0.3] },
                   { "prefab": 60930001, "name": "half-sunk boulder",
                     "position": [4,0.2,0], "scale": [1,1,1], "solid": false },
                   { "prefab": 60930002, "name": "floor",
                     "position": [0,0,0], "scale": [30,1,30] },
                   { "prefab": 60930003, "name": "a bought column",
                     "position": [6,0,0], "scale": [1,1,1] }
                 ] }"#,
        )
        .expect("manifest");
        let report = run(&corpus_from(vec![("w", m)]));
        let c = report
            .clauses
            .iter()
            .find(|c| c.name == "geometry rests on the ground plane")
            .expect("the clause runs");
        assert_eq!(
            c.severity,
            Severity::Warn,
            "half-buried geometry is legitimate"
        );
        assert!(!c.pass, "the buried post is reported");
        assert_eq!(c.notes.len(), 1, "and nothing else is: {:?}", c.notes);
        assert!(c.notes[0].contains("buried post"), "{:?}", c.notes);
        assert!(report.passed(), "a Warn clause never breaks conformance");
    }

    /// C8. The failure it exists for: a room published without the directory
    /// its models live in. Every other check passes — the manifest is valid,
    /// the spawn is there, the veils are labelled — and the visitor arrives in
    /// a hall with no columns.
    #[test]
    fn a_world_missing_its_own_files_fails_and_links_out_are_left_alone() {
        let dir = std::env::temp_dir().join("thread-c8-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("models")).expect("tmp");
        std::fs::write(dir.join("models/here.glb"), b"glb").expect("write");
        let m: WorldManifest = serde_json::from_str(
            r#"{ "thread": "thread/0.1", "world": { "id": "w", "title": "W" },
                 "spawns": [{ "name": "e", "position": [0,0,0] }],
                 "assets": [
                   { "id": "a", "uri": "models/here.glb", "kind": "gltf" },
                   { "id": "b", "uri": "models/gone.glb", "kind": "gltf" },
                   { "id": "c", "uri": "https://cdn.example/x.glb", "kind": "gltf" },
                   { "id": "d", "uri": "behaviors/absent.wasm", "kind": "wasm" }
                 ] }"#,
        )
        .expect("manifest");
        let corpus = Corpus {
            worlds: vec![World { name: "w".into(), manifest: m, dir: Some(dir.clone()) }],
            load_errors: Vec::new(),
        };
        let report = run(&corpus);
        let c = report.clauses.iter().find(|c| c.name == "declared files are present").unwrap();
        assert_eq!(c.severity, Severity::Error);
        assert!(!c.pass);
        assert_eq!(
            c.notes.len(),
            1,
            "only the missing local file a browser must draw — not the link out, \
             and not the wasm the spec lets a browser ignore: {:?}",
            c.notes
        );
        assert!(c.notes[0].contains("gone.glb"), "{:?}", c.notes);
        assert!(!report.passed(), "a room without its models is not conformant");

        // A world fetched over the wire has no tree to inspect — skipped, not failed.
        let fetched = single_corpus("host", &serde_json::to_string(&corpus.worlds[0].manifest).unwrap());
        let c = run(&fetched)
            .clauses
            .iter()
            .find(|c| c.name == "declared files are present")
            .cloned()
            .unwrap();
        assert!(c.pass, "nothing to check without a directory: {:?}", c.notes);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
