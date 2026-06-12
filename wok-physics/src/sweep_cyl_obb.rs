//! Swept vertical cylinder vs an oriented box.
//!
//! The capsule's OBB sweep maps the capsule into the box frame, because a rotated capsule is
//! still a capsule. A rotated VERTICAL cylinder is not a vertical cylinder, so that trick is
//! gone: this sweep stays in the world frame and runs the solid-pair conservative advancement
//! ([`crate::sweep_cyl::advance_cylinder`], see its module docs) over an alternating projection
//! whose two halves each live in their natural frame - the cylinder projection in world space
//! (where it is exact and closed form) and the box projection through the rotation (the clamp in
//! the box frame, an exact rigid round trip, [`crate::geom::closest_point_on_obb`]). Both
//! projections exact onto convex solids, so the fixed point is the closest pair.
//!
//! Exactness follows the cylinder contract: face and wall contacts settle in a round or two and
//! land within `GAP_EPS`; the tilted-face rim contact (the standable-crate-face case this shape
//! exists for) converges geometrically and is pinned against the analytic support point in the
//! tests. The deep fallback is the local box's face normal at the cylinder centre, rotated back -
//! exactly the capsule OBB sweep's fallback.
//!
//! Determinism (canon contract): fixed iteration caps, one quaternion conjugation per projection,
//! no RNG, no parallelism; relative positions only, so position-independent to float precision.

use glam::{Quat, Vec3};
use wok_scene::Aabb;

use crate::cylinder::Cylinder;
use crate::geom::{alternate_pair, closest_point_on_cylinder, closest_point_on_obb};
use crate::sweep::{SweptHit, face_normal};
use crate::sweep_cyl::advance_cylinder;

/// Sweep `cylinder` through `delta` against a static oriented box; `None` if it never makes
/// contact. `rotation` must be a unit quaternion (classification produces one).
pub fn sweep_cylinder_obb(
    cylinder: &Cylinder,
    delta: Vec3,
    center: Vec3,
    half_extents: Vec3,
    rotation: Quat,
) -> Option<SweptHit> {
    sweep_cylinder_obb_inflated(cylinder, delta, center, half_extents, rotation, 0.0)
}

/// The oriented-box sweep with the surfaces inflated by `skin` (the slide's separation).
pub(crate) fn sweep_cylinder_obb_inflated(
    cylinder: &Cylinder,
    delta: Vec3,
    center: Vec3,
    half_extents: Vec3,
    rotation: Quat,
    skin: f32,
) -> Option<SweptHit> {
    let local_box = Aabb::from_center_extents(Vec3::ZERO, half_extents);
    advance_cylinder(
        cylinder,
        delta,
        skin,
        |c| {
            alternate_pair(
                c.center,
                |p| closest_point_on_cylinder(c.center, c.radius, c.half_height, p),
                |p| closest_point_on_obb(center, half_extents, rotation, p),
            )
        },
        |cyl_center| rotation * face_normal(&local_box, rotation.conjugate() * (cyl_center - center)),
    )
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_4;

    // The standing player cylinder of the cylinder suites: feet at `feet`, 1.5m tall, 0.5m radius.
    fn player(feet: Vec3) -> Cylinder {
        Cylinder::new(feet + Vec3::new(0.0, 0.75, 0.0), 0.5, 0.75)
    }

    #[test]
    fn cylinder_moving_at_a_yawed_face_stops_at_the_analytic_toi() {
        // A tall box yawed 0.5 rad, approached head-on along its +x face normal n (horizontal, so
        // the cylinder's wall is the contact feature). Contact when the centre is
        // half-extent + radius (1.5) from the box centre along n: a 3.5m advance of 5, toi 0.7.
        // The normal comes back as n itself, the rotated face's true normal.
        let rotation = Quat::from_rotation_y(0.5);
        let n = rotation * Vec3::X;
        let center = Vec3::new(0.0, 1.0, 0.0);
        let c = Cylinder::new(center + n * 5.0, 0.5, 0.75);
        let hit = sweep_cylinder_obb(&c, -n * 5.0, center, Vec3::new(1.0, 3.0, 1.0), rotation)
            .expect("should hit the rotated face");
        assert!((hit.toi - 0.7).abs() < 1e-3, "toi = {}", hit.toi);
        assert!((hit.normal - n).length() < 1e-3, "normal = {:?}, expected {n:?}", hit.normal);
        assert!(((hit.point - center).dot(n) - 1.0).abs() < 1e-3, "point off the face: {:?}", hit.point);
    }

    #[test]
    fn the_bottom_rim_lands_on_a_tilted_face_at_the_analytic_support_toi() {
        // The standable-tilted-face geometry, pinned at the sweep level: a slab pitched 30
        // degrees about x, the cylinder descending onto its top face. The first contact is the
        // bottom rim's downhill point - with the documented fillet, the support point of the
        // filleted solid against the face normal n: the eroded core's support
        // (centre - (hh - f) * Y + (r - f) * u, with u the downhill horizontal, -n flattened)
        // plus the fillet ball, so contact comes when the core support is a fillet from the
        // plane. The analytic toi solves (core_support(t) - face_center) . n = f along the fall;
        // the face normal itself comes back exactly (a flat face is exact, fillet or not).
        let tilt = 30.0_f32.to_radians();
        let rotation = Quat::from_rotation_x(tilt);
        let n = rotation * Vec3::Y;
        let center = Vec3::new(0.0, 1.0, 0.0);
        let he = Vec3::new(2.0, 1.0, 2.0);
        let face_center = center + n * he.y;

        let c = player(Vec3::new(0.0, 4.25, 0.0)); // centre (0, 5, 0)
        let fall = Vec3::new(0.0, -4.0, 0.0);
        let f = crate::sweep_cyl::fillet_of(&c);
        let n_xz = Vec3::new(n.x, 0.0, n.z);
        let u = -n_xz.normalize();
        let core_support = c.center - Vec3::new(0.0, c.half_height - f, 0.0) + u * (c.radius - f);
        let expected = ((core_support - face_center).dot(n) - f) / (4.0 * n.y);

        let hit = sweep_cylinder_obb(&c, fall, center, he, rotation).expect("should land on the tilted face");
        assert!((hit.toi - expected).abs() < 2e-3, "toi = {}, analytic {expected}", hit.toi);
        assert!((hit.normal - n).length() < 1e-3, "the face normal comes back: {:?} vs {n:?}", hit.normal);
        // The contact sits on the fillet arc at the rim: r - f(1 - sin(tilt)) out from the axis.
        let radial = Vec3::new(hit.point.x - c.center.x, 0.0, hit.point.z - c.center.z);
        let expected_radial = c.radius - f * (1.0 - n_xz.length());
        assert!(
            (radial.length() - expected_radial).abs() < 5e-3,
            "contact off the filleted rim: radial {} vs {expected_radial}",
            radial.length()
        );
    }

    #[test]
    fn cylinder_moving_at_a_yawed_corner_column_stops_at_the_edge() {
        // A 45-degree yaw points one vertical box edge straight at the cylinder: the edge column
        // sits sqrt(2) before the centre. Contact when the wall is the radius from the edge:
        // x = 5 - sqrt(2) - 0.5, toi of a +5 move. Wall-vs-edge is the vertical-feature pair, so
        // the normal points straight back.
        let rotation = Quat::from_rotation_y(FRAC_PI_4);
        let c = player(Vec3::ZERO);
        let hit = sweep_cylinder_obb(&c, Vec3::new(5.0, 0.0, 0.0), Vec3::new(5.0, 1.0, 0.0), Vec3::ONE, rotation)
            .expect("should hit the edge");
        let expected = (5.0 - std::f32::consts::SQRT_2 - 0.5) / 5.0;
        assert!((hit.toi - expected).abs() < 1e-3, "toi = {}, expected {expected}", hit.toi);
        assert!((hit.normal - Vec3::NEG_X).length() < 1e-3, "edge normal points back: {:?}", hit.normal);
    }

    #[test]
    fn obb_out_of_reach_behind_grazing_or_zero_motion_misses() {
        let rotation = Quat::from_rotation_y(0.5);
        let (center, he) = (Vec3::new(5.0, 1.0, 0.0), Vec3::ONE);
        let c = player(Vec3::ZERO);
        assert!(sweep_cylinder_obb(&c, Vec3::new(1.0, 0.0, 0.0), center, he, rotation).is_none(), "too short");
        assert!(sweep_cylinder_obb(&c, Vec3::new(-5.0, 0.0, 0.0), center, he, rotation).is_none(), "moving away");
        assert!(sweep_cylinder_obb(&c, Vec3::ZERO, center, he, rotation).is_none(), "zero motion");
        let beside = player(Vec3::new(0.0, 0.0, 2.5));
        assert!(
            sweep_cylinder_obb(&beside, Vec3::new(10.0, 0.0, 0.0), center, he, rotation).is_none(),
            "a graze just outside must miss"
        );
    }

    #[test]
    fn a_cylinder_inside_the_obb_resolves_at_toi_zero_with_a_finite_unit_normal() {
        // Deep overlap: the local box's face-normal fallback answers, rotated back to the world.
        let rotation = Quat::from_rotation_y(0.5);
        let c = player(Vec3::new(0.0, 0.25, 0.0));
        let hit = sweep_cylinder_obb(&c, Vec3::new(5.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0), Vec3::splat(2.0), rotation)
            .expect("overlap is a hit");
        assert_eq!(hit.toi, 0.0);
        assert!(hit.normal.is_finite(), "normal must be finite: {:?}", hit.normal);
        assert!((hit.normal.length() - 1.0).abs() < 1e-5, "normal must be unit: {:?}", hit.normal);
    }

    #[test]
    fn the_obb_sweep_is_deterministic_and_position_independent() {
        let rotation = Quat::from_rotation_y(0.6);
        let d = Vec3::new(5.0, -0.4, 0.3);
        let (center, he) = (Vec3::new(5.0, 1.2, 0.4), Vec3::new(1.0, 2.0, 1.5));
        let here = sweep_cylinder_obb(&player(Vec3::ZERO), d, center, he, rotation);
        assert_eq!(
            here,
            sweep_cylinder_obb(&player(Vec3::ZERO), d, center, he, rotation),
            "bitwise reproduction"
        );

        let offset = Vec3::new(128.0, 0.0, -256.0);
        let there = sweep_cylinder_obb(&player(offset), d, center + offset, he, rotation);
        let (a, b) = (here.expect("hits"), there.expect("hits"));
        assert!((a.toi - b.toi).abs() < 1e-4, "toi drifted: {} vs {}", a.toi, b.toi);
        assert!((a.normal - b.normal).length() < 1e-4, "normal drifted: {:?} vs {:?}", a.normal, b.normal);
    }
}
