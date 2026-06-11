//! Swept capsule vs an oriented box: the box sweep, run in the box's own frame.
//!
//! A rotated capsule is still a capsule - a rigid map carries the segment to a segment and leaves
//! the radius alone - so the sweep maps the capsule and its motion into the box's local frame
//! (where the box is the axis-aligned `[-half_extents, +half_extents]`), runs the identical exact
//! conservative advancement the AABB sweep uses ([`crate::sweep::advance_capsule`] over
//! [`crate::geom::closest_points_segment_aabb`]), and maps the contact back. Nothing is
//! approximated: the local query is the part 2a box sweep verbatim, and the frame change is exact
//! geometry, not a bound.
//!
//! Mapping the contact back: `toi` is a fraction of the motion, invariant under any rigid map, so
//! it passes through untouched. The contact point is a point, so it takes the full map (rotate,
//! then add the centre). The normal takes only the rotation: a normal transforms by the inverse
//! transpose of the frame map, and for a rotation that is the rotation itself - which is exactly
//! why the per-axis scale lives in `half_extents` rather than in the frame map. Were the map
//! non-rigid, rotating the local normal back would bend it off the true surface normal.
//!
//! Determinism (canon contract): the advancement loop's fixed order and cap, plus one quaternion
//! conjugation per endpoint - fixed arithmetic, no RNG, no parallelism. The map reads only
//! positions relative to the box centre, so the query is position-independent to float precision.

use glam::{Quat, Vec3};
use wok_scene::Aabb;

use crate::capsule::Capsule;
use crate::geom::closest_points_segment_aabb;
use crate::sweep::{SweptHit, advance_capsule, face_normal};

/// Sweep `capsule` through `delta` against a static oriented box; `None` if it never makes
/// contact. `rotation` must be a unit quaternion (classification produces one).
pub fn sweep_capsule_obb(
    capsule: &Capsule,
    delta: Vec3,
    center: Vec3,
    half_extents: Vec3,
    rotation: Quat,
) -> Option<SweptHit> {
    sweep_capsule_obb_inflated(capsule, delta, center, half_extents, rotation, 0.0)
}

/// The oriented-box sweep with the capsule radius inflated by `skin` ([`crate::slide`]'s
/// separation): the box's local frame, the box sweep's advancement, the contact mapped back.
pub(crate) fn sweep_capsule_obb_inflated(
    capsule: &Capsule,
    delta: Vec3,
    center: Vec3,
    half_extents: Vec3,
    rotation: Quat,
    skin: f32,
) -> Option<SweptHit> {
    // Into the box frame: the conjugate is the unit quaternion's exact inverse. A rigid map, so
    // the capsule stays a capsule of the same radius.
    let inv = rotation.conjugate();
    let local_capsule = Capsule::new(inv * (capsule.a - center), inv * (capsule.b - center), capsule.radius);
    let local_delta = inv * delta;
    let local_box = Aabb::from_center_extents(Vec3::ZERO, half_extents);

    let hit = advance_capsule(
        &local_capsule,
        local_delta,
        capsule.radius + skin,
        |a, b| closest_points_segment_aabb(a, b, &local_box),
        |p| face_normal(&local_box, p),
    )?;
    Some(SweptHit {
        toi: hit.toi,
        normal: rotation * hit.normal,
        point: rotation * hit.point + center,
    })
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_4;

    // The standing player capsule the other sweeps' tests use: feet at `feet`, 2m tall, 0.5m
    // radius.
    fn player(feet: Vec3) -> Capsule {
        Capsule::upright(feet + Vec3::new(0.0, 1.0, 0.0), 2.0, 0.5)
    }

    #[test]
    fn capsule_moving_at_a_rotated_face_stops_at_the_analytic_toi() {
        // A tall box yawed 30 degrees, approached head-on along its own +x face normal n. The
        // capsule centre starts on the face's axis 5m from the box centre; contact when the
        // centre is half-extent + radius (1.5) from the centre along n: a 3.5m advance of 5,
        // toi = 0.7 exactly (a flat face converges in one advancement step). The normal must come
        // back as n itself - the rotated face's true normal, not a world axis.
        let rotation = Quat::from_rotation_y(0.5);
        let n = rotation * Vec3::X;
        let center = Vec3::new(0.0, 1.0, 0.0);
        let start = center + n * 5.0;
        let c = Capsule::upright(start, 2.0, 0.5);
        let hit = sweep_capsule_obb(&c, -n * 5.0, center, Vec3::new(1.0, 3.0, 1.0), rotation)
            .expect("should hit the rotated face");
        assert!((hit.toi - 0.7).abs() < 1e-3, "toi = {}", hit.toi);
        assert!((hit.normal - n).length() < 1e-4, "normal = {:?}, expected {n:?}", hit.normal);
        // The contact point lies on the face plane: half-extent along n from the centre.
        assert!(((hit.point - center).dot(n) - 1.0).abs() < 1e-3, "point off the face: {:?}", hit.point);
    }

    #[test]
    fn capsule_moving_at_a_yawed_corner_column_stops_at_the_edge() {
        // A 45-degree yaw points one vertical edge of the box straight at the capsule: the edge
        // column sits sqrt(2) before the centre along -x. Contact when the segment is the radius
        // from the edge: x = 5 - sqrt(2) - 0.5, toi of a +5 move from x = 0.
        let rotation = Quat::from_rotation_y(FRAC_PI_4);
        let c = player(Vec3::ZERO);
        let hit = sweep_capsule_obb(&c, Vec3::new(5.0, 0.0, 0.0), Vec3::new(5.0, 1.0, 0.0), Vec3::ONE, rotation)
            .expect("should hit the edge");
        let expected = (5.0 - std::f32::consts::SQRT_2 - 0.5) / 5.0;
        assert!((hit.toi - expected).abs() < 1e-3, "toi = {}, expected {expected}", hit.toi);
        assert!((hit.normal - Vec3::NEG_X).length() < 1e-3, "edge normal points back: {:?}", hit.normal);
    }

    #[test]
    fn obb_out_of_reach_behind_or_grazing_misses() {
        let rotation = Quat::from_rotation_y(0.5);
        let (center, he) = (Vec3::new(5.0, 1.0, 0.0), Vec3::ONE);
        let c = player(Vec3::ZERO);
        assert!(sweep_capsule_obb(&c, Vec3::new(1.0, 0.0, 0.0), center, he, rotation).is_none(), "too short");
        assert!(sweep_capsule_obb(&c, Vec3::new(-5.0, 0.0, 0.0), center, he, rotation).is_none(), "moving away");
        assert!(sweep_capsule_obb(&c, Vec3::ZERO, center, he, rotation).is_none(), "zero motion");
        // A graze: passing beside the box with the lateral gap a hair open never closes it.
        let beside = player(Vec3::new(0.0, 0.0, 2.5));
        assert!(
            sweep_capsule_obb(&beside, Vec3::new(10.0, 0.0, 0.0), center, he, rotation).is_none(),
            "a graze just outside must miss"
        );
    }

    #[test]
    fn segment_inside_the_obb_resolves_at_toi_zero_with_a_finite_unit_normal() {
        // Deep overlap: the local box's face-normal fallback answers, rotated back to the world.
        let rotation = Quat::from_rotation_y(0.5);
        let c = player(Vec3::new(0.0, 0.5, 0.0));
        let hit = sweep_capsule_obb(&c, Vec3::new(5.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0), Vec3::splat(2.0), rotation)
            .expect("overlap is a hit");
        assert_eq!(hit.toi, 0.0);
        assert!(hit.normal.is_finite(), "normal must be finite: {:?}", hit.normal);
        assert!((hit.normal.length() - 1.0).abs() < 1e-5, "normal must be unit: {:?}", hit.normal);
    }

    #[test]
    fn the_phantom_shelf_is_gone_outside_a_yawed_face() {
        // The finding that parked OBBs, inverted: a capsule descending just outside the rotated
        // face, at a spot the old conservative world AABB still covered (the cut-off corner
        // region). Against the honest oriented box the descent finds nothing; against the part 1
        // conservative box the same descent found the invisible shelf. Box: half-extent 1, yawed
        // 45 degrees, top at y = 2 - its world AABB reaches sqrt(2) in x/z, the drawn faces only
        // 1/sqrt(2) at this azimuth.
        let rotation = Quat::from_rotation_y(FRAC_PI_4);
        let center = Vec3::new(0.0, 1.0, 0.0);
        let he = Vec3::ONE;
        let c = player(Vec3::new(1.3, 2.5, 1.3)); // outside the rotated solid, inside its world AABB
        let fall = Vec3::new(0.0, -3.0, 0.0);
        assert!(
            sweep_capsule_obb(&c, fall, center, he, rotation).is_none(),
            "no support outside the rotated face: the shelf is retired"
        );
        // The same descent against the conservative box of this OBB proves the margin existed.
        let conservative = Aabb::from_center_extents(center, Vec3::new(std::f32::consts::SQRT_2, 1.0, std::f32::consts::SQRT_2));
        assert!(
            crate::sweep::sweep_capsule_aabb(&c, fall, &conservative).is_some(),
            "fixture: the old conservative box did offer the shelf here"
        );
    }

    #[test]
    fn obb_sweep_is_deterministic_and_position_independent() {
        let rotation = Quat::from_rotation_y(0.6);
        let d = Vec3::new(5.0, -0.4, 0.3);
        let (center, he) = (Vec3::new(5.0, 1.2, 0.4), Vec3::new(1.0, 2.0, 1.5));
        let here = sweep_capsule_obb(&player(Vec3::ZERO), d, center, he, rotation);
        assert_eq!(
            here,
            sweep_capsule_obb(&player(Vec3::ZERO), d, center, he, rotation),
            "bitwise reproduction"
        );

        // Chunk-local position-independence: offset capsule and box together; the qualitative
        // contact (normal direction, toi) must match to float precision.
        let offset = Vec3::new(128.0, 0.0, -256.0);
        let there = sweep_capsule_obb(&player(offset), d, center + offset, he, rotation);
        let (a, b) = (here.expect("hits"), there.expect("hits"));
        assert!((a.toi - b.toi).abs() < 1e-4, "toi drifted: {} vs {}", a.toi, b.toi);
        assert!((a.normal - b.normal).length() < 1e-4, "normal drifted: {:?} vs {:?}", a.normal, b.normal);
    }
}
