//! Canonical Pixygon races, grounded in the Codex.
//!
//! A race is a *body mode* — a set of biology-slot defaults (see
//! [`AvatarRaceMode`]). These four are the canonical playable species from the
//! universe Codex:
//! - **Humen** — the baseline / quintessence-race (smooth skin, no exotic parts).
//! - **Hedningr** (Reptillian) — the reptiles: scale skin, tail, claws, horns.
//! - **Arra** — the beast-race: fur coat, claws, snout, beast ears, tail.
//! - **Draumander** (Skyfolk) — the bird-race: feathered skin, wings, beak snout.
//!
//! Part ids here are placeholders in the avatar id space (category 50); real
//! authored parts (meshes/sprites) replace them via the catalog later. The
//! *structure* — which biology slots each race fills — is the canon-faithful bit.

use crate::{AvatarCatalog, AvatarPart, AvatarRaceMode, AvatarSlot};
use thread_id::StructuredId;

/// Category for avatar parts in the structured-id space.
pub const CAT_AVATAR_PART: u8 = 50;
/// Category for avatar race modes.
pub const CAT_AVATAR_RACE: u8 = 51;

// Biology part ids (subcategory = body-feature kind, number = variant).
// Matches StructuredId's CCSSNNNN decimal encoding (category 50 = avatar part).
mod ids {
    use super::CAT_AVATAR_PART;
    const fn part(sub: u32, num: u32) -> u32 {
        CAT_AVATAR_PART as u32 * 1_000_000 + sub * 10_000 + num
    }
    pub const SKIN_SMOOTH: u32 = part(1, 1);
    pub const SKIN_SCALE: u32 = part(1, 2);
    pub const SKIN_FUR: u32 = part(1, 3);
    pub const SKIN_FEATHER: u32 = part(1, 4);
    pub const TAIL_REPTILE: u32 = part(2, 1);
    pub const TAIL_BEAST: u32 = part(2, 2);
    pub const CLAWS_BASIC: u32 = part(3, 1);
    pub const COAT_FUR: u32 = part(4, 1);
    pub const WINGS_FEATHER: u32 = part(5, 1);
    pub const HORNS_BASIC: u32 = part(6, 1);
    pub const SNOUT_BEAST: u32 = part(7, 1);
    pub const SNOUT_BEAK: u32 = part(7, 2);
    pub const EARS_BEAST: u32 = part(8, 1);
}

/// The canonical playable species.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanonicalRace {
    Humen,
    Hedningr,
    Arra,
    Draumander,
}

impl CanonicalRace {
    pub const ALL: [CanonicalRace; 4] = [
        CanonicalRace::Humen,
        CanonicalRace::Hedningr,
        CanonicalRace::Arra,
        CanonicalRace::Draumander,
    ];

    /// Codex slug for this race.
    pub fn slug(self) -> &'static str {
        match self {
            CanonicalRace::Humen => "humen",
            CanonicalRace::Hedningr => "reptillian",
            CanonicalRace::Arra => "arra",
            CanonicalRace::Draumander => "skyfolk",
        }
    }

    /// Display name (matches the Codex `name`).
    pub fn name(self) -> &'static str {
        match self {
            CanonicalRace::Humen => "Humen",
            CanonicalRace::Hedningr => "Hedningr",
            CanonicalRace::Arra => "Arra",
            CanonicalRace::Draumander => "Draumander",
        }
    }

    fn race_number(self) -> u16 {
        match self {
            CanonicalRace::Humen => 1,
            CanonicalRace::Hedningr => 2,
            CanonicalRace::Arra => 3,
            CanonicalRace::Draumander => 4,
        }
    }

    /// The biology defaults that make this race a distinct body mode.
    pub fn race_mode(self) -> AvatarRaceMode {
        let id = StructuredId::new(CAT_AVATAR_RACE, 0, self.race_number());
        let mut race = AvatarRaceMode::new(id, self.name());
        match self {
            CanonicalRace::Humen => {
                race = race.with_default(AvatarSlot::SkinType, ids::SKIN_SMOOTH);
            }
            CanonicalRace::Hedningr => {
                race = race
                    .with_default(AvatarSlot::SkinType, ids::SKIN_SCALE)
                    .with_default(AvatarSlot::Tail, ids::TAIL_REPTILE)
                    .with_default(AvatarSlot::Claws, ids::CLAWS_BASIC)
                    .with_default(AvatarSlot::Horns, ids::HORNS_BASIC);
            }
            CanonicalRace::Arra => {
                race = race
                    .with_default(AvatarSlot::SkinType, ids::SKIN_FUR)
                    .with_default(AvatarSlot::Coat, ids::COAT_FUR)
                    .with_default(AvatarSlot::Claws, ids::CLAWS_BASIC)
                    .with_default(AvatarSlot::Snout, ids::SNOUT_BEAST)
                    .with_default(AvatarSlot::Ears, ids::EARS_BEAST)
                    .with_default(AvatarSlot::Tail, ids::TAIL_BEAST);
            }
            CanonicalRace::Draumander => {
                race = race
                    .with_default(AvatarSlot::SkinType, ids::SKIN_FEATHER)
                    .with_default(AvatarSlot::Wings, ids::WINGS_FEATHER)
                    .with_default(AvatarSlot::Snout, ids::SNOUT_BEAK);
            }
        }
        race
    }
}

/// Every canonical race as an [`AvatarRaceMode`].
pub fn canonical_races() -> Vec<AvatarRaceMode> {
    CanonicalRace::ALL.iter().map(|r| r.race_mode()).collect()
}

/// A catalog of the placeholder biology parts referenced by the canonical races,
/// so a race-stamped [`crate::AvatarSpec`] resolves through an [`crate::AvatarBuilder`].
pub fn biology_catalog() -> AvatarCatalog {
    let mut cat = AvatarCatalog::new();
    let entries: &[(u32, AvatarSlot, &str)] = &[
        (ids::SKIN_SMOOTH, AvatarSlot::SkinType, "Smooth Skin"),
        (ids::SKIN_SCALE, AvatarSlot::SkinType, "Scale Skin"),
        (ids::SKIN_FUR, AvatarSlot::SkinType, "Fur Skin"),
        (ids::SKIN_FEATHER, AvatarSlot::SkinType, "Feather Skin"),
        (ids::TAIL_REPTILE, AvatarSlot::Tail, "Reptile Tail"),
        (ids::TAIL_BEAST, AvatarSlot::Tail, "Beast Tail"),
        (ids::CLAWS_BASIC, AvatarSlot::Claws, "Claws"),
        (ids::COAT_FUR, AvatarSlot::Coat, "Fur Coat"),
        (ids::WINGS_FEATHER, AvatarSlot::Wings, "Feathered Wings"),
        (ids::HORNS_BASIC, AvatarSlot::Horns, "Horns"),
        (ids::SNOUT_BEAST, AvatarSlot::Snout, "Beast Snout"),
        (ids::SNOUT_BEAK, AvatarSlot::Snout, "Beak"),
        (ids::EARS_BEAST, AvatarSlot::Ears, "Beast Ears"),
    ];
    for &(id, slot, name) in entries {
        cat.add(AvatarPart::mesh_part(
            StructuredId(id),
            slot,
            name,
            "placeholder",
        ));
    }
    cat
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AvatarSpec, SlotCategory};

    #[test]
    fn four_canonical_races() {
        assert_eq!(CanonicalRace::ALL.len(), 4);
        assert_eq!(canonical_races().len(), 4);
        assert_eq!(CanonicalRace::Hedningr.name(), "Hedningr");
        assert_eq!(CanonicalRace::Hedningr.slug(), "reptillian");
        assert_eq!(CanonicalRace::Draumander.slug(), "skyfolk");
    }

    #[test]
    fn hedningr_is_a_reptile_body() {
        let mut spec = AvatarSpec::new();
        CanonicalRace::Hedningr.race_mode().apply_to(&mut spec);
        assert_eq!(spec.get(AvatarSlot::SkinType), ids::SKIN_SCALE);
        assert!(spec.has(AvatarSlot::Tail));
        assert!(spec.has(AvatarSlot::Claws));
        assert!(spec.has(AvatarSlot::Horns));
        // Reptiles have no wings.
        assert!(!spec.has(AvatarSlot::Wings));
    }

    #[test]
    fn draumander_has_wings() {
        let mut spec = AvatarSpec::new();
        CanonicalRace::Draumander.race_mode().apply_to(&mut spec);
        assert!(spec.has(AvatarSlot::Wings));
        assert_eq!(spec.get(AvatarSlot::Snout), ids::SNOUT_BEAK);
    }

    #[test]
    fn humen_is_baseline() {
        let mut spec = AvatarSpec::new();
        CanonicalRace::Humen.race_mode().apply_to(&mut spec);
        // Only the baseline skin; no tail/claws/wings/horns.
        assert_eq!(spec.get(AvatarSlot::SkinType), ids::SKIN_SMOOTH);
        for s in [
            AvatarSlot::Tail,
            AvatarSlot::Claws,
            AvatarSlot::Wings,
            AvatarSlot::Horns,
            AvatarSlot::Coat,
        ] {
            assert!(!spec.has(s), "Humen should not have {s:?}");
        }
    }

    #[test]
    fn all_race_defaults_are_biology_and_resolve_in_catalog() {
        let cat = biology_catalog();
        for race in canonical_races() {
            let mut spec = AvatarSpec::new();
            race.apply_to(&mut spec);
            for (slot, id) in spec.iter() {
                assert_eq!(
                    slot.category(),
                    SlotCategory::Biology,
                    "race default in non-biology slot"
                );
                assert!(
                    cat.get(StructuredId(id)).is_some(),
                    "part {id} not in biology catalog"
                );
            }
        }
    }
}
