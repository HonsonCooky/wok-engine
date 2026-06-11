//! Swept capsule vs AABB: the earliest time-of-impact of a moving capsule against static boxes.
//!
//! This is the core continuous-collision primitive the character controller rests on. Given a
//! [`Capsule`], a motion `delta`, and a static [`Aabb`], [`sweep_capsule_aabb`] reports the first
//! fraction of the motion at which the capsule touches the box, the outward contact normal there,
//! and the contact point, or `None` if the motion never brings them together.
//!
//! ## Method: conservative advancement
//!
//! The capsule reduces to its segment plus a radius `r`; impact is when the segment-to-box distance
//! falls to `r`. Define `gap(t) = dist(segment + delta*t, box) - r`. We want the first `t` in
//! `[0, 1]` with `gap(t) = 0`.
//!
//! Translating one convex shape past another makes that distance a convex function of `t` (the
//! distance from a point to the convex Minkowski difference, sampled along a line). A convex
//! function lies above its tangents, so stepping to where the current tangent predicts contact -
//! advancing by `gap / closing_speed`, where `closing_speed` is the approach rate along the current
//! closest direction - always lands with `gap` still non-negative: it never tunnels. Iterating
//! drives `gap` to zero from above. Flat-face contact converges in one step (the direction is
//! constant); rounded edge/corner contact takes a few.
//!
//! A motion that is not closing the gap (moving away, or grazing parallel) reports no hit: that is
//! both correct - the capsule never reaches the box - and what lets [`crate::slide`] keep moving a
//! capsule that is already flush against a wall.
//!
//! The advancement loop itself is shape-agnostic: it needs only an exact closest-point pair against
//! a convex static and a fallback normal for the segment-inside case. It is factored as
//! [`advance_capsule`] so the vertical-cylinder sweep ([`crate::sweep_round`]) runs the identical
//! machinery against its own projection; this module instantiates it for the [`Aabb`] and owns the
//! earliest-impact dispatch over mixed [`Collider`] sets.
//!
//! Determinism (canon contract): a fixed iteration order with a fixed cap, no RNG, no parallel
//! reduction; the multi-collider sweep takes the earliest impact and breaks ties by slice order.
//! The math reads only relative positions, so it is position-independent to float precision.

use glam::Vec3;
use wok_scene::Aabb;

use crate::capsule::Capsule;
use crate::collider::Collider;
use crate::geom::closest_points_segment_aabb;
use crate::sweep_obb::sweep_capsule_obb_inflated;
use crate::sweep_round::{sweep_capsule_cylinder_inflated, sweep_capsule_sphere_inflated};

/// Below this distance the segment is touching or inside the static shape and the closest-points
/// direction is unreliable; we fall back to the shape's own deep normal (see [`face_normal`]).
pub(crate) const TOUCHING: f32 = 1e-6;

/// Approach rates at or below this count as "not closing": the motion is parallel to or receding
/// from the shape, so no impact. Also guards the `gap / closing` divide.
pub(crate) const CLOSING_EPS: f32 = 1e-8;

/// Impact is declared once the gap is within this of zero. The resulting `toi` sits a hair before
/// true contact (the capsule surface is `GAP_EPS` from the shape), far tighter than the response
/// needs.
pub(crate) const GAP_EPS: f32 = 1e-5;

/// Below this squared length the motion is treated as zero: a capsule that does not move cannot
/// sweep into anything.
pub(crate) const MOTION_EPS_SQ: f32 = 1e-12;

/// Cap on advancement steps. Convergence is fast for the contacts a player meets; the cap only
/// bounds a degenerate grazing approach, which is reported as no hit.
const MAX_STEPS: usize = 32;

/// The result of a capsule sweep that found contact.
///
/// `toi` is the fraction of `delta` travelled before impact (`0.0` already touching, `1.0` at the
/// very end of the motion). `normal` is the unit outward contact normal: it points from the box
/// toward the capsule, the direction to push out along and the plane to slide on. `point` is the
/// contact point on the box surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SweptHit {
    pub toi: f32,
    pub normal: Vec3,
    pub point: Vec3,
}

/// Sweep `capsule` through `delta` against a single static `aabb`; `None` if it never makes contact.
///
/// See the module docs for the method. The reported `toi` is the true geometric time of impact (no
/// skin); [`crate::slide`] applies its own skin via [`sweep_capsule_aabbs_inflated`].
pub fn sweep_capsule_aabb(capsule: &Capsule, delta: Vec3, aabb: &Aabb) -> Option<SweptHit> {
    sweep_one(capsule, delta, aabb, 0.0)
}

/// Sweep `capsule` through `delta` against many static boxes, returning the earliest impact.
///
/// Boxes are visited in slice order and the smallest `toi` wins; an exact tie keeps the
/// earlier-indexed box. That fixed order is the determinism contract's "defined order" for
/// resolution over several colliders.
pub fn sweep_capsule_aabbs(capsule: &Capsule, delta: Vec3, aabbs: &[Aabb]) -> Option<SweptHit> {
    sweep_capsule_aabbs_inflated(capsule, delta, aabbs, 0.0)
}

/// Earliest-impact sweep with the capsule radius inflated by `skin`, so the capsule stops `skin`
/// short of every surface. [`crate::slide`] uses this to keep a small, robust separation while
/// sliding; with `skin = 0.0` it is exactly [`sweep_capsule_aabbs`].
pub(crate) fn sweep_capsule_aabbs_inflated(
    capsule: &Capsule,
    delta: Vec3,
    aabbs: &[Aabb],
    skin: f32,
) -> Option<SweptHit> {
    let mut best: Option<SweptHit> = None;
    for aabb in aabbs {
        if let Some(hit) = sweep_one(capsule, delta, aabb, skin) {
            // Earliest impact wins; an exact tie keeps the earlier-indexed box (already in `best`).
            match best {
                Some(b) if b.toi <= hit.toi => {}
                _ => best = Some(hit),
            }
        }
    }
    best
}

/// Sweep `capsule` through `delta` against one static [`Collider`], dispatching to the shape's own
/// sweep; `None` if it never makes contact.
pub fn sweep_capsule_collider(capsule: &Capsule, delta: Vec3, collider: &Collider) -> Option<SweptHit> {
    sweep_collider_one(capsule, delta, collider, 0.0)
}

/// Sweep `capsule` through `delta` against a mixed set of static colliders, returning the earliest
/// impact: the same slice-order, earliest-toi-wins contract as [`sweep_capsule_aabbs`].
pub fn sweep_capsule_colliders(capsule: &Capsule, delta: Vec3, colliders: &[Collider]) -> Option<SweptHit> {
    sweep_capsule_colliders_inflated(capsule, delta, colliders, 0.0)
}

/// Earliest-impact collider sweep with the capsule radius inflated by `skin`: what
/// [`crate::slide`] runs each iteration. With `skin = 0.0` it is exactly
/// [`sweep_capsule_colliders`].
pub(crate) fn sweep_capsule_colliders_inflated(
    capsule: &Capsule,
    delta: Vec3,
    colliders: &[Collider],
    skin: f32,
) -> Option<SweptHit> {
    let mut best: Option<SweptHit> = None;
    for collider in colliders {
        if let Some(hit) = sweep_collider_one(capsule, delta, collider, skin) {
            // Earliest impact wins; an exact tie keeps the earlier-indexed collider.
            match best {
                Some(b) if b.toi <= hit.toi => {}
                _ => best = Some(hit),
            }
        }
    }
    best
}

/// One collider, one sweep: the per-shape dispatch every multi-collider query funnels through.
fn sweep_collider_one(capsule: &Capsule, delta: Vec3, collider: &Collider, skin: f32) -> Option<SweptHit> {
    match *collider {
        Collider::Aabb(ref aabb) => sweep_one(capsule, delta, aabb, skin),
        Collider::Sphere { center, radius } => sweep_capsule_sphere_inflated(capsule, delta, center, radius, skin),
        Collider::VertCylinder { center, radius, half_height } => {
            sweep_capsule_cylinder_inflated(capsule, delta, center, radius, half_height, skin)
        }
        Collider::Obb { center, half_extents, rotation } => {
            sweep_capsule_obb_inflated(capsule, delta, center, half_extents, rotation, skin)
        }
    }
}

/// Conservative advancement of one capsule against one box, with the radius inflated by `skin`.
fn sweep_one(capsule: &Capsule, delta: Vec3, aabb: &Aabb, skin: f32) -> Option<SweptHit> {
    advance_capsule(
        capsule,
        delta,
        capsule.radius + skin,
        |a, b| closest_points_segment_aabb(a, b, aabb),
        |p| face_normal(aabb, p),
    )
}

/// Conservative advancement of one capsule against one convex static shape, the shape supplied as
/// its exact closest-point pairing (`closest`, segment endpoints in, `(on_segment, on_shape)` out)
/// and a fallback outward normal for the segment-touching-or-inside case (`deep_normal`, given the
/// segment point). `radius` is the capsule radius, already inflated by whatever skin the caller
/// keeps. The convexity argument in the module docs is shape-generic, so so is the loop.
pub(crate) fn advance_capsule(
    capsule: &Capsule,
    delta: Vec3,
    radius: f32,
    closest: impl Fn(Vec3, Vec3) -> (Vec3, Vec3),
    deep_normal: impl Fn(Vec3) -> Vec3,
) -> Option<SweptHit> {
    if delta.length_squared() <= MOTION_EPS_SQ {
        return None;
    }
    let mut a = capsule.a;
    let mut b = capsule.b;
    let mut toi = 0.0_f32;

    for _ in 0..MAX_STEPS {
        let (on_segment, on_shape) = closest(a, b);
        let to_shape = on_shape - on_segment;
        let dist = to_shape.length();

        if dist <= TOUCHING {
            // Segment is touching or inside the shape: the closest direction is degenerate, so use
            // the shape's deep normal to give a sane push-out direction.
            let normal = deep_normal(on_segment);
            return Some(SweptHit { toi, normal, point: on_shape });
        }

        let toward_shape = to_shape / dist;
        let closing = delta.dot(toward_shape);
        if closing <= CLOSING_EPS {
            // Moving away from or parallel to the shape: the gap never closes, so no impact.
            return None;
        }

        let gap = dist - radius;
        if gap <= GAP_EPS {
            // Capsule surface has reached the shape: outward normal points shape -> capsule.
            return Some(SweptHit { toi, normal: -toward_shape, point: on_shape });
        }

        // Advance to where the current tangent predicts contact. Convexity guarantees this does not
        // overshoot the true impact (see module docs).
        let step = gap / closing;
        toi += step;
        if toi > 1.0 {
            return None;
        }
        let advance = delta * step;
        a += advance;
        b += advance;
    }

    // Did not converge within the cap: a grazing approach the controller can treat as a miss.
    None
}

/// Outward normal of the box face nearest to `p` (which is on or inside the box), used only for the
/// degenerate touching/inside case. Picks the least-penetration axis with the fixed tie order
/// `-x, +x, -y, +y, -z, +z`, mirroring part 1's MTV tie-break. `pub(crate)` so the oriented-box
/// sweep ([`crate::sweep_obb`]) reuses it against its local box.
pub(crate) fn face_normal(aabb: &Aabb, p: Vec3) -> Vec3 {
    let candidates = [
        (p.x - aabb.min.x, Vec3::NEG_X),
        (aabb.max.x - p.x, Vec3::X),
        (p.y - aabb.min.y, Vec3::NEG_Y),
        (aabb.max.y - p.y, Vec3::Y),
        (p.z - aabb.min.z, Vec3::NEG_Z),
        (aabb.max.z - p.z, Vec3::Z),
    ];
    let mut best = candidates[0];
    for &c in &candidates[1..] {
        if c.0 < best.0 {
            best = c;
        }
    }
    best.1
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    // A standing player capsule: feet at `feet`, 2m tall, 0.5m radius.
    fn player(feet: Vec3) -> Capsule {
        Capsule::upright(feet + Vec3::new(0.0, 1.0, 0.0), 2.0, 0.5)
    }

    #[test]
    fn capsule_moving_toward_a_wall_stops_at_the_expected_toi() {
        // Player at x = 0, wall face at x = 3. Radius 0.5, so contact when the centre reaches
        // x = 2.5; moving +5 in x that is toi = 0.5. Normal faces back along -x.
        let c = player(Vec3::ZERO);
        let wall = Aabb::new(Vec3::new(3.0, 0.0, -1.0), Vec3::new(4.0, 3.0, 1.0));
        let hit = sweep_capsule_aabb(&c, Vec3::new(5.0, 0.0, 0.0), &wall).expect("should hit");
        assert!((hit.toi - 0.5).abs() < 1e-3, "toi = {}", hit.toi);
        assert!((hit.normal - Vec3::NEG_X).length() < 1e-4, "normal = {:?}", hit.normal);
    }

    #[test]
    fn capsule_moving_away_reports_no_hit() {
        let c = player(Vec3::ZERO);
        let wall = Aabb::new(Vec3::new(3.0, 0.0, -1.0), Vec3::new(4.0, 3.0, 1.0));
        // Moving -x, directly away from the wall.
        assert!(sweep_capsule_aabb(&c, Vec3::new(-5.0, 0.0, 0.0), &wall).is_none());
    }

    #[test]
    fn capsule_moving_parallel_reports_no_hit() {
        // Beside the wall with 0.5m clearance (centre x = 2.0, +x extent 2.5, wall face at 3.0),
        // sliding along z. The x gap never closes, so no impact.
        let c = player(Vec3::new(2.0, 0.0, 0.0));
        let wall = Aabb::new(Vec3::new(3.0, 0.0, -1.0), Vec3::new(4.0, 3.0, 1.0));
        assert!(sweep_capsule_aabb(&c, Vec3::new(0.0, 0.0, 10.0), &wall).is_none());
    }

    #[test]
    fn motion_too_short_to_reach_misses() {
        // Wall face at x = 3, contact needs centre at 2.5 (a +2.5 move); only moving +1.
        let c = player(Vec3::ZERO);
        let wall = Aabb::new(Vec3::new(3.0, 0.0, -1.0), Vec3::new(4.0, 3.0, 1.0));
        assert!(sweep_capsule_aabb(&c, Vec3::new(1.0, 0.0, 0.0), &wall).is_none());
    }

    #[test]
    fn capsule_falling_onto_a_floor_hits_with_an_up_normal() {
        // Feet at y = 2, floor top at y = 0. Contact when feet reach 0: a -2 move, toi = 0.5 of -4.
        let c = player(Vec3::new(0.0, 2.0, 0.0));
        let floor = Aabb::new(Vec3::new(-5.0, -1.0, -5.0), Vec3::new(5.0, 0.0, 5.0));
        let hit = sweep_capsule_aabb(&c, Vec3::new(0.0, -4.0, 0.0), &floor).expect("should land");
        assert!((hit.toi - 0.5).abs() < 1e-3, "toi = {}", hit.toi);
        assert!((hit.normal - Vec3::Y).length() < 1e-4, "normal = {:?}", hit.normal);
    }

    #[test]
    fn zero_motion_never_hits() {
        let c = player(Vec3::ZERO);
        let wall = Aabb::new(Vec3::new(0.2, 0.0, -1.0), Vec3::new(1.0, 3.0, 1.0));
        assert!(sweep_capsule_aabb(&c, Vec3::ZERO, &wall).is_none());
    }

    #[test]
    fn earliest_of_several_boxes_wins() {
        let c = player(Vec3::ZERO);
        let near = Aabb::new(Vec3::new(3.0, 0.0, -1.0), Vec3::new(4.0, 3.0, 1.0)); // contact toi 0.5
        let far = Aabb::new(Vec3::new(6.0, 0.0, -1.0), Vec3::new(7.0, 3.0, 1.0)); // contact toi 1.1 (miss in 5)
        let hit = sweep_capsule_aabbs(&c, Vec3::new(5.0, 0.0, 0.0), &[far, near]).expect("hits near");
        assert!((hit.toi - 0.5).abs() < 1e-3, "toi = {}", hit.toi);
        assert!((hit.point.x - 3.0).abs() < 1e-3, "contact on near box face, x = {}", hit.point.x);
    }

    #[test]
    fn sweep_is_deterministic() {
        let c = player(Vec3::ZERO);
        let wall = Aabb::new(Vec3::new(3.0, 0.0, -1.0), Vec3::new(4.0, 3.0, 1.0));
        let d = Vec3::new(5.0, -0.3, 0.2);
        assert_eq!(sweep_capsule_aabb(&c, d, &wall), sweep_capsule_aabb(&c, d, &wall));
    }

    #[test]
    fn capsule_starting_inside_a_box_resolves_at_toi_zero() {
        // Degenerate input the slide should not normally produce, but must handle: the segment is
        // already inside the box. The sweep reports contact at toi 0 with a finite, unit face
        // normal rather than dividing by a zero distance.
        let c = Capsule::new(Vec3::new(0.0, -0.5, 0.0), Vec3::new(0.0, 0.5, 0.0), 0.5);
        let box_around = Aabb::new(Vec3::splat(-2.0), Vec3::splat(2.0));
        let hit = sweep_capsule_aabb(&c, Vec3::new(5.0, 0.0, 0.0), &box_around).expect("overlap is a hit");
        assert_eq!(hit.toi, 0.0);
        assert!(hit.normal.is_finite(), "normal must be finite, got {:?}", hit.normal);
        assert!((hit.normal.length() - 1.0).abs() < 1e-5, "normal must be unit, len {}", hit.normal.length());
    }

    #[test]
    fn the_earliest_impact_wins_across_mixed_collider_shapes() {
        // The dispatch's contract is the AABB multi-sweep's: earliest toi over the slice, in slice
        // order. A sphere in front of a box must win, whichever the slice lists first.
        let c = player(Vec3::ZERO);
        let near_sphere = Collider::Sphere { center: Vec3::new(3.0, 1.0, 0.0), radius: 1.0 }; // contact toi 0.3
        let far_box = Collider::Aabb(Aabb::new(Vec3::new(6.0, 0.0, -1.0), Vec3::new(7.0, 3.0, 1.0)));
        let hit = sweep_capsule_colliders(&c, Vec3::new(5.0, 0.0, 0.0), &[far_box, near_sphere])
            .expect("hits the sphere");
        assert!((hit.toi - 0.3).abs() < 1e-4, "toi = {}", hit.toi);
        assert!((hit.normal - Vec3::NEG_X).length() < 1e-4, "normal = {:?}", hit.normal);
    }

    #[test]
    fn zero_radius_capsule_sweeps_as_a_segment() {
        // Radius 0: contact is when the segment itself reaches the wall face at x = 3 (toi 0.6 of 5).
        let c = Capsule::new(Vec3::new(0.0, 0.5, 0.0), Vec3::new(0.0, 1.5, 0.0), 0.0);
        let wall = Aabb::new(Vec3::new(3.0, 0.0, -1.0), Vec3::new(4.0, 3.0, 1.0));
        let hit = sweep_capsule_aabb(&c, Vec3::new(5.0, 0.0, 0.0), &wall).expect("segment reaches wall");
        assert!((hit.toi - 0.6).abs() < 1e-3, "toi = {}", hit.toi);
    }
}
