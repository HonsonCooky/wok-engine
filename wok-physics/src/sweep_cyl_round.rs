//! Swept vertical cylinder vs the round static colliders: sphere and vertical cylinder.
//!
//! Both run [`crate::sweep_cyl::advance_cylinder`], the solid-pair conservative advancement, over
//! their own closest pairs (see that module's docs for the method, the skin semantics, and the
//! exactness contract: flat caps and walls exact to `GAP_EPS`, rims conservatively rounded within
//! the projection residual).
//!
//! - **Sphere**: the closest pair is closed form, no projection loop - the cylinder point nearest
//!   the sphere's centre ([`crate::geom::closest_point_on_cylinder`], exact everywhere, the rim
//!   included) and the sphere surface point along that line. Overlap (the centre within its own
//!   radius of the cylinder) collapses the pair to a shared point, which the advancement reads as
//!   the deep case.
//! - **Vertical cylinder**: alternating projection between the two cylinders, both projections
//!   exact. Wall-vs-wall and cap-vs-cap settle in a round (the radial and vertical directions are
//!   constant); a stacked pair with coincident axes settles vertically without needing an azimuth.
//!
//! Determinism (canon contract): fixed arithmetic and iteration caps, no RNG, no parallelism;
//! relative positions only, so position-independent to float precision.

use glam::Vec3;

use crate::cylinder::Cylinder;
use crate::geom::{alternate_pair, closest_point_on_cylinder};
use crate::sweep::{SweptHit, TOUCHING};
use crate::sweep_cyl::advance_cylinder;
use crate::sweep_round::cylinder_deep_normal;

/// Sweep `cylinder` through `delta` against a static sphere; `None` if it never makes contact.
pub fn sweep_cylinder_sphere(cylinder: &Cylinder, delta: Vec3, center: Vec3, radius: f32) -> Option<SweptHit> {
    sweep_cylinder_sphere_inflated(cylinder, delta, center, radius, 0.0)
}

/// The sphere sweep with the surfaces inflated by `skin` (the slide's separation).
pub(crate) fn sweep_cylinder_sphere_inflated(
    cylinder: &Cylinder,
    delta: Vec3,
    center: Vec3,
    radius: f32,
    skin: f32,
) -> Option<SweptHit> {
    advance_cylinder(
        cylinder,
        delta,
        skin,
        |c| {
            // Closed form: the cylinder point nearest the centre, and the sphere point along the
            // connecting line. Overlap collapses to a shared point (the deep case upstream).
            let on_cyl = closest_point_on_cylinder(c.center, c.radius, c.half_height, center);
            let to_center = center - on_cyl;
            let d = to_center.length();
            if d <= radius + TOUCHING {
                return (on_cyl, on_cyl);
            }
            (on_cyl, center - to_center * (radius / d))
        },
        |cyl_center| {
            // Deep fallback: push radially away from the sphere's centre; a centre-through-centre
            // overlap has no direction, so push back against the motion (non-zero upstream).
            let dir = cyl_center - center;
            let len = dir.length();
            if len > TOUCHING { dir / len } else { -delta.normalize() }
        },
    )
}

/// Sweep `cylinder` through `delta` against a static solid vertical cylinder; `None` if it never
/// makes contact.
pub fn sweep_cylinder_vert_cylinder(
    cylinder: &Cylinder,
    delta: Vec3,
    center: Vec3,
    radius: f32,
    half_height: f32,
) -> Option<SweptHit> {
    sweep_cylinder_vert_cylinder_inflated(cylinder, delta, center, radius, half_height, 0.0)
}

/// The cylinder-vs-cylinder sweep with the surfaces inflated by `skin`.
pub(crate) fn sweep_cylinder_vert_cylinder_inflated(
    cylinder: &Cylinder,
    delta: Vec3,
    center: Vec3,
    radius: f32,
    half_height: f32,
    skin: f32,
) -> Option<SweptHit> {
    advance_cylinder(
        cylinder,
        delta,
        skin,
        |c| {
            alternate_pair(
                c.center,
                |p| closest_point_on_cylinder(c.center, c.radius, c.half_height, p),
                |p| closest_point_on_cylinder(center, radius, half_height, p),
            )
        },
        |cyl_center| cylinder_deep_normal(center, radius, half_height, cyl_center),
    )
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    // The standing player cylinder of the cylinder suites: feet at `feet`, 1.5m tall, 0.5m radius.
    fn player(feet: Vec3) -> Cylinder {
        Cylinder::new(feet + Vec3::new(0.0, 0.75, 0.0), 0.5, 0.75)
    }

    // ---- sphere ----

    #[test]
    fn cylinder_moving_at_a_sphere_stops_at_the_analytic_toi() {
        // Sphere of radius 1 at the cylinder's mid-height: the wall meets it when the horizontal
        // centre gap is 1.5, a 3.5m advance of 5: toi 0.7, normal -x.
        let hit = sweep_cylinder_sphere(&player(Vec3::ZERO), Vec3::new(5.0, 0.0, 0.0), Vec3::new(5.0, 0.75, 0.0), 1.0)
            .expect("should hit");
        assert!((hit.toi - 0.7).abs() < 1e-3, "toi = {}", hit.toi);
        assert!((hit.normal - Vec3::NEG_X).length() < 1e-4, "normal = {:?}", hit.normal);
        assert!((hit.point - Vec3::new(4.0, 0.75, 0.0)).length() < 1e-3, "point = {:?}", hit.point);
    }

    #[test]
    fn the_flat_bottom_lands_on_a_sphere_apex_at_the_analytic_toi_axis_centred_or_not() {
        // Sphere of radius 1 at the origin's height 1: apex at y = 2. The flat bottom meets the
        // apex when the base reaches 2 - the same toi whether the axis is over the centre or the
        // apex sits out under the rim (offset 0.3 < 0.5): the disc bears on any point under it.
        // The capsule's curved bottom landed later and tilted off-axis; this pins the difference.
        for offset in [0.0_f32, 0.3] {
            let c = player(Vec3::new(offset, 4.0, 0.0));
            let hit = sweep_cylinder_sphere(&c, Vec3::new(0.0, -4.0, 0.0), Vec3::new(0.0, 1.0, 0.0), 1.0)
                .expect("should land on the apex");
            assert!((hit.toi - 0.5).abs() < 1e-3, "offset {offset}: toi = {}", hit.toi);
            assert!((hit.normal - Vec3::Y).length() < 1e-4, "offset {offset}: normal = {:?}", hit.normal);
            assert!((hit.point - Vec3::new(0.0, 2.0, 0.0)).length() < 1e-3, "offset {offset}: point = {:?}", hit.point);
        }
    }

    #[test]
    fn sphere_out_of_reach_behind_grazing_or_zero_motion_misses() {
        let c = player(Vec3::ZERO);
        let center = Vec3::new(5.0, 0.75, 0.0);
        assert!(sweep_cylinder_sphere(&c, Vec3::new(2.0, 0.0, 0.0), center, 1.0).is_none(), "too short");
        assert!(sweep_cylinder_sphere(&c, Vec3::new(-5.0, 0.0, 0.0), center, 1.0).is_none(), "moving away");
        assert!(sweep_cylinder_sphere(&c, Vec3::ZERO, center, 1.0).is_none(), "zero motion");
        // A graze just outside the combined 1.5: the gap never closes.
        let grazing = Vec3::new(5.0, 0.75, 1.5 + 1e-2);
        assert!(sweep_cylinder_sphere(&c, Vec3::new(10.0, 0.0, 0.0), grazing, 1.0).is_none(), "a graze must miss");
    }

    #[test]
    fn a_sphere_overlapping_the_cylinder_resolves_at_toi_zero_with_a_radial_push() {
        // The sphere's centre sits just inside the wall: deep overlap, pushed radially apart.
        let c = player(Vec3::ZERO);
        let hit = sweep_cylinder_sphere(&c, Vec3::new(-5.0, 0.0, 0.0), Vec3::new(0.8, 0.75, 0.0), 0.5)
            .expect("overlap is a hit");
        assert_eq!(hit.toi, 0.0);
        assert!((hit.normal - Vec3::NEG_X).length() < 1e-4, "expected a -x push: {:?}", hit.normal);
        assert!((hit.normal.length() - 1.0).abs() < 1e-5, "normal must be unit");
    }

    #[test]
    fn the_sphere_sweep_is_deterministic_and_position_independent() {
        let d = Vec3::new(5.0, -0.4, 0.3);
        let center = Vec3::new(5.0, 1.2, 0.4);
        let here = sweep_cylinder_sphere(&player(Vec3::ZERO), d, center, 1.0);
        assert_eq!(here, sweep_cylinder_sphere(&player(Vec3::ZERO), d, center, 1.0), "bitwise reproduction");

        let offset = Vec3::new(128.0, 0.0, -256.0);
        let there = sweep_cylinder_sphere(&player(offset), d, center + offset, 1.0);
        let (a, b) = (here.expect("hits"), there.expect("hits"));
        assert!((a.toi - b.toi).abs() < 1e-4, "toi drifted: {} vs {}", a.toi, b.toi);
        // The normal bound is 1e-3 where the capsule suites use 1e-4: the cylinder's witness
        // vector is a fillet long (5cm), ten times shorter than the capsule's radius, so the
        // same float noise at offset coordinates reads ten times larger in the direction.
        assert!((a.normal - b.normal).length() < 1e-3, "normal drifted: {:?} vs {:?}", a.normal, b.normal);
    }

    // ---- vertical cylinder ----

    #[test]
    fn cylinder_moving_at_a_pillar_wall_stops_at_the_analytic_toi() {
        // Pillar radius 1 at x = 5, tall enough that the walls meet: combined radius 1.5, contact
        // at x = 3.5, toi 0.7 of +5, normal -x.
        let hit = sweep_cylinder_vert_cylinder(
            &player(Vec3::ZERO),
            Vec3::new(5.0, 0.0, 0.0),
            Vec3::new(5.0, 0.75, 0.0),
            1.0,
            3.0,
        )
        .expect("should hit the wall");
        assert!((hit.toi - 0.7).abs() < 1e-3, "toi = {}", hit.toi);
        assert!((hit.normal - Vec3::NEG_X).length() < 1e-4, "normal = {:?}", hit.normal);
    }

    #[test]
    fn the_flat_bottom_lands_on_a_pillar_cap_at_the_analytic_toi_even_rim_on_rim() {
        // Pillar top at y = 2 (half-height 1 about y = 1, radius 1). Falling from base 4, the
        // flat bottom meets the cap when the base reaches 2: toi 0.5 of -4, normal +Y - with the
        // axes aligned, and equally with the axis out at 1.4 where only the rim ring overlaps
        // the cap (footprints overlap in [0.9, 1.0] of the pillar's radius). Flat-on-flat: any
        // overlap bears at the same height.
        for offset in [0.0_f32, 1.4] {
            let c = player(Vec3::new(offset, 4.0, 0.0));
            let hit = sweep_cylinder_vert_cylinder(&c, Vec3::new(0.0, -4.0, 0.0), Vec3::new(0.0, 1.0, 0.0), 1.0, 1.0)
                .expect("should land on the cap");
            assert!((hit.toi - 0.5).abs() < 1e-3, "offset {offset}: toi = {}", hit.toi);
            assert!((hit.normal - Vec3::Y).length() < 1e-4, "offset {offset}: normal = {:?}", hit.normal);
        }
    }

    #[test]
    fn pillar_out_of_reach_behind_grazing_or_zero_motion_misses() {
        let c = player(Vec3::ZERO);
        let (center, r, hh) = (Vec3::new(5.0, 1.0, 0.0), 1.0, 3.0);
        assert!(sweep_cylinder_vert_cylinder(&c, Vec3::new(2.0, 0.0, 0.0), center, r, hh).is_none(), "too short");
        assert!(sweep_cylinder_vert_cylinder(&c, Vec3::new(-5.0, 0.0, 0.0), center, r, hh).is_none(), "moving away");
        assert!(sweep_cylinder_vert_cylinder(&c, Vec3::ZERO, center, r, hh).is_none(), "zero motion");
        let grazing = Vec3::new(5.0, 1.0, 1.5 + 1e-2);
        assert!(
            sweep_cylinder_vert_cylinder(&c, Vec3::new(10.0, 0.0, 0.0), grazing, r, hh).is_none(),
            "a graze must miss"
        );
    }

    #[test]
    fn a_cylinder_inside_the_pillar_resolves_at_toi_zero_with_a_radial_push() {
        let c = player(Vec3::new(0.8, 0.0, 0.0));
        let hit = sweep_cylinder_vert_cylinder(&c, Vec3::new(5.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0), 1.0, 4.0)
            .expect("overlap is a hit");
        assert_eq!(hit.toi, 0.0);
        assert!((hit.normal - Vec3::X).length() < 1e-4, "expected a radial +x push: {:?}", hit.normal);
    }

    #[test]
    fn the_pillar_sweep_is_deterministic_and_position_independent() {
        let d = Vec3::new(5.0, -0.4, 0.3);
        let (center, r, hh) = (Vec3::new(5.0, 1.2, 0.4), 1.0, 2.0);
        let here = sweep_cylinder_vert_cylinder(&player(Vec3::ZERO), d, center, r, hh);
        assert_eq!(
            here,
            sweep_cylinder_vert_cylinder(&player(Vec3::ZERO), d, center, r, hh),
            "bitwise reproduction"
        );

        let offset = Vec3::new(128.0, 0.0, -256.0);
        let there = sweep_cylinder_vert_cylinder(&player(offset), d, center + offset, r, hh);
        let (a, b) = (here.expect("hits"), there.expect("hits"));
        assert!((a.toi - b.toi).abs() < 1e-4, "toi drifted: {} vs {}", a.toi, b.toi);
        // 1e-3 rather than the capsule suites' 1e-4: the fillet-length witness vector (see the
        // sphere test above).
        assert!((a.normal - b.normal).length() < 1e-3, "normal drifted: {:?} vs {:?}", a.normal, b.normal);
    }
}
