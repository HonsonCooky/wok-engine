//! The camera occlusion fade: per-drawn-item opacity state, advanced once per rendered frame.
//!
//! With the spring arm no longer clamping against prefabs (`crate::follow`), a prefab can sit
//! between the eye and the player. The answer is presentation, not camera motion: any drawn prefab
//! whose world AABB intersects the eye-to-anchor segment fades toward `OPACITY_OCCLUDED`, and
//! recovers to fully opaque once clear. The fade rides wok-render's per-item opacity (screen-door
//! cutout used as a fade), so an occluder still writes depth and casts its full shadow. Terrain
//! never fades (it is the ground the scene stands on, and the terrain floor already keeps the eye
//! above it), the player never fades (it is what the fade exists to reveal), and line overlays are
//! outside the mesh pass entirely.
//!
//! Association is by draw order: taste loads everything up front and never reloads, so the
//! prefab items iterate in the same store order every frame and the index is a stable identity.
//! (Per-placement association beyond this is explicitly out of scope.) The fade is render-side
//! state with frame dt, like the camera: presentation, never simulation, so the replay contract
//! is untouched. All the logic is pure (the segment test and the constant-rate approach), and the
//! visual itself is the play test.

use glam::Vec3;
use wok_scene::Aabb;

use crate::constants::{OCCLUSION_FADE_S, OPACITY_OCCLUDED};

/// Does the segment from `start` to `end` pass through `aabb`? The slab test, restricted to the
/// segment's parameter range: clip `t` in `0..=1` against the box's three axis slabs and ask
/// whether a range survives. Closed on every boundary - a segment grazing a face counts as
/// intersecting - and total: a degenerate (zero-length) segment is a point-in-box test.
pub fn segment_hits_aabb(start: Vec3, end: Vec3, aabb: &Aabb) -> bool {
    let d = end - start;
    let mut t_min = 0.0_f32;
    let mut t_max = 1.0_f32;
    for axis in 0..3 {
        let (s, dir, min, max) = (start[axis], d[axis], aabb.min[axis], aabb.max[axis]);
        if dir == 0.0 {
            // Parallel to this slab: either always inside it or never.
            if s < min || s > max {
                return false;
            }
        } else {
            let t1 = (min - s) / dir;
            let t2 = (max - s) / dir;
            let (lo, hi) = if t1 <= t2 { (t1, t2) } else { (t2, t1) };
            t_min = t_min.max(lo);
            t_max = t_max.min(hi);
            if t_min > t_max {
                return false;
            }
        }
    }
    true
}

/// Move `current` toward `target` by at most `max_delta`, arriving exactly - the same
/// constant-rate approach locomotion uses (`crate::sim`), scalar: no asymptote, so a fade reaches
/// exactly `OPACITY_OCCLUDED` and a recovery exactly 1.0 in finite frames.
fn approach(current: f32, target: f32, max_delta: f32) -> f32 {
    let gap = target - current;
    if gap.abs() <= max_delta { target } else { current + max_delta.copysign(gap) }
}

/// Per-item fade state across frames, indexed by draw order (see the module docs). New indices
/// start fully opaque, so an item fades in from 1.0 the first frame it occludes rather than
/// popping to the faded value.
pub struct OcclusionFade {
    opacities: Vec<f32>,
}

impl OcclusionFade {
    pub fn new() -> OcclusionFade {
        OcclusionFade { opacities: Vec::new() }
    }

    /// Advance item `index` one frame toward its target - `OPACITY_OCCLUDED` while `occluded`,
    /// 1.0 when clear - at the constant rate that spans the full fade in `OCCLUSION_FADE_S`
    /// seconds, and return the opacity to draw with this frame.
    pub fn advance(&mut self, index: usize, occluded: bool, dt: f32) -> f32 {
        if index >= self.opacities.len() {
            self.opacities.resize(index + 1, 1.0);
        }
        let target = if occluded { OPACITY_OCCLUDED } else { 1.0 };
        let rate = (1.0 - OPACITY_OCCLUDED) / OCCLUSION_FADE_S;
        let next = approach(self.opacities[index], target, rate * dt.max(0.0));
        self.opacities[index] = next;
        next
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 60.0;

    fn unit_box() -> Aabb {
        Aabb::new(Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0))
    }

    // ---- segment vs AABB ----

    #[test]
    fn a_segment_through_the_box_intersects() {
        assert!(segment_hits_aabb(Vec3::new(0.0, 0.0, -5.0), Vec3::new(0.0, 0.0, 5.0), &unit_box()));
        // And a diagonal through a corner region.
        assert!(segment_hits_aabb(Vec3::new(-3.0, -3.0, -3.0), Vec3::new(3.0, 3.0, 3.0), &unit_box()));
    }

    #[test]
    fn a_segment_beside_the_box_misses() {
        // Parallel to z but outside the x slab: the camera line passing a crate to its side.
        assert!(!segment_hits_aabb(Vec3::new(2.0, 0.0, -5.0), Vec3::new(2.0, 0.0, 5.0), &unit_box()));
    }

    #[test]
    fn a_segment_that_stops_short_of_the_box_misses() {
        // The line through the box intersects; the segment matters: both endpoints before it.
        assert!(!segment_hits_aabb(Vec3::new(0.0, 0.0, -5.0), Vec3::new(0.0, 0.0, -2.0), &unit_box()));
        // And both past it.
        assert!(!segment_hits_aabb(Vec3::new(0.0, 0.0, 2.0), Vec3::new(0.0, 0.0, 5.0), &unit_box()));
    }

    #[test]
    fn an_endpoint_inside_the_box_intersects() {
        // The eye or the anchor sitting inside an occluder still counts as occluded.
        assert!(segment_hits_aabb(Vec3::ZERO, Vec3::new(0.0, 0.0, 9.0), &unit_box()));
        assert!(segment_hits_aabb(Vec3::new(0.0, 0.0, -9.0), Vec3::ZERO, &unit_box()));
    }

    #[test]
    fn a_grazing_segment_touches_the_closed_boundary() {
        // Sliding exactly along the +X face: the slabs are closed, so the graze intersects.
        assert!(segment_hits_aabb(Vec3::new(1.0, 0.0, -5.0), Vec3::new(1.0, 0.0, 5.0), &unit_box()));
        // Just past the face misses.
        assert!(!segment_hits_aabb(Vec3::new(1.0 + 1e-5, 0.0, -5.0), Vec3::new(1.0 + 1e-5, 0.0, 5.0), &unit_box()));
    }

    #[test]
    fn a_degenerate_segment_is_a_point_test() {
        assert!(segment_hits_aabb(Vec3::ZERO, Vec3::ZERO, &unit_box()));
        assert!(!segment_hits_aabb(Vec3::new(3.0, 0.0, 0.0), Vec3::new(3.0, 0.0, 0.0), &unit_box()));
    }

    // ---- the fade state ----

    #[test]
    fn an_occluded_item_reaches_the_faded_opacity_in_the_fade_time_exactly() {
        let mut fade = OcclusionFade::new();
        // The approach arrives exactly: after ceil(OCCLUSION_FADE_S / DT) frames occluded, the
        // opacity is OPACITY_OCCLUDED itself, not an asymptotic near-miss.
        let frames = (OCCLUSION_FADE_S / DT).ceil() as usize;
        let mut last = 1.0;
        for _ in 0..frames {
            last = fade.advance(0, true, DT);
        }
        assert_eq!(last, OPACITY_OCCLUDED);
        // And it holds there, never overshooting below.
        assert_eq!(fade.advance(0, true, DT), OPACITY_OCCLUDED);
    }

    #[test]
    fn a_cleared_item_recovers_to_exactly_opaque() {
        let mut fade = OcclusionFade::new();
        for _ in 0..30 {
            fade.advance(0, true, DT);
        }
        let frames = (OCCLUSION_FADE_S / DT).ceil() as usize;
        let mut last = 0.0;
        for _ in 0..frames {
            last = fade.advance(0, false, DT);
        }
        assert_eq!(last, 1.0, "recovery must arrive at exactly opaque");
    }

    #[test]
    fn the_fade_moves_gradually_not_as_a_pop() {
        let mut fade = OcclusionFade::new();
        let first = fade.advance(0, true, DT);
        assert!(first < 1.0, "the first occluded frame starts fading");
        assert!(
            first > OPACITY_OCCLUDED,
            "one frame must not complete a {OCCLUSION_FADE_S}s fade: {first}"
        );
        // Reversing mid-fade walks back up the same way.
        let second = fade.advance(0, false, DT);
        assert!(second > first && second <= 1.0, "{first} -> {second}");
    }

    #[test]
    fn items_fade_independently_and_new_indices_start_opaque() {
        let mut fade = OcclusionFade::new();
        fade.advance(0, true, DT);
        fade.advance(0, true, DT);
        // Item 5 appears later: it starts from 1.0, untouched by item 0's fade.
        let fresh = fade.advance(5, false, DT);
        assert_eq!(fresh, 1.0);
        let faded = fade.advance(0, true, DT);
        assert!(faded < fresh);
    }

    #[test]
    fn advance_is_deterministic() {
        let run = || {
            let mut fade = OcclusionFade::new();
            let mut out = Vec::new();
            for i in 0..20 {
                out.push(fade.advance(0, i % 3 != 0, DT));
            }
            out
        };
        assert_eq!(run(), run());
    }
}
