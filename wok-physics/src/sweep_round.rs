//! Swept capsule vs the round static colliders: sphere and vertical cylinder.
//!
//! Two shapes, two methods, one contract (the same [`SweptHit`] semantics as [`crate::sweep`]:
//! true geometric `toi`, outward normal pointing shape-to-capsule, no hit when the motion is not
//! closing, so a flush capsule slides instead of stalling).
//!
//! ## Sphere: exact, closed form
//!
//! Capsule vs sphere is segment vs point with the radii combined: impact is when the distance from
//! the capsule's segment to the sphere's centre falls to `capsule.radius + sphere.radius`. In the
//! capsule's frame the centre is a point moving at `-delta`, and the contact condition is the point
//! entering a capsule of the combined radius around the fixed segment - a ray-vs-capsule
//! intersection with a closed form: a quadratic against the infinite wall (valid where the axial
//! projection lands on the segment) and one against each end sphere, earliest valid entering root
//! wins. No iteration, no tolerance beyond float arithmetic: the `toi` is exact.
//!
//! ## Vertical cylinder: conservative advancement, exact projection
//!
//! The cylinder runs the identical conservative-advancement loop as the box
//! ([`crate::sweep::advance_capsule`]): the cylinder is convex, so the convexity argument in the
//! sweep module's docs carries over verbatim. Its closest-point projection
//! ([`crate::geom::closest_point_on_cylinder`]) is exact everywhere - wall, cap planes, and the rim
//! (the nearest point of a solid of revolution shares the query's azimuth, so the radial and axial
//! clamps compose exactly). The rim is therefore not approximated or rounded; like every contact
//! out of the advancement loop, the reported `toi` sits within the loop's `GAP_EPS` of true
//! contact, the same bound the box sweep has always had.
//!
//! Determinism (canon contract): fixed arithmetic and fixed iteration caps, no RNG, no
//! parallelism; both queries read only relative positions, so they are position-independent to
//! float precision.

use glam::Vec3;

use crate::capsule::Capsule;
use crate::geom::{closest_point_on_segment, closest_points_segment_cylinder};
use crate::sweep::{CLOSING_EPS, GAP_EPS, MOTION_EPS_SQ, SweptHit, TOUCHING, advance_capsule};

/// Below this squared length the capsule's segment is a point and the sphere query reduces to
/// sphere-vs-sphere (only the end-sphere case remains).
const DEGENERATE_SEGMENT_SQ: f32 = 1e-12;

/// Below this squared quadratic leading coefficient the relative motion has no component in the
/// constraint's plane, so that case cannot produce an entering root.
const QUADRATIC_EPS: f32 = 1e-12;

/// Sweep `capsule` through `delta` against a static sphere; `None` if it never makes contact.
pub fn sweep_capsule_sphere(capsule: &Capsule, delta: Vec3, center: Vec3, radius: f32) -> Option<SweptHit> {
    sweep_capsule_sphere_inflated(capsule, delta, center, radius, 0.0)
}

/// The sphere sweep with the capsule radius inflated by `skin` ([`crate::slide`]'s separation).
pub(crate) fn sweep_capsule_sphere_inflated(
    capsule: &Capsule,
    delta: Vec3,
    center: Vec3,
    sphere_radius: f32,
    skin: f32,
) -> Option<SweptHit> {
    if delta.length_squared() <= MOTION_EPS_SQ {
        return None;
    }
    let combined = capsule.radius + sphere_radius + skin;

    // Where the shapes stand now decides the degenerate branches, mirroring the box sweep's order:
    // segment through the centre is a deep overlap; surface contact only counts when closing.
    let on_seg = closest_point_on_segment(capsule.a, capsule.b, center);
    let to_center = center - on_seg;
    let dist = to_center.length();
    if dist <= TOUCHING {
        // The segment passes through the centre: every direction is as good as any, push back
        // against the motion (delta is non-zero here, so the normalize is safe).
        return Some(SweptHit { toi: 0.0, normal: -delta.normalize(), point: center });
    }
    if dist <= combined + GAP_EPS {
        // Touching or overlapping at the surface: a contact only if the motion closes, so a
        // capsule sliding flush past the sphere keeps moving.
        let toward = to_center / dist;
        if delta.dot(toward) <= CLOSING_EPS {
            return None;
        }
        let normal = -toward;
        return Some(SweptHit { toi: 0.0, normal, point: center + normal * sphere_radius });
    }

    // Clearly apart: the exact entry time of the centre (moving at -delta in the capsule's frame)
    // into the combined-radius capsule around the fixed segment.
    let toi = ray_capsule_entry(center, -delta, capsule.a, capsule.b, combined)?;
    if toi > 1.0 {
        return None;
    }
    // Contact features at the impact: in the relative frame the centre sits at `q`; the capsule's
    // nearest segment point there is `combined` away, and the outward normal runs centre-to-segment
    // (shape -> capsule, the slide plane's orientation).
    let q = center - delta * toi;
    let p_seg = closest_point_on_segment(capsule.a, capsule.b, q);
    let normal = (p_seg - q) / combined;
    Some(SweptHit { toi, normal, point: center + normal * sphere_radius })
}

/// Sweep `capsule` through `delta` against a static solid vertical cylinder; `None` if it never
/// makes contact.
pub fn sweep_capsule_cylinder(
    capsule: &Capsule,
    delta: Vec3,
    center: Vec3,
    radius: f32,
    half_height: f32,
) -> Option<SweptHit> {
    sweep_capsule_cylinder_inflated(capsule, delta, center, radius, half_height, 0.0)
}

/// The cylinder sweep with the capsule radius inflated by `skin`: the box's advancement loop over
/// the cylinder's exact projection.
pub(crate) fn sweep_capsule_cylinder_inflated(
    capsule: &Capsule,
    delta: Vec3,
    center: Vec3,
    radius: f32,
    half_height: f32,
    skin: f32,
) -> Option<SweptHit> {
    advance_capsule(
        capsule,
        delta,
        capsule.radius + skin,
        |a, b| closest_points_segment_cylinder(a, b, center, radius, half_height),
        |p| cylinder_deep_normal(center, radius, half_height, p),
    )
}

/// Outward normal for a point on or inside the cylinder (the advancement loop's degenerate
/// segment-inside case): the least-penetration direction, radial first then the caps, a fixed tie
/// order mirroring the box's face-normal fallback. A point on the axis has no radial direction and
/// falls through to a cap. `pub(crate)` so the moving-cylinder sweep ([`crate::sweep_cyl`]) shares
/// the fallback against its static-cylinder pair.
pub(crate) fn cylinder_deep_normal(center: Vec3, radius: f32, half_height: f32, p: Vec3) -> Vec3 {
    let radial = Vec3::new(p.x - center.x, 0.0, p.z - center.z);
    let radial_len = radial.length();
    let mut best_pen = f32::INFINITY;
    let mut best_normal = Vec3::Y;
    if radial_len > TOUCHING {
        best_pen = radius - radial_len;
        best_normal = radial / radial_len;
    }
    let top_pen = (center.y + half_height) - p.y;
    if top_pen < best_pen {
        best_pen = top_pen;
        best_normal = Vec3::Y;
    }
    if p.y - (center.y - half_height) < best_pen {
        best_normal = Vec3::NEG_Y;
    }
    best_normal
}

/// The smallest `t >= 0` at which the ray `origin + dir * t` enters the capsule of `radius` around
/// the segment `a`..`b`, assuming the origin starts outside it. Closed form: the entering root of
/// the infinite-wall quadratic (kept only when the contact's axial projection lands on the
/// segment) and of each end-sphere quadratic; the earliest valid one wins. `None` when the ray
/// never enters.
fn ray_capsule_entry(origin: Vec3, dir: Vec3, a: Vec3, b: Vec3, radius: f32) -> Option<f32> {
    let mut best: Option<f32> = None;
    let mut consider = |t: f32| {
        if t >= 0.0 && best.is_none_or(|other| t < other) {
            best = Some(t);
        }
    };

    let ab = b - a;
    let len_sq = ab.length_squared();
    if len_sq > DEGENERATE_SEGMENT_SQ {
        let len = len_sq.sqrt();
        let axis = ab / len;
        // Components perpendicular to the axis: the wall constraint lives in that plane.
        let m = origin - a;
        let m_perp = m - axis * m.dot(axis);
        let d_perp = dir - axis * dir.dot(axis);
        if let Some(t) = entry_root(d_perp.length_squared(), 2.0 * m_perp.dot(d_perp), m_perp.length_squared() - radius * radius)
        {
            // A wall crossing only counts where the hit's axial projection is on the segment;
            // beyond the ends the surface belongs to the end spheres.
            let s = (m + dir * t).dot(axis);
            if (0.0..=len).contains(&s) {
                consider(t);
            }
        }
    }
    for end in [a, b] {
        let m = origin - end;
        if let Some(t) = entry_root(dir.length_squared(), 2.0 * m.dot(dir), m.length_squared() - radius * radius) {
            consider(t);
        }
    }
    best
}

/// The entering (smaller) root of `a t^2 + b t + c = 0`, or `None` when the quadratic never
/// crosses zero or has no motion in it. Starting outside (`c > 0`), the smaller root is where the
/// gap first closes; a tangent graze (zero discriminant) counts as touching.
fn entry_root(a: f32, b: f32, c: f32) -> Option<f32> {
    if a <= QUADRATIC_EPS {
        return None;
    }
    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 {
        return None;
    }
    Some((-b - disc.sqrt()) / (2.0 * a))
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    // The standing player capsule the box sweep's tests use: feet at `feet`, 2m tall, 0.5m radius.
    fn player(feet: Vec3) -> Capsule {
        Capsule::upright(feet + Vec3::new(0.0, 1.0, 0.0), 2.0, 0.5)
    }

    // ---- sphere ----

    #[test]
    fn capsule_moving_at_a_sphere_stops_at_the_exact_toi() {
        // Sphere of radius 1 at x = 5, centre at segment height: contact when the horizontal gap
        // is the combined 1.5, i.e. the capsule has moved 3.5 of its 5: toi = 0.7, exactly (the
        // closed form has no convergence slack). Normal faces back along -x.
        let c = player(Vec3::ZERO);
        let hit = sweep_capsule_sphere(&c, Vec3::new(5.0, 0.0, 0.0), Vec3::new(5.0, 1.0, 0.0), 1.0)
            .expect("should hit");
        assert!((hit.toi - 0.7).abs() < 1e-6, "toi = {}", hit.toi);
        assert!((hit.normal - Vec3::NEG_X).length() < 1e-5, "normal = {:?}", hit.normal);
        assert!((hit.point - Vec3::new(4.0, 1.0, 0.0)).length() < 1e-4, "point = {:?}", hit.point);
    }

    #[test]
    fn capsule_falling_onto_a_sphere_hits_its_cap_against_the_top() {
        // Sphere of radius 1 at the origin, capsule centred above it falling straight down: the
        // lower cap meets the sphere when the segment's lower end is the combined 1.5 above the
        // centre. Lower end starts at y = 4.5 (feet 4), so a 3m fall's first 3.0... contact at
        // end y = 1.5: a -3 move of -6 is toi 0.5.
        let c = player(Vec3::new(0.0, 4.0, 0.0));
        let hit = sweep_capsule_sphere(&c, Vec3::new(0.0, -6.0, 0.0), Vec3::ZERO, 1.0).expect("should land");
        assert!((hit.toi - 0.5).abs() < 1e-5, "toi = {}", hit.toi);
        assert!((hit.normal - Vec3::Y).length() < 1e-5, "normal = {:?}", hit.normal);
    }

    #[test]
    fn sphere_out_of_reach_or_behind_misses() {
        let c = player(Vec3::ZERO);
        // Too short: contact needs a 3.5 move, only moving 2.
        assert!(sweep_capsule_sphere(&c, Vec3::new(2.0, 0.0, 0.0), Vec3::new(5.0, 1.0, 0.0), 1.0).is_none());
        // Moving away.
        assert!(sweep_capsule_sphere(&c, Vec3::new(-5.0, 0.0, 0.0), Vec3::new(5.0, 1.0, 0.0), 1.0).is_none());
        // Zero motion.
        assert!(sweep_capsule_sphere(&c, Vec3::ZERO, Vec3::new(2.0, 1.0, 0.0), 1.0).is_none());
    }

    #[test]
    fn a_graze_just_outside_the_combined_radius_misses() {
        // Passing with the lateral offset a hair past the combined 1.5: the quadratic never
        // reaches zero, so the sweep reports a clean miss rather than a phantom rub.
        let c = player(Vec3::ZERO);
        let center = Vec3::new(5.0, 1.0, 1.5 + 1e-3);
        assert!(sweep_capsule_sphere(&c, Vec3::new(10.0, 0.0, 0.0), center, 1.0).is_none());
    }

    #[test]
    fn a_flush_sphere_does_not_block_tangential_motion() {
        // Resting exactly at the combined radius and moving tangentially: not closing, no hit -
        // the property the slide relies on to keep moving along whatever it touched last frame.
        let c = player(Vec3::ZERO);
        let center = Vec3::new(1.5, 1.0, 0.0);
        assert!(sweep_capsule_sphere(&c, Vec3::new(0.0, 0.0, 5.0), center, 1.0).is_none());
        // The same flush contact moving inward is an immediate hit.
        let hit = sweep_capsule_sphere(&c, Vec3::new(5.0, 0.0, 0.0), center, 1.0).expect("inward is a hit");
        assert_eq!(hit.toi, 0.0);
        assert!((hit.normal - Vec3::NEG_X).length() < 1e-4, "normal = {:?}", hit.normal);
    }

    #[test]
    fn segment_through_the_sphere_centre_resolves_at_toi_zero() {
        let c = player(Vec3::ZERO);
        let hit = sweep_capsule_sphere(&c, Vec3::new(5.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0), 0.5)
            .expect("deep overlap is a hit");
        assert_eq!(hit.toi, 0.0);
        assert!((hit.normal.length() - 1.0).abs() < 1e-5, "normal must stay unit");
    }

    #[test]
    fn sphere_sweep_is_deterministic_and_position_independent() {
        let d = Vec3::new(5.0, -0.4, 0.3);
        let center = Vec3::new(5.0, 1.2, 0.4);
        let here = sweep_capsule_sphere(&player(Vec3::ZERO), d, center, 1.0);
        assert_eq!(here, sweep_capsule_sphere(&player(Vec3::ZERO), d, center, 1.0), "bitwise reproduction");

        // Chunk-local position-independence: offset capsule and sphere together; the qualitative
        // contact (normal direction, toi) must match to float precision.
        let offset = Vec3::new(128.0, 0.0, -256.0);
        let there = sweep_capsule_sphere(&player(offset), d, center + offset, 1.0);
        let (a, b) = (here.expect("hits"), there.expect("hits"));
        assert!((a.toi - b.toi).abs() < 1e-4, "toi drifted: {} vs {}", a.toi, b.toi);
        assert!((a.normal - b.normal).length() < 1e-4, "normal drifted: {:?} vs {:?}", a.normal, b.normal);
    }

    // ---- vertical cylinder ----

    #[test]
    fn capsule_moving_at_a_cylinder_wall_stops_at_the_expected_toi() {
        // Cylinder radius 1 at x = 5, tall enough that the wall is the contact: combined radius
        // 1.5, so contact at x = 3.5, toi 0.7 of a +5 move - the wall is exact under the
        // advancement loop (constant normal: it converges like a box face).
        let c = player(Vec3::ZERO);
        let hit = sweep_capsule_cylinder(&c, Vec3::new(5.0, 0.0, 0.0), Vec3::new(5.0, 1.0, 0.0), 1.0, 3.0)
            .expect("should hit the wall");
        assert!((hit.toi - 0.7).abs() < 1e-3, "toi = {}", hit.toi);
        assert!((hit.normal - Vec3::NEG_X).length() < 1e-4, "normal = {:?}", hit.normal);
    }

    #[test]
    fn capsule_falling_onto_a_cylinder_cap_lands_with_an_up_normal() {
        // Cylinder top at y = 2 (half-height 1 about y = 1), capsule falling from feet at 4:
        // contact when the feet reach the cap, a -2 move of -4: toi 0.5, normal +Y - the cap
        // plane is exact the same way a box top is.
        let c = player(Vec3::new(0.3, 4.0, 0.0));
        let hit = sweep_capsule_cylinder(&c, Vec3::new(0.0, -4.0, 0.0), Vec3::new(0.0, 1.0, 0.0), 1.0, 1.0)
            .expect("should land on the cap");
        assert!((hit.toi - 0.5).abs() < 1e-3, "toi = {}", hit.toi);
        assert!((hit.normal - Vec3::Y).length() < 1e-4, "normal = {:?}", hit.normal);
    }

    #[test]
    fn cylinder_out_of_reach_or_behind_misses() {
        let c = player(Vec3::ZERO);
        let (center, r, hh) = (Vec3::new(5.0, 1.0, 0.0), 1.0, 3.0);
        assert!(sweep_capsule_cylinder(&c, Vec3::new(2.0, 0.0, 0.0), center, r, hh).is_none(), "too short");
        assert!(sweep_capsule_cylinder(&c, Vec3::new(-5.0, 0.0, 0.0), center, r, hh).is_none(), "moving away");
        assert!(sweep_capsule_cylinder(&c, Vec3::ZERO, center, r, hh).is_none(), "zero motion");
    }

    #[test]
    fn a_graze_just_outside_the_cylinder_misses() {
        // Passing the wall with the lateral gap a hair open: the gap never closes to zero, and the
        // advancement loop reports the pass as a miss.
        let c = player(Vec3::ZERO);
        let center = Vec3::new(5.0, 1.0, 1.5 + 1e-2);
        assert!(sweep_capsule_cylinder(&c, Vec3::new(10.0, 0.0, 0.0), center, 1.0, 3.0).is_none());
    }

    #[test]
    fn the_rim_contact_is_the_exact_rim_circle() {
        // Aim the capsule's lower cap diagonally at the top rim: the reported contact point must
        // lie on the rim circle itself (radius and cap height simultaneously), pinning that the
        // rim is not rounded off or boxed.
        let c = player(Vec3::new(3.0, 3.5, 0.0));
        let (center, r, hh) = (Vec3::new(0.0, 1.0, 0.0), 1.0, 1.0);
        let hit = sweep_capsule_cylinder(&c, Vec3::new(-3.0, -3.0, 0.0), center, r, hh)
            .expect("should catch the rim");
        let radial = Vec3::new(hit.point.x - center.x, 0.0, hit.point.z - center.z);
        assert!((radial.length() - r).abs() < 1e-3, "contact off the rim radius: {:?}", hit.point);
        assert!((hit.point.y - (center.y + hh)).abs() < 1e-3, "contact off the cap height: {:?}", hit.point);
        // The rim normal leans outward and upward: neither a pure wall nor a pure cap answer.
        assert!(hit.normal.x > 0.1 && hit.normal.y > 0.1, "rim normal should lean out and up: {:?}", hit.normal);
    }

    #[test]
    fn segment_inside_the_cylinder_resolves_at_toi_zero_with_a_radial_push() {
        // Deep overlap: the deep normal picks the least-penetration direction - here the segment
        // stands just inside the wall, so the push is radial.
        let c = player(Vec3::new(0.8, 0.0, 0.0));
        let hit = sweep_capsule_cylinder(&c, Vec3::new(5.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0), 1.0, 4.0)
            .expect("overlap is a hit");
        assert_eq!(hit.toi, 0.0);
        assert!((hit.normal - Vec3::X).length() < 1e-4, "expected a radial +x push: {:?}", hit.normal);
    }

    #[test]
    fn cylinder_sweep_is_deterministic_and_position_independent() {
        let d = Vec3::new(5.0, -0.4, 0.3);
        let (center, r, hh) = (Vec3::new(5.0, 1.2, 0.4), 1.0, 2.0);
        let here = sweep_capsule_cylinder(&player(Vec3::ZERO), d, center, r, hh);
        assert_eq!(here, sweep_capsule_cylinder(&player(Vec3::ZERO), d, center, r, hh), "bitwise reproduction");

        let offset = Vec3::new(128.0, 0.0, -256.0);
        let there = sweep_capsule_cylinder(&player(offset), d, center + offset, r, hh);
        let (a, b) = (here.expect("hits"), there.expect("hits"));
        assert!((a.toi - b.toi).abs() < 1e-4, "toi drifted: {} vs {}", a.toi, b.toi);
        assert!((a.normal - b.normal).length() < 1e-4, "normal drifted: {:?} vs {:?}", a.normal, b.normal);
    }
}
