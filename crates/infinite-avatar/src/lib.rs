//! infinite-avatar — modular humanoid character definition.
//!
//! Rust port of the Pixygon `com.pixygon.avatar` Unity package. This crate holds
//! the **portable data model** only — which part sits in which slot, plus body
//! height. Turning a spec into pixels is the renderer's job, behind the
//! [`AvatarRenderer`] seam (a 3D bone-assembler or a 2D sprite-stack).
//!
//! Key ideas carried over from the Unity blueprint:
//! - An avatar is assembled from **slotted parts** ([`AvatarSlot`]).
//! - A **race** ([`AvatarRaceMode`]) is a *body mode* (biology defaults + a morph
//!   weight), not an outfit — a Reptilian and a Human can wear the same shirt.
//! - Parts are keyed by [`StructuredId`] so they're stable across games.

pub mod manifest;
pub mod races;

use std::collections::HashMap;

use thread_id::StructuredId;
use serde::{Deserialize, Serialize};

/// One of the customization points on a humanoid avatar.
///
/// Slots fall into four [`SlotCategory`] groups: biology (race-defined), hair,
/// clothing, and accessories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AvatarSlot {
    // --- Biology (set by the race; tweakable) ---
    Body,
    SkinType,
    SkinTone,
    Eyes,
    Ears,
    Tail,
    Claws,
    Gills,
    Webbing,
    Horns,
    Wings,
    Snout,
    Coat,
    // --- Hair ---
    Hair,
    HairColor,
    // --- Clothing ---
    Shirt,
    Pants,
    Shoes,
    Jacket,
    Headgear,
    Socks,
    Gloves,
    // --- Accessories ---
    AccessoryHead,
    AccessoryBody,
    AccessoryLapel,
    Offhand,
}

/// Coarse grouping of [`AvatarSlot`]s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotCategory {
    Biology,
    Hair,
    Clothing,
    Accessory,
}

impl AvatarSlot {
    /// Every slot, in back-to-front assembly order (body first, accessories last).
    pub const ALL: [AvatarSlot; 26] = [
        AvatarSlot::Body,
        AvatarSlot::SkinType,
        AvatarSlot::SkinTone,
        AvatarSlot::Eyes,
        AvatarSlot::Ears,
        AvatarSlot::Tail,
        AvatarSlot::Claws,
        AvatarSlot::Gills,
        AvatarSlot::Webbing,
        AvatarSlot::Horns,
        AvatarSlot::Wings,
        AvatarSlot::Snout,
        AvatarSlot::Coat,
        AvatarSlot::Hair,
        AvatarSlot::HairColor,
        AvatarSlot::Shirt,
        AvatarSlot::Pants,
        AvatarSlot::Shoes,
        AvatarSlot::Jacket,
        AvatarSlot::Headgear,
        AvatarSlot::Socks,
        AvatarSlot::Gloves,
        AvatarSlot::AccessoryHead,
        AvatarSlot::AccessoryBody,
        AvatarSlot::AccessoryLapel,
        AvatarSlot::Offhand,
    ];

    /// Which category this slot belongs to.
    pub fn category(self) -> SlotCategory {
        use AvatarSlot::*;
        match self {
            Body | SkinType | SkinTone | Eyes | Ears | Tail | Claws | Gills | Webbing | Horns
            | Wings | Snout | Coat => SlotCategory::Biology,
            Hair | HairColor => SlotCategory::Hair,
            Shirt | Pants | Shoes | Jacket | Headgear | Socks | Gloves => SlotCategory::Clothing,
            AccessoryHead | AccessoryBody | AccessoryLapel | Offhand => SlotCategory::Accessory,
        }
    }

    /// The slot's wire name — identical to the Unity `AvatarSlot` enum name and
    /// the slot keys the web Studio saves. This IS the serde form.
    pub fn name(self) -> &'static str {
        match self {
            AvatarSlot::Body => "Body",
            AvatarSlot::SkinType => "SkinType",
            AvatarSlot::SkinTone => "SkinTone",
            AvatarSlot::Eyes => "Eyes",
            AvatarSlot::Ears => "Ears",
            AvatarSlot::Tail => "Tail",
            AvatarSlot::Claws => "Claws",
            AvatarSlot::Gills => "Gills",
            AvatarSlot::Webbing => "Webbing",
            AvatarSlot::Horns => "Horns",
            AvatarSlot::Wings => "Wings",
            AvatarSlot::Snout => "Snout",
            AvatarSlot::Coat => "Coat",
            AvatarSlot::Hair => "Hair",
            AvatarSlot::HairColor => "HairColor",
            AvatarSlot::Shirt => "Shirt",
            AvatarSlot::Pants => "Pants",
            AvatarSlot::Shoes => "Shoes",
            AvatarSlot::Jacket => "Jacket",
            AvatarSlot::Headgear => "Headgear",
            AvatarSlot::Socks => "Socks",
            AvatarSlot::Gloves => "Gloves",
            AvatarSlot::AccessoryHead => "AccessoryHead",
            AvatarSlot::AccessoryBody => "AccessoryBody",
            AvatarSlot::AccessoryLapel => "AccessoryLapel",
            AvatarSlot::Offhand => "Offhand",
        }
    }

    /// Parse a wire name back to a slot (exact match).
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.name() == name)
    }

    /// Is this slot part of the body/biology (set by the race)?
    pub fn is_biology(self) -> bool {
        self.category() == SlotCategory::Biology
    }
}

/// The resolved, portable description of an avatar: which part id fills each
/// slot, plus an overall body height. Part id `0` means "empty". Serialize it
/// into a save, hand it to an [`AvatarBuilder`] to render.
///
/// **This is the shared wire shape** across the whole Pixygon ecosystem — the
/// same JSON the Unity `AvatarSpec`/`AvatarData` bridge and the web
/// `@pixygon/avatar` Studio speak (avatar-v0.1):
///
/// ```json
/// { "bodyHeight": 0.5, "parts": { "Body": 11010001, "Jacket": 123456 } }
/// ```
///
/// Slot keys are the [`AvatarSlot`] names; values are the stable **partId**s
/// (`AvatarAsset.partId` — see [`manifest`]). Never key parts by name or
/// database id.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvatarSpec {
    #[serde(default)]
    parts: HashMap<AvatarSlot, u32>,
    /// 0..1 — lerps the overall body scale.
    #[serde(rename = "bodyHeight", default = "half")]
    pub body_height: f32,
}

fn half() -> f32 {
    0.5
}

impl Default for AvatarSpec {
    fn default() -> Self {
        Self {
            parts: HashMap::new(),
            body_height: 0.5,
        }
    }
}

impl AvatarSpec {
    pub fn new() -> Self {
        Self::default()
    }

    /// Part id in `slot`, or `0` if empty.
    pub fn get(&self, slot: AvatarSlot) -> u32 {
        self.parts.get(&slot).copied().unwrap_or(0)
    }

    /// Place `part_id` in `slot` (id `0` clears it).
    pub fn set(&mut self, slot: AvatarSlot, part_id: u32) {
        if part_id == 0 {
            self.parts.remove(&slot);
        } else {
            self.parts.insert(slot, part_id);
        }
    }

    pub fn has(&self, slot: AvatarSlot) -> bool {
        self.parts.contains_key(&slot)
    }

    pub fn clear(&mut self, slot: AvatarSlot) {
        self.parts.remove(&slot);
    }

    /// Iterate the filled (slot, part_id) pairs.
    pub fn iter(&self) -> impl Iterator<Item = (AvatarSlot, u32)> + '_ {
        self.parts.iter().map(|(&s, &id)| (s, id))
    }

    /// Build a spec from a flat `slot-name → partId` config (the shape the web
    /// Studio saves and `resolveAvatarConfig` consumes). Unknown slot names and
    /// zero ids are skipped.
    pub fn from_config(config: &HashMap<String, u32>) -> Self {
        let mut spec = Self::default();
        for (name, part_id) in config {
            if let Some(slot) = AvatarSlot::from_name(name) {
                spec.set(slot, *part_id);
            }
        }
        spec
    }

    /// The flat `slot-name → partId` view of this spec (filled slots only).
    pub fn to_config(&self) -> HashMap<String, u32> {
        self.parts
            .iter()
            .map(|(slot, id)| (slot.name().to_string(), *id))
            .collect()
    }

    pub fn filled_count(&self) -> usize {
        self.parts.len()
    }
}

/// A catalog entry: one wearable/biological part, with its 3D and 2D assets.
///
/// Asset references are abstract string keys here — the renderer resolves them
/// to a mesh/material or a sprite. (The Unity package held `GameObject`/`Sprite`
/// directly; the Rust engine loads by key.)
#[derive(Debug, Clone)]
pub struct AvatarPart {
    pub id: StructuredId,
    pub slot: AvatarSlot,
    pub display_name: String,
    /// Tint applied to the part (skin tone, hair color, …). RGBA 0..1.
    pub tint: [f32; 4],
    /// `false` = locked until granted (SkinCard / NFT / account).
    pub owned_by_default: bool,
    /// 3D asset key (mesh + material), if this part has a 3D form.
    pub mesh: Option<String>,
    /// 2D asset key (sprite), if this part has a 2D form.
    pub sprite: Option<String>,
    /// Layer order for the 2D sprite stack (back-to-front).
    pub sorting_order: i32,
}

impl AvatarPart {
    /// A minimal 3D part.
    pub fn mesh_part(id: StructuredId, slot: AvatarSlot, name: &str, mesh: &str) -> Self {
        Self {
            id,
            slot,
            display_name: name.to_string(),
            tint: [1.0, 1.0, 1.0, 1.0],
            owned_by_default: true,
            mesh: Some(mesh.to_string()),
            sprite: None,
            sorting_order: 0,
        }
    }
}

/// A race = a set of biology slot defaults plus a morph weight. Applying it sets
/// the biology slots; everything else stays custom.
#[derive(Debug, Clone)]
pub struct AvatarRaceMode {
    pub id: StructuredId,
    pub name: String,
    /// Default part ids for (biology) slots.
    pub slot_defaults: HashMap<AvatarSlot, u32>,
    /// 1.0 = full race; `< 1.0` blends toward a proto-form.
    pub morph_weight: f32,
}

impl AvatarRaceMode {
    pub fn new(id: StructuredId, name: &str) -> Self {
        Self {
            id,
            name: name.to_string(),
            slot_defaults: HashMap::new(),
            morph_weight: 1.0,
        }
    }

    pub fn with_default(mut self, slot: AvatarSlot, part_id: u32) -> Self {
        self.slot_defaults.insert(slot, part_id);
        self
    }

    /// Stamp this race's biology defaults onto `spec`.
    pub fn apply_to(&self, spec: &mut AvatarSpec) {
        for (&slot, &id) in &self.slot_defaults {
            spec.set(slot, id);
        }
    }
}

/// The per-game wardrobe: the parts available to choose from.
#[derive(Debug, Clone, Default)]
pub struct AvatarCatalog {
    parts: Vec<AvatarPart>,
}

impl AvatarCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, part: AvatarPart) {
        self.parts.push(part);
    }

    /// Look up a part by its structured id.
    pub fn get(&self, id: StructuredId) -> Option<&AvatarPart> {
        self.parts.iter().find(|p| p.id == id)
    }

    /// All parts that can go in `slot` (for a customizer menu).
    pub fn parts_for_slot(&self, slot: AvatarSlot) -> impl Iterator<Item = &AvatarPart> {
        self.parts.iter().filter(move |p| p.slot == slot)
    }

    pub fn len(&self) -> usize {
        self.parts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }
}

/// The rendering seam. Implemented by a 3D rig-assembler (parent meshes to
/// bones, swap skin material, scale by body height) or a 2D sprite-stacker.
pub trait AvatarRenderer {
    /// Tear down the current body.
    fn clear(&mut self);
    /// Set the base body and overall height (0..1).
    fn set_body(&mut self, body: &AvatarPart, body_height: f32);
    /// Mount one slot's part.
    fn set_part(&mut self, slot: AvatarSlot, part: &AvatarPart);
    /// Finalize (rebuild/bake).
    fn commit(&mut self);
}

/// Resolves an [`AvatarSpec`] against a catalog and drives a renderer.
pub struct AvatarBuilder<'a> {
    catalog: &'a AvatarCatalog,
}

impl<'a> AvatarBuilder<'a> {
    pub fn new(catalog: &'a AvatarCatalog) -> Self {
        Self { catalog }
    }

    /// Build `spec` into `renderer`: clear, set the body, mount each filled slot
    /// in assembly order, then commit. Slots whose part id isn't in the catalog
    /// are skipped (a missing hat just doesn't render).
    pub fn build(&self, spec: &AvatarSpec, renderer: &mut impl AvatarRenderer) {
        renderer.clear();

        // Body first (provides the rig / base layer).
        let body_id = spec.get(AvatarSlot::Body);
        if body_id != 0 {
            if let Some(body) = self.catalog.get(StructuredId(body_id)) {
                renderer.set_body(body, spec.body_height);
            }
        }

        for &slot in AvatarSlot::ALL.iter() {
            if slot == AvatarSlot::Body {
                continue;
            }
            let id = spec.get(slot);
            if id == 0 {
                continue;
            }
            if let Some(part) = self.catalog.get(StructuredId(id)) {
                renderer.set_part(slot, part);
            }
        }

        renderer.commit();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(id: u32, slot: AvatarSlot, name: &str) -> AvatarPart {
        AvatarPart::mesh_part(StructuredId(id), slot, name, "mesh_key")
    }

    /// The 26 slot wire names, in Unity `AvatarSlot` declaration order — this
    /// pins cross-runtime parity: Unity, `@pixygon/avatar`, and this crate must
    /// all agree on these strings forever (append-only).
    #[test]
    fn slot_wire_names_match_the_unity_enum() {
        let expected = [
            "Body",
            "SkinType",
            "SkinTone",
            "Eyes",
            "Ears",
            "Tail",
            "Claws",
            "Gills",
            "Webbing",
            "Horns",
            "Wings",
            "Snout",
            "Coat",
            "Hair",
            "HairColor",
            "Shirt",
            "Pants",
            "Shoes",
            "Jacket",
            "Headgear",
            "Socks",
            "Gloves",
            "AccessoryHead",
            "AccessoryBody",
            "AccessoryLapel",
            "Offhand",
        ];
        assert_eq!(AvatarSlot::ALL.len(), expected.len());
        for (slot, want) in AvatarSlot::ALL.into_iter().zip(expected) {
            assert_eq!(slot.name(), want);
            assert_eq!(AvatarSlot::from_name(want), Some(slot));
            // serde uses the same strings — the enum IS the wire format.
            assert_eq!(serde_json::to_string(&slot).unwrap(), format!("\"{want}\""));
        }
    }

    /// The shared avatar-v0.1 wire shape parses and round-trips.
    #[test]
    fn spec_speaks_the_shared_wire_shape() {
        let spec: AvatarSpec = serde_json::from_str(
            r#"{ "bodyHeight": 0.7, "parts": { "Body": 11010001, "Jacket": 123456 } }"#,
        )
        .unwrap();
        assert_eq!(spec.body_height, 0.7);
        assert_eq!(spec.get(AvatarSlot::Body), 11010001);
        assert_eq!(spec.get(AvatarSlot::Jacket), 123456);
        // Partial JSON gets defaults (bodyHeight 0.5, no parts).
        let bare: AvatarSpec = serde_json::from_str("{}").unwrap();
        assert_eq!(bare.body_height, 0.5);
        assert_eq!(bare.filled_count(), 0);
        // Round-trip preserves the shape.
        let back: AvatarSpec =
            serde_json::from_str(&serde_json::to_string(&spec).unwrap()).unwrap();
        assert_eq!(back.get(AvatarSlot::Jacket), 123456);
        assert_eq!(back.body_height, 0.7);
        // Flat Studio config (slot-name → partId) converts both ways.
        let config = spec.to_config();
        assert_eq!(config["Jacket"], 123456);
        let again = AvatarSpec::from_config(&config);
        assert_eq!(again.get(AvatarSlot::Body), 11010001);
    }

    #[test]
    fn slot_categories() {
        assert_eq!(AvatarSlot::Tail.category(), SlotCategory::Biology);
        assert!(AvatarSlot::Tail.is_biology());
        assert_eq!(AvatarSlot::Hair.category(), SlotCategory::Hair);
        assert_eq!(AvatarSlot::Shirt.category(), SlotCategory::Clothing);
        assert_eq!(AvatarSlot::Offhand.category(), SlotCategory::Accessory);
        assert!(!AvatarSlot::Shirt.is_biology());
        // Every slot is in ALL exactly once.
        assert_eq!(AvatarSlot::ALL.len(), 26);
    }

    #[test]
    fn spec_get_set_clear() {
        let mut spec = AvatarSpec::new();
        assert_eq!(spec.body_height, 0.5);
        assert_eq!(spec.get(AvatarSlot::Shirt), 0);
        assert!(!spec.has(AvatarSlot::Shirt));

        spec.set(AvatarSlot::Shirt, 42);
        assert_eq!(spec.get(AvatarSlot::Shirt), 42);
        assert!(spec.has(AvatarSlot::Shirt));
        assert_eq!(spec.filled_count(), 1);

        // Setting id 0 clears.
        spec.set(AvatarSlot::Shirt, 0);
        assert!(!spec.has(AvatarSlot::Shirt));
        assert_eq!(spec.filled_count(), 0);
    }

    #[test]
    fn race_applies_biology_defaults() {
        let mut spec = AvatarSpec::new();
        spec.set(AvatarSlot::Shirt, 100); // custom clothing stays

        let reptilian = AvatarRaceMode::new(StructuredId(1), "Reptilian")
            .with_default(AvatarSlot::SkinType, 7)
            .with_default(AvatarSlot::Tail, 8)
            .with_default(AvatarSlot::Claws, 9);
        reptilian.apply_to(&mut spec);

        assert_eq!(spec.get(AvatarSlot::SkinType), 7);
        assert_eq!(spec.get(AvatarSlot::Tail), 8);
        assert_eq!(spec.get(AvatarSlot::Claws), 9);
        // Clothing untouched by race.
        assert_eq!(spec.get(AvatarSlot::Shirt), 100);
    }

    #[test]
    fn catalog_lookup() {
        let mut cat = AvatarCatalog::new();
        cat.add(part(10, AvatarSlot::Hair, "Spiky"));
        cat.add(part(11, AvatarSlot::Hair, "Bob"));
        cat.add(part(20, AvatarSlot::Shirt, "Tunic"));

        assert_eq!(cat.len(), 3);
        assert_eq!(cat.get(StructuredId(11)).unwrap().display_name, "Bob");
        assert!(cat.get(StructuredId(999)).is_none());
        assert_eq!(cat.parts_for_slot(AvatarSlot::Hair).count(), 2);
        assert_eq!(cat.parts_for_slot(AvatarSlot::Shirt).count(), 1);
    }

    /// Records the calls the builder makes, so we can assert ordering.
    #[derive(Default)]
    struct RecordingRenderer {
        log: Vec<String>,
    }
    impl AvatarRenderer for RecordingRenderer {
        fn clear(&mut self) {
            self.log.push("clear".into());
        }
        fn set_body(&mut self, body: &AvatarPart, h: f32) {
            self.log.push(format!("body:{}:{}", body.display_name, h));
        }
        fn set_part(&mut self, slot: AvatarSlot, part: &AvatarPart) {
            self.log
                .push(format!("part:{:?}:{}", slot, part.display_name));
        }
        fn commit(&mut self) {
            self.log.push("commit".into());
        }
    }

    #[test]
    fn builder_assembles_in_order() {
        let mut cat = AvatarCatalog::new();
        cat.add(part(1, AvatarSlot::Body, "HumanBody"));
        cat.add(part(2, AvatarSlot::Hair, "Spiky"));
        cat.add(part(3, AvatarSlot::Shirt, "Tunic"));

        let mut spec = AvatarSpec::new();
        spec.body_height = 0.7;
        spec.set(AvatarSlot::Body, 1);
        spec.set(AvatarSlot::Hair, 2);
        spec.set(AvatarSlot::Shirt, 3);
        spec.set(AvatarSlot::Pants, 999); // not in catalog → skipped

        let mut r = RecordingRenderer::default();
        AvatarBuilder::new(&cat).build(&spec, &mut r);

        assert_eq!(r.log[0], "clear");
        assert_eq!(r.log[1], "body:HumanBody:0.7");
        // Body comes before parts; Hair (slot order) before Shirt.
        let hair_idx = r.log.iter().position(|s| s.contains("Spiky")).unwrap();
        let shirt_idx = r.log.iter().position(|s| s.contains("Tunic")).unwrap();
        assert!(hair_idx < shirt_idx);
        assert!(!r.log.iter().any(|s| s.contains("Pants"))); // skipped (missing)
        assert_eq!(r.log.last().unwrap(), "commit");
    }

    #[test]
    fn spec_serde_roundtrip() {
        let mut spec = AvatarSpec::new();
        spec.body_height = 0.9;
        spec.set(AvatarSlot::Hair, 5);
        spec.set(AvatarSlot::Tail, 8);

        let json = serde_json::to_string(&spec).unwrap();
        let back: AvatarSpec = serde_json::from_str(&json).unwrap();
        assert_eq!(back.body_height, 0.9);
        assert_eq!(back.get(AvatarSlot::Hair), 5);
        assert_eq!(back.get(AvatarSlot::Tail), 8);
    }
}
