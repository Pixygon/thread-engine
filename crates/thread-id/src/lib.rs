//! # `thread-id` — the StructuredId scheme for the Thread
//!
//! A compact, human-readable id for renderable prefabs and entities on the Thread:
//! `CCSSNNNN` (8 decimal digits) packed into a `u32` — category / subcategory /
//! number. It (de)serializes as either the 8-digit string `"21010023"` or the raw
//! number `21010023`, so a World Manifest can key prefabs by a stable id.
//!
//! This crate is deliberately tiny and dependency-light (only `serde`) so the
//! neutral [World Manifest](https://github.com/Pixygon/Infinite) format can depend
//! on the id scheme without pulling in any engine or game code. The Pixygon-specific
//! *category meanings* (which range is Actors, Items, …) live in the engine, not
//! here — this crate is just the encoding.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

/// A structured decimal ID encoding category, subcategory, and item number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StructuredId(pub u32);

impl StructuredId {
    /// Create a new structured ID from components.
    ///
    /// # Panics
    /// Panics if category is 0 or >99, subcategory >99, or number is 0 or >9999.
    pub fn new(category: u8, subcategory: u8, number: u16) -> Self {
        assert!((1..=99).contains(&category), "category must be 1-99");
        assert!(subcategory <= 99, "subcategory must be 0-99");
        assert!((1..=9999).contains(&number), "number must be 1-9999");
        Self(category as u32 * 1_000_000 + subcategory as u32 * 10_000 + number as u32)
    }

    /// Try to create a structured ID, returning None if components are out of range.
    pub fn try_new(category: u8, subcategory: u8, number: u16) -> Option<Self> {
        if !(1..=99).contains(&category) || subcategory > 99 || !(1..=9999).contains(&number) {
            return None;
        }
        Some(Self(
            category as u32 * 1_000_000 + subcategory as u32 * 10_000 + number as u32,
        ))
    }

    /// Extract the category (CC) component.
    pub fn category(&self) -> u8 {
        (self.0 / 1_000_000) as u8
    }

    /// Extract the subcategory (SS) component.
    pub fn subcategory(&self) -> u8 {
        ((self.0 / 10_000) % 100) as u8
    }

    /// Extract the item number (NNNN) component.
    pub fn number(&self) -> u16 {
        (self.0 % 10_000) as u16
    }

    /// Determine the domain based on category range.
    pub fn domain(&self) -> IdDomain {
        match self.category() {
            10..=19 => IdDomain::Actors,
            20..=29 => IdDomain::Items,
            30..=39 => IdDomain::Skills,
            40..=49 => IdDomain::World,
            _ => IdDomain::Unknown,
        }
    }

    /// Format as zero-padded 8-digit string "CCSSNNNN".
    pub fn to_id_string(&self) -> String {
        format!("{:08}", self.0)
    }

    /// Parse from an 8-digit string "CCSSNNNN".
    pub fn parse(s: &str) -> Option<Self> {
        let val: u32 = s.parse().ok()?;
        Self::from_raw(val)
    }

    /// Create from a raw u32, validating the encoded components.
    pub fn from_raw(val: u32) -> Option<Self> {
        let id = Self(val);
        let cat = id.category();
        let num = id.number();
        if !(1..=99).contains(&cat) || !(1..=9999).contains(&num) {
            return None;
        }
        Some(id)
    }
}

impl fmt::Display for StructuredId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:08}", self.0)
    }
}

impl Serialize for StructuredId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_id_string())
    }
}

impl<'de> Deserialize<'de> for StructuredId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct IdVisitor;

        impl<'de> serde::de::Visitor<'de> for IdVisitor {
            type Value = StructuredId;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a structured ID string \"CCSSNNNN\" or a u32 number")
            }

            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<StructuredId, E> {
                StructuredId::parse(v)
                    .ok_or_else(|| E::custom(format!("invalid structured ID: {v}")))
            }

            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<StructuredId, E> {
                if v > u32::MAX as u64 {
                    return Err(E::custom(format!("structured ID too large: {v}")));
                }
                StructuredId::from_raw(v as u32)
                    .ok_or_else(|| E::custom(format!("invalid structured ID value: {v}")))
            }

            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<StructuredId, E> {
                if v < 0 || v > u32::MAX as i64 {
                    return Err(E::custom(format!("structured ID out of range: {v}")));
                }
                self.visit_u64(v as u64)
            }
        }

        deserializer.deserialize_any(IdVisitor)
    }
}

/// High-level domain grouping derived from category ranges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IdDomain {
    /// Categories 10-19: Player characters, humanoid NPCs, creatures
    Actors,
    /// Categories 20-29: Weapons, armor, accessories, clothing, consumables, materials, gems, runes
    Items,
    /// Categories 30-39: Active and passive skills
    Skills,
    /// Categories 40-49: Doors, containers, levers, portals, signs
    World,
    /// Category outside defined ranges
    Unknown,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_and_decodes_components() {
        let id = StructuredId::new(21, 1, 23);
        assert_eq!(id.0, 21_01_0023);
        assert_eq!(id.category(), 21);
        assert_eq!(id.subcategory(), 1);
        assert_eq!(id.number(), 23);
        assert_eq!(id.to_id_string(), "21010023");
    }

    #[test]
    fn parses_and_validates_raw() {
        assert_eq!(
            StructuredId::parse("21010023"),
            Some(StructuredId(21010023))
        );
        assert!(StructuredId::from_raw(0).is_none(), "category 0 is invalid");
        assert!(StructuredId::try_new(0, 0, 1).is_none());
    }

    #[test]
    fn domain_follows_category_ranges() {
        assert_eq!(StructuredId::new(11, 0, 1).domain(), IdDomain::Actors);
        assert_eq!(StructuredId::new(21, 0, 1).domain(), IdDomain::Items);
        assert_eq!(StructuredId::new(99, 0, 1).domain(), IdDomain::Unknown);
    }

    #[test]
    fn serde_accepts_string_or_number() {
        let from_str: StructuredId = serde_json::from_str("\"21010023\"").unwrap();
        let from_num: StructuredId = serde_json::from_str("21010023").unwrap();
        assert_eq!(from_str, from_num);
        // Serializes back to the canonical 8-digit string.
        assert_eq!(serde_json::to_string(&from_str).unwrap(), "\"21010023\"");
    }
}
