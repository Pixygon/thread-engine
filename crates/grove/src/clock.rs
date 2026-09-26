//! The clock — how old a plant is, and where the year stands.
//!
//! A plant is a species, a seed and a clock. The species is the rule set, the
//! seed is which individual, and the clock is *when you are looking*. Nothing
//! here reshapes a plant: the seed already decided the whole potential plant,
//! every branch it could ever have, and the clock only chooses which of those
//! branches have emerged, how far they have grown, and how thick they are.
//!
//! Time is **ticks the game controls**, never wall-clock days: `age` counts
//! seasons since the plant sprouted, and `season` says where in the year it is.
//! A world that runs a season every ten minutes and one that runs a season a
//! month use the same numbers.
use serde::{Deserialize, Serialize};

/// When a plant is being looked at.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct Clock {
    /// Seasons since sprouting. `None` = fully grown, which is what a recipe
    /// with no clock shows: the whole potential plant, nothing filtered.
    pub age: Option<f32>,
    /// Position in the year, 0..1: 0 is bud break, and it runs through leaf,
    /// bloom, fruit and seed drop back to bare. Life stages and the year's
    /// expression are the next step; the wood already knows the age.
    pub season: f32,
}

impl Default for Clock {
    fn default() -> Self {
        Self::grown()
    }
}

impl Clock {
    /// Fully grown, high summer — the plant at its potential.
    pub fn grown() -> Self {
        Self { age: None, season: 0.35 }
    }

    /// A plant `age` seasons old, in high summer.
    pub fn at(age: f32) -> Self {
        Self { age: Some(age.max(0.0)), season: 0.35 }
    }

    /// How far through its growing life the plant is, 0..1, given the seasons
    /// its species takes to reach full size. Past full size this stays 1: an
    /// old tree is not a bigger tree, and what age does after that (gnarl,
    /// dieback, the withered state) is the next step's work.
    pub fn maturity(&self, seasons_to_grown: f32) -> f32 {
        match self.age {
            None => 1.0,
            Some(a) => (a / seasons_to_grown.max(1e-3)).clamp(0.0, 1.0),
        }
    }

    /// Is this the plant at its potential — nothing filtered, nothing scaled?
    pub fn is_grown(&self, seasons_to_grown: f32) -> bool {
        self.maturity(seasons_to_grown) >= 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maturity_runs_zero_to_one_and_stops() {
        assert_eq!(Clock::grown().maturity(12.0), 1.0);
        assert_eq!(Clock::at(0.0).maturity(12.0), 0.0);
        assert!((Clock::at(6.0).maturity(12.0) - 0.5).abs() < 1e-6);
        assert_eq!(Clock::at(40.0).maturity(12.0), 1.0, "an old tree is not a bigger tree");
        assert!(Clock::at(12.0).is_grown(12.0));
        assert!(!Clock::at(11.0).is_grown(12.0));
    }

    #[test]
    fn a_clock_round_trips_through_json() {
        let json = serde_json::to_string(&Clock::at(3.5)).unwrap();
        let back: Clock = serde_json::from_str(&json).unwrap();
        assert_eq!(back.age, Some(3.5));
        // An empty clock is a grown plant, so old recipes show what they always did.
        let empty: Clock = serde_json::from_str("{}").unwrap();
        assert!(empty.age.is_none());
    }
}
