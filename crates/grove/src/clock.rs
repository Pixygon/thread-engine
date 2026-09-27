//! The clock — how old a plant is, and where the year stands.
//!
//! A plant is a species, a seed and a clock (and what has happened to it). The
//! species is the rule set, the seed is which individual, and the clock is
//! *when you are looking*. Nothing here reshapes a plant: the seed already
//! decided the whole potential plant, every branch it could ever have, and the
//! clock only chooses which of those branches have emerged, how far they have
//! grown, what the crown is wearing and what is hanging in it.
//!
//! Time is **ticks the game controls**, never wall-clock days: `age` counts
//! seasons since the plant sprouted, and `season` says where in the year it is.
//! A world that runs a season every ten minutes and one that runs a season a
//! month use the same numbers.
//!
//! Two readings come off it. [`Stage`] is the long arc — sapling, mature, old,
//! dying — and [`Phase`] is the year going round: bud, leaf, bloom, fruit,
//! seed drop, bare.
use serde::{Deserialize, Serialize};

/// When a plant is being looked at.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct Clock {
    /// Seasons since sprouting. `None` = fully grown, which is what a recipe
    /// with no clock shows: the whole potential plant, nothing filtered.
    pub age: Option<f32>,
    /// Position in the year, 0..1. It runs bud → leaf → bloom → fruit → seed
    /// drop → bare and wraps, so 1.0 and 0.0 are the same moment.
    pub season: f32,
}

impl Default for Clock {
    fn default() -> Self {
        Self::grown()
    }
}

/// The long arc of a life. What it changes is never structure — an old tree is
/// the same tree, thinner in the crown; nothing re-rolls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    /// Still growing into the plant the seed decided.
    Sapling,
    /// Grown, and at its fullest.
    Mature,
    /// Grown a long time: the crown thins and the outermost twigs go bare.
    Old,
    /// The end of the arc: most of the crown is gone.
    Dying,
}

/// Where the year stands. The windows are fixed here rather than per species
/// because a world runs one year for everything in it; what a species *does*
/// in each window is the species' business.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Bud,
    Leaf,
    Bloom,
    Fruit,
    SeedDrop,
    Bare,
}

/// The year, as fractions of it: bud break, leaf out, bloom, fruit set, seed
/// drop, bare.
const YEAR: [(f32, Phase); 6] = [
    (0.00, Phase::Bud),
    (0.10, Phase::Leaf),
    (0.25, Phase::Bloom),
    (0.40, Phase::Fruit),
    (0.70, Phase::SeedDrop),
    (0.85, Phase::Bare),
];

/// Fractions of a whole life at which the long arc turns. Growing up is a
/// small slice of a tree's life — a plant is grown for far longer than it
/// spends getting there.
const OLD_AT: f32 = 0.70;
const DYING_AT: f32 = 0.92;

impl Clock {
    /// Fully grown, in the bloom of the year — the plant at its potential.
    pub fn grown() -> Self {
        Self { age: None, season: 0.35 }
    }

    /// A plant `age` seasons old, in the bloom of the year.
    pub fn at(age: f32) -> Self {
        Self { age: Some(age.max(0.0)), season: 0.35 }
    }

    /// The same moment, elsewhere in the year.
    pub fn in_season(self, season: f32) -> Self {
        Self { season: wrap(season), ..self }
    }

    /// How far through its growing life the plant is, 0..1, given the seasons
    /// its species takes to reach full size. Past full size this stays 1: an
    /// old tree is not a bigger tree — what age does after that is [`Stage`].
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

    /// Where in the long arc this plant is. A clock with no age is a plant in
    /// its prime: a recipe that says nothing about time shows a mature plant,
    /// not a dying one.
    pub fn stage(&self, seasons_to_grown: f32, seasons_of_life: f32) -> Stage {
        let Some(age) = self.age else { return Stage::Mature };
        if age < seasons_to_grown {
            return Stage::Sapling;
        }
        let life = seasons_of_life.max(seasons_to_grown.max(1e-3));
        let t = age / life;
        if t >= DYING_AT {
            Stage::Dying
        } else if t >= OLD_AT {
            Stage::Old
        } else {
            Stage::Mature
        }
    }

    /// Where the year stands, 0..1 — `season` brought inside one turn, since
    /// a year has no end.
    pub fn year(&self) -> f32 {
        wrap(self.season)
    }

    /// Where the year stands.
    pub fn phase(&self) -> Phase {
        let s = wrap(self.season);
        let mut phase = Phase::Bare;
        for (start, p) in YEAR {
            if s >= start {
                phase = p;
            }
        }
        phase
    }

    /// How much of the crown is out, 0..1 — the year in one number. Bare in
    /// winter, a quarter out at bud break, full from leaf-out to the end of
    /// fruiting, then falling through the seed drop. An evergreen species
    /// ignores this; that is the species' call, not the clock's.
    pub fn leaf_fullness(&self) -> f32 {
        let s = wrap(self.season);
        match s {
            s if s < 0.10 => ramp(s, 0.0, 0.10, 0.0, 0.25),
            s if s < 0.25 => ramp(s, 0.10, 0.25, 0.25, 1.0),
            s if s < 0.70 => 1.0,
            s if s < 0.85 => ramp(s, 0.70, 0.85, 1.0, 0.0),
            _ => 0.0,
        }
    }

    /// How far the leaves have turned, 0..1 — green through the growing
    /// season, gold by the time they drop.
    pub fn autumn(&self) -> f32 {
        ramp(wrap(self.season), 0.62, 0.82, 0.0, 1.0)
    }

    /// Is the plant old enough to carry blooms and fruit? A sapling puts
    /// everything it has into growing.
    pub fn bears(&self, seasons_to_grown: f32) -> bool {
        self.is_grown(seasons_to_grown)
    }
}

/// 0..1, wrapping: a year has no end.
fn wrap(s: f32) -> f32 {
    if !s.is_finite() {
        return 0.0;
    }
    let f = s - s.floor();
    if f < 0.0 {
        f + 1.0
    } else {
        f
    }
}

/// `x` from `a`..`b` mapped to `from`..`to`, flat outside.
fn ramp(x: f32, a: f32, b: f32, from: f32, to: f32) -> f32 {
    if x <= a {
        return from;
    }
    if x >= b {
        return to;
    }
    from + (to - from) * ((x - a) / (b - a))
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
    fn the_long_arc_turns_where_it_says() {
        let (grown, life) = (12.0, 160.0);
        assert_eq!(Clock::at(3.0).stage(grown, life), Stage::Sapling);
        assert_eq!(Clock::at(11.9).stage(grown, life), Stage::Sapling);
        assert_eq!(Clock::at(12.0).stage(grown, life), Stage::Mature);
        assert_eq!(Clock::at(100.0).stage(grown, life), Stage::Mature);
        assert_eq!(Clock::at(120.0).stage(grown, life), Stage::Old);
        assert_eq!(Clock::at(155.0).stage(grown, life), Stage::Dying);
        assert_eq!(Clock::at(1e6).stage(grown, life), Stage::Dying);
        // A recipe that says nothing about time shows a plant in its prime.
        assert_eq!(Clock::grown().stage(grown, life), Stage::Mature);
        // A species that outlives its own life curve never skips a stage.
        assert_eq!(Clock::at(20.0).stage(24.0, 24.0), Stage::Sapling);
    }

    #[test]
    fn the_year_goes_round() {
        assert_eq!(Clock::grown().in_season(0.02).phase(), Phase::Bud);
        assert_eq!(Clock::grown().in_season(0.15).phase(), Phase::Leaf);
        assert_eq!(Clock::grown().in_season(0.35).phase(), Phase::Bloom);
        assert_eq!(Clock::grown().in_season(0.5).phase(), Phase::Fruit);
        assert_eq!(Clock::grown().in_season(0.75).phase(), Phase::SeedDrop);
        assert_eq!(Clock::grown().in_season(0.9).phase(), Phase::Bare);
        // It wraps: next year is this year.
        assert_eq!(Clock::grown().in_season(1.35).phase(), Phase::Bloom);
        assert_eq!(Clock::grown().in_season(-0.05).phase(), Phase::Bare);
    }

    #[test]
    fn the_crown_comes_and_goes_with_the_year() {
        let at = |s: f32| Clock::grown().in_season(s).leaf_fullness();
        assert_eq!(at(0.92), 0.0, "bare in winter");
        assert!(at(0.05) > 0.0 && at(0.05) < 0.3, "budding");
        assert_eq!(at(0.35), 1.0, "full through bloom");
        assert_eq!(at(0.6), 1.0, "full through fruit");
        assert!(at(0.78) < 0.6 && at(0.78) > 0.0, "falling");
        // No steps: the crown fills and empties smoothly all the way round.
        let mut worst = 0.0f32;
        for k in 0..2000 {
            let a = at(k as f32 / 2000.0);
            let b = at((k + 1) as f32 / 2000.0);
            worst = worst.max((b - a).abs());
        }
        assert!(worst < 0.01, "the year should be continuous (worst step {worst})");
        // Green in summer, turned by the drop.
        assert_eq!(Clock::grown().in_season(0.4).autumn(), 0.0);
        assert_eq!(Clock::grown().in_season(0.83).autumn(), 1.0);
    }

    #[test]
    fn a_clock_round_trips_through_json() {
        let json = serde_json::to_string(&Clock::at(3.5)).unwrap();
        let back: Clock = serde_json::from_str(&json).unwrap();
        assert_eq!(back.age, Some(3.5));
        // An empty clock is a grown plant, so old recipes show what they always did.
        let empty: Clock = serde_json::from_str("{}").unwrap();
        assert!(empty.age.is_none());
        assert_eq!(serde_json::to_string(&Stage::Sapling).unwrap(), "\"sapling\"");
        assert_eq!(serde_json::to_string(&Phase::SeedDrop).unwrap(), "\"seeddrop\"");
    }
}
