//! Travelers — the people sharing a world, and where they are *now*.
//!
//! The client half of [presence-wire-v0.1]: a transport feeds
//! timestamped [`PoseSample`]s in per the presence-wire spec; this interpolates
//! each traveler ~100 ms in the past (lerp position, slerp rotation, extrapolate
//! with velocity across gaps) and produces [`EntityInstance`]s for any renderer.
//! Coordinate frame matches the manifest: right-handed, +Y up, metres, rotation
//! as a unit quaternion.
//!
//! [presence-wire-v0.1]: https://github.com/Pixygon/thread-spec/blob/main/specs/presence-wire-v0.1.md

use std::collections::HashMap;

use glam::{Mat4, Quat, Vec3};

/// One traveler, ready to draw: a world transform and a tint.
///
/// This is the entire surface presence needed from a renderer, and it is why
/// this module could leave the engine it grew up in — two fields, owned here,
/// so the Thread's presence implementation depends on no renderer at all. A
/// host converts it to whatever its own instance buffer wants.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Instance {
    pub model: Mat4,
    pub color: [f32; 4],
}

/// Render this far behind the newest pose, to hide network jitter (ms).
const INTERP_DELAY_MS: u64 = 100;
/// Cap velocity extrapolation across a gap before freezing (ms).
const MAX_EXTRAP_MS: u64 = 250;
/// Poses retained per traveler (a little over the interpolation window).
const MAX_BUFFER: usize = 16;

/// One sampled pose of a traveler. `t_ms` is a monotonic local arrival time —
/// the transport stamps it on receipt so interpolation shares one clock.
#[derive(Debug, Clone, Copy)]
pub struct PoseSample {
    pub t_ms: u64,
    pub pos: Vec3,
    pub rot: Quat,
    pub vel: Vec3,
    /// Locomotion/animation state (idle/walk/run/…) — used once avatars animate.
    pub anim: u8,
}

/// One traveler's render-ready pose this frame: interpolated transform, look,
/// portable body (0 = placeholder), and horizontal speed (drives walk vs idle).
pub struct TravelerPose {
    pub id: u32,
    /// Interpolated world position (feet), before any capsule lift.
    pub position: Vec3,
    pub model: Mat4,
    pub color: [f32; 4],
    pub body: u32,
    pub speed: f32,
}

/// A remote traveler: identity + a short history of poses to interpolate.
pub struct Traveler {
    pub id: u32,
    pub name: String,
    pub color: [f32; 4],
    /// The traveler's portable avatar (the shared `AvatarSpec` from their
    /// Passport descriptor) — the renderer swaps the placeholder capsule for
    /// their assembled body when this is present.
    pub avatar: Option<infinite_avatar::AvatarSpec>,
    buffer: Vec<PoseSample>,
}

impl Traveler {
    fn new(id: u32, name: String, color: [f32; 4]) -> Self {
        Self {
            id,
            name,
            color,
            avatar: None,
            buffer: Vec::new(),
        }
    }

    fn push(&mut self, sample: PoseSample) {
        // Poses arrive in order (relay-stamped); drop the odd out-of-order one.
        if self.buffer.last().is_some_and(|l| sample.t_ms < l.t_ms) {
            return;
        }
        self.buffer.push(sample);
        if self.buffer.len() > MAX_BUFFER {
            self.buffer.remove(0);
        }
    }

    /// Interpolated (position, rotation) at render time `render_t` (ms), or
    /// `None` if there's no pose yet.
    fn sample_at(&self, render_t: u64) -> Option<(Vec3, Quat)> {
        let first = self.buffer.first()?;
        let last = self.buffer.last()?;
        if render_t <= first.t_ms {
            return Some((first.pos, first.rot));
        }
        if render_t >= last.t_ms {
            // Extrapolate along the last velocity, capped, then freeze.
            let gap = (render_t - last.t_ms).min(MAX_EXTRAP_MS);
            return Some((last.pos + last.vel * (gap as f32 / 1000.0), last.rot));
        }
        // Interpolate within the bracketing pair.
        for pair in self.buffer.windows(2) {
            let (a, b) = (&pair[0], &pair[1]);
            if a.t_ms <= render_t && render_t < b.t_ms {
                let span = (b.t_ms - a.t_ms).max(1) as f32;
                let f = (render_t - a.t_ms) as f32 / span;
                return Some((a.pos.lerp(b.pos, f), a.rot.slerp(b.rot, f)));
            }
        }
        Some((last.pos, last.rot))
    }
}

/// All travelers currently present in the world.
#[derive(Default)]
pub struct PresenceState {
    travelers: HashMap<u32, Traveler>,
}

impl PresenceState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn count(&self) -> usize {
        self.travelers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.travelers.is_empty()
    }

    /// A traveler joined (or re-announced). Idempotent.
    pub fn join(&mut self, id: u32, name: impl Into<String>, color: [f32; 4]) {
        self.travelers
            .entry(id)
            .or_insert_with(|| Traveler::new(id, name.into(), color));
    }

    /// A traveler left.
    pub fn leave(&mut self, id: u32) {
        self.travelers.remove(&id);
    }

    /// Upgrade a traveler's look once their identity resolves (e.g. a Passport
    /// descriptor arrived): each `Some` replaces the placeholder. No-op for
    /// unknown ids.
    pub fn restyle(&mut self, id: u32, name: Option<String>, color: Option<[f32; 4]>) {
        if let Some(t) = self.travelers.get_mut(&id) {
            if let Some(n) = name {
                t.name = n;
            }
            if let Some(c) = color {
                t.color = c;
            }
        }
    }

    /// Attach a traveler's portable avatar (from their Passport descriptor).
    pub fn set_avatar(&mut self, id: u32, avatar: Option<infinite_avatar::AvatarSpec>) {
        if let Some(t) = self.travelers.get_mut(&id) {
            t.avatar = avatar;
        }
    }

    /// Record a pose for a traveler (auto-joins with a default look if unknown).
    pub fn pose(&mut self, id: u32, sample: PoseSample) {
        self.travelers
            .entry(id)
            .or_insert_with(|| Traveler::new(id, format!("traveler {id}"), default_color(id)))
            .push(sample);
    }

    /// Every traveler as a drawable [`Instance`], interpolated at `now_ms`.
    /// `foot_lift` raises a centered capsule so its feet meet the ground
    /// (match the player).
    pub fn entities(&self, now_ms: u64, foot_lift: f32) -> Vec<Instance> {
        self.poses(now_ms, foot_lift)
            .into_iter()
            .map(|p| Instance { model: p.model, color: p.color })
            .collect()
    }

    /// Every traveler's interpolated pose at `now_ms` — what the skinned
    /// character path consumes; [`Self::entities`] is the flat capsule view.
    pub fn poses(&self, now_ms: u64, foot_lift: f32) -> Vec<TravelerPose> {
        let render_t = now_ms.saturating_sub(INTERP_DELAY_MS);
        self.travelers
            .values()
            .filter_map(|t| {
                let (pos, rot) = t.sample_at(render_t)?;
                let model =
                    Mat4::from_rotation_translation(rot, pos + Vec3::new(0.0, foot_lift, 0.0));
                let body = t
                    .avatar
                    .as_ref()
                    .map(|s| s.get(infinite_avatar::AvatarSlot::Body))
                    .unwrap_or(0);
                let speed = t
                    .buffer
                    .last()
                    .map(|s| Vec3::new(s.vel.x, 0.0, s.vel.z).length())
                    .unwrap_or(0.0);
                Some(TravelerPose {
                    id: t.id,
                    position: pos,
                    model,
                    color: t.color,
                    body,
                    speed,
                })
            })
            .collect()
    }
}

/// A stable, distinct-ish colour per traveler id (until avatars carry real looks).
fn default_color(id: u32) -> [f32; 4] {
    let h = id.wrapping_mul(2654435761);
    [
        0.4 + ((h & 0xFF) as f32 / 255.0) * 0.5,
        0.4 + (((h >> 8) & 0xFF) as f32 / 255.0) * 0.5,
        0.4 + (((h >> 16) & 0xFF) as f32 / 255.0) * 0.5,
        1.0,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(t_ms: u64, pos: Vec3, vel: Vec3) -> PoseSample {
        PoseSample {
            t_ms,
            pos,
            rot: Quat::IDENTITY,
            vel,
            anim: 0,
        }
    }

    #[test]
    fn interpolates_between_poses() {
        let mut t = Traveler::new(1, "x".into(), [1.0; 4]);
        t.push(s(0, Vec3::ZERO, Vec3::ZERO));
        t.push(s(100, Vec3::new(10.0, 0.0, 0.0), Vec3::ZERO));
        let (p, _) = t.sample_at(50).unwrap();
        assert!((p.x - 5.0).abs() < 1e-4, "midpoint lerp, got {}", p.x);
        assert!((t.sample_at(0).unwrap().0.x - 0.0).abs() < 1e-4);
        assert!((t.sample_at(100).unwrap().0.x - 10.0).abs() < 1e-4);
    }

    #[test]
    fn extrapolates_along_velocity_then_freezes() {
        let mut t = Traveler::new(1, "x".into(), [1.0; 4]);
        t.push(s(0, Vec3::ZERO, Vec3::ZERO));
        t.push(s(100, Vec3::ZERO, Vec3::new(10.0, 0.0, 0.0)));
        // 50 ms past the last pose → 10 m/s * 0.05 s = 0.5 m.
        assert!((t.sample_at(150).unwrap().0.x - 0.5).abs() < 1e-4);
        // Far past → frozen at the cap (250 ms → 2.5 m).
        assert!((t.sample_at(5000).unwrap().0.x - 2.5).abs() < 1e-4);
    }

    #[test]
    fn restyle_upgrades_the_placeholder_look() {
        let mut p = PresenceState::new();
        p.pose(5, s(0, Vec3::ZERO, Vec3::ZERO)); // auto-joined with defaults
        p.restyle(5, Some("Anders".into()), Some([0.1, 0.2, 0.3, 1.0]));
        let t = p.travelers.get(&5).unwrap();
        assert_eq!(t.name, "Anders");
        assert_eq!(t.color, [0.1, 0.2, 0.3, 1.0]);
        // Unknown ids are ignored, partial restyles keep the rest.
        p.restyle(99, Some("ghost".into()), None);
        p.restyle(5, None, Some([1.0; 4]));
        assert_eq!(p.travelers.get(&5).unwrap().name, "Anders");
        assert_eq!(p.travelers.get(&5).unwrap().color, [1.0; 4]);
    }

    #[test]
    fn produces_one_entity_per_traveler() {
        let mut p = PresenceState::new();
        p.pose(1, s(1000, Vec3::new(3.0, 0.0, 0.0), Vec3::ZERO));
        p.pose(2, s(1000, Vec3::new(-3.0, 0.0, 0.0), Vec3::ZERO));
        assert_eq!(p.count(), 2);
        // now well past the poses → both render (frozen).
        let ents = p.entities(1100, 0.9);
        assert_eq!(ents.len(), 2);
        p.leave(1);
        assert_eq!(p.entities(1100, 0.9).len(), 1);
    }
}
