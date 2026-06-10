//! Closest-point primitives the swept capsule queries are built from.
//!
//! The sweeps ([`crate::sweep`], [`crate::sweep_round`]) reduce a capsule to its segment and need
//! the distance from that segment to a static shape. That distance is realized by a pair of
//! closest points, one on each shape; the `closest_points_segment_*` routines find them by
//! alternating projection (both shapes convex, so projecting back and forth converges to the
//! closest pair), composing the exact single-shape projections ([`closest_point_on_aabb`],
//! [`closest_point_on_cylinder`], [`closest_point_on_segment`]).
//!
//! Determinism (canon contract): every routine here is a fixed sequence of arithmetic with no RNG
//! and no parallelism, and the iterative ones have a fixed iteration order and cap. All read only
//! relative positions, so they are position-independent to float precision.
//!
//! [`Aabb`]: wok_scene::Aabb

use glam::Vec3;
use wok_scene::Aabb;

/// Below this squared length a segment is treated as a point (the sphere case): we cannot project
/// onto a direction we do not have, so we return the segment's first endpoint.
const DEGENERATE_SEGMENT_SQ: f32 = 1e-12;

/// The iteration stops once both projected points move less than this (squared) between rounds. At
/// ~1e-7 m it is far below any scale the collision response cares about.
const CONVERGED_SQ: f32 = 1e-14;

/// Cap on the alternating-projection rounds. Convergence is geometric and a flat-face or
/// vertical-segment contact lands in one or two rounds; the cap only bounds the pathological,
/// near-parallel-and-distant case, which does not arise for player-scale collision.
const MAX_ITERS: usize = 16;

/// The point of `aabb` closest to `p`: `p` clamped into the box on every axis. Inside the box this
/// returns `p` itself (distance zero).
pub(crate) fn closest_point_on_aabb(aabb: &Aabb, p: Vec3) -> Vec3 {
    p.clamp(aabb.min, aabb.max)
}

/// The point of the segment `a`..`b` closest to `p`: the perpendicular projection of `p` onto the
/// line, clamped to the segment. A zero-length segment returns `a`.
pub(crate) fn closest_point_on_segment(a: Vec3, b: Vec3, p: Vec3) -> Vec3 {
    let ab = b - a;
    let len_sq = ab.length_squared();
    if len_sq <= DEGENERATE_SEGMENT_SQ {
        return a;
    }
    let t = ((p - a).dot(ab) / len_sq).clamp(0.0, 1.0);
    a + ab * t
}

/// The point of the solid vertical cylinder (`center`, `radius`, `half_height`) closest to `p`:
/// `p` clamped axially into the height span and radially onto the disc. Inside the solid this
/// returns `p` itself (distance zero). Exact everywhere, the rim included: the nearest point of a
/// convex solid of revolution shares the query point's azimuth, so clamping the radial and axial
/// coordinates independently is the true projection, not an approximation.
pub(crate) fn closest_point_on_cylinder(center: Vec3, radius: f32, half_height: f32, p: Vec3) -> Vec3 {
    let y = p.y.clamp(center.y - half_height, center.y + half_height);
    let radial = Vec3::new(p.x - center.x, 0.0, p.z - center.z);
    let dist_sq = radial.length_squared();
    if dist_sq <= radius * radius {
        // Radially inside (including on the axis, where the azimuth is undefined but also unneeded).
        return Vec3::new(p.x, y, p.z);
    }
    let scaled = radial * (radius / dist_sq.sqrt());
    Vec3::new(center.x + scaled.x, y, center.z + scaled.z)
}

/// Closest points between the segment `a`..`b` and `aabb`, returned as `(on_segment, on_aabb)`.
///
/// Both shapes are convex, so projecting a point back and forth between them (onto the box, then
/// onto the segment, repeat) is a non-expansive map whose fixed point is the closest pair: each
/// projection can only shrink the gap, and the distance converges to the true minimum (it lands in
/// the overlap, distance zero, when the segment crosses the box). For a vertical segment against an
/// axis-aligned box this settles immediately; the loop exists for oblique segments.
pub(crate) fn closest_points_segment_aabb(a: Vec3, b: Vec3, aabb: &Aabb) -> (Vec3, Vec3) {
    alternate(a, b, |p| closest_point_on_aabb(aabb, p))
}

/// Closest points between the segment `a`..`b` and the solid vertical cylinder, returned as
/// `(on_segment, on_cylinder)`: the same alternating projection as the box, against the exact
/// cylinder projection. The upright player segment against a vertical cylinder settles immediately
/// (the radial direction is constant along both); the loop exists for oblique segments.
pub(crate) fn closest_points_segment_cylinder(
    a: Vec3,
    b: Vec3,
    center: Vec3,
    radius: f32,
    half_height: f32,
) -> (Vec3, Vec3) {
    alternate(a, b, |p| closest_point_on_cylinder(center, radius, half_height, p))
}

/// The alternating-projection loop shared by the segment-vs-shape pairs: project onto the shape,
/// back onto the segment, repeat until both points settle or the cap lands. `project` must be the
/// exact projection onto a convex shape.
fn alternate(a: Vec3, b: Vec3, project: impl Fn(Vec3) -> Vec3) -> (Vec3, Vec3) {
    let mut on_segment = (a + b) * 0.5;
    let mut on_shape = project(on_segment);
    for _ in 0..MAX_ITERS {
        let next_segment = closest_point_on_segment(a, b, on_shape);
        let next_shape = project(next_segment);
        let settled = (next_segment - on_segment).length_squared() < CONVERGED_SQ
            && (next_shape - on_shape).length_squared() < CONVERGED_SQ;
        on_segment = next_segment;
        on_shape = next_shape;
        if settled {
            break;
        }
    }
    (on_segment, on_shape)
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    fn box_at(center: Vec3, half: Vec3) -> Aabb {
        Aabb::from_center_extents(center, half)
    }

    // ---- closest_point_on_aabb ----

    #[test]
    fn point_outside_box_clamps_to_the_face() {
        let b = box_at(Vec3::ZERO, Vec3::splat(1.0)); // [-1, 1]^3
        assert_eq!(closest_point_on_aabb(&b, Vec3::new(5.0, 0.0, 0.0)), Vec3::new(1.0, 0.0, 0.0));
        assert_eq!(closest_point_on_aabb(&b, Vec3::new(-5.0, 0.5, 2.0)), Vec3::new(-1.0, 0.5, 1.0));
    }

    #[test]
    fn point_inside_box_is_returned_unchanged() {
        let b = box_at(Vec3::ZERO, Vec3::splat(1.0));
        let p = Vec3::new(0.25, -0.5, 0.75);
        assert_eq!(closest_point_on_aabb(&b, p), p);
    }

    // ---- closest_point_on_segment ----

    #[test]
    fn projection_lands_inside_the_segment() {
        let a = Vec3::new(0.0, 0.0, 0.0);
        let b = Vec3::new(0.0, 4.0, 0.0);
        // A point off to the side at y = 1 projects to (0, 1, 0).
        assert_eq!(closest_point_on_segment(a, b, Vec3::new(3.0, 1.0, 0.0)), Vec3::new(0.0, 1.0, 0.0));
    }

    #[test]
    fn projection_clamps_past_the_ends() {
        let a = Vec3::new(0.0, 0.0, 0.0);
        let b = Vec3::new(0.0, 4.0, 0.0);
        assert_eq!(closest_point_on_segment(a, b, Vec3::new(0.0, -2.0, 0.0)), a);
        assert_eq!(closest_point_on_segment(a, b, Vec3::new(0.0, 10.0, 0.0)), b);
    }

    #[test]
    fn degenerate_segment_returns_its_point() {
        let a = Vec3::new(1.0, 2.0, 3.0);
        assert_eq!(closest_point_on_segment(a, a, Vec3::new(9.0, 9.0, 9.0)), a);
    }

    // ---- closest_points_segment_aabb ----

    #[test]
    fn vertical_segment_beside_a_box_finds_the_facing_points() {
        // Segment at x = 3 (vertical), box at the origin spanning [-1, 1]^3. Closest box point is on
        // the +x face at x = 1; closest segment point shares that y, z (here y = 0, z = 0).
        let b = box_at(Vec3::ZERO, Vec3::splat(1.0));
        let (ps, pb) = closest_points_segment_aabb(Vec3::new(3.0, -0.5, 0.0), Vec3::new(3.0, 0.5, 0.0), &b);
        // The segment point is on the segment (x = 3), the box point on the +x face (x = 1).
        assert!((ps.x - 3.0).abs() < 1e-5, "ps.x = {}", ps.x);
        assert!((pb.x - 1.0).abs() < 1e-5, "pb.x = {}", pb.x);
        // Gap is the 2.0 along x.
        assert!(((pb - ps).length() - 2.0).abs() < 1e-4, "gap = {}", (pb - ps).length());
    }

    #[test]
    fn segment_crossing_the_box_has_zero_gap() {
        // Segment passes straight through the box: closest points coincide, distance zero.
        let b = box_at(Vec3::ZERO, Vec3::splat(1.0));
        let (ps, pb) = closest_points_segment_aabb(Vec3::new(0.0, -5.0, 0.0), Vec3::new(0.0, 5.0, 0.0), &b);
        assert!((pb - ps).length() < 1e-5, "gap = {}", (pb - ps).length());
    }

    // ---- closest_point_on_cylinder ----

    #[test]
    fn point_beside_the_wall_clamps_radially() {
        // Unit-radius cylinder about the origin, half-height 2; a point out at x = 5, mid-height,
        // projects to the wall at x = 1 with y and azimuth kept.
        let p = closest_point_on_cylinder(Vec3::ZERO, 1.0, 2.0, Vec3::new(5.0, 0.5, 0.0));
        assert!((p - Vec3::new(1.0, 0.5, 0.0)).length() < 1e-6, "got {p:?}");
    }

    #[test]
    fn point_above_the_cap_clamps_to_the_cap_plane() {
        // Radially inside, above the top: straight down onto the cap, xz unchanged.
        let p = closest_point_on_cylinder(Vec3::ZERO, 1.0, 2.0, Vec3::new(0.3, 7.0, -0.2));
        assert!((p - Vec3::new(0.3, 2.0, -0.2)).length() < 1e-6, "got {p:?}");
    }

    #[test]
    fn point_off_the_rim_clamps_to_the_rim_circle_exactly() {
        // Outside both radially and axially: the projection is the rim point at the query's
        // azimuth - exact, not a rounded approximation.
        let p = closest_point_on_cylinder(Vec3::ZERO, 1.0, 2.0, Vec3::new(4.0, 5.0, 0.0));
        assert!((p - Vec3::new(1.0, 2.0, 0.0)).length() < 1e-6, "got {p:?}");
    }

    #[test]
    fn point_inside_the_cylinder_is_returned_unchanged() {
        let p = Vec3::new(0.25, -1.0, 0.5);
        assert_eq!(closest_point_on_cylinder(Vec3::ZERO, 1.0, 2.0, p), p);
    }

    #[test]
    fn point_on_the_axis_above_the_cap_needs_no_azimuth() {
        // The degenerate azimuth case: radially on the axis, the projection is straight down.
        let p = closest_point_on_cylinder(Vec3::ZERO, 1.0, 2.0, Vec3::new(0.0, 9.0, 0.0));
        assert_eq!(p, Vec3::new(0.0, 2.0, 0.0));
    }

    // ---- closest_points_segment_cylinder ----

    #[test]
    fn vertical_segment_beside_a_cylinder_finds_the_facing_points() {
        // Player-shaped: a vertical segment at x = 3 against a unit cylinder at the origin. The
        // closest pair faces along x with a gap of 2.
        let (ps, pc) = closest_points_segment_cylinder(
            Vec3::new(3.0, -0.5, 0.0),
            Vec3::new(3.0, 0.5, 0.0),
            Vec3::ZERO,
            1.0,
            2.0,
        );
        assert!((ps.x - 3.0).abs() < 1e-5, "ps = {ps:?}");
        assert!((pc.x - 1.0).abs() < 1e-5, "pc = {pc:?}");
        assert!(((pc - ps).length() - 2.0).abs() < 1e-4, "gap = {}", (pc - ps).length());
    }

    #[test]
    fn segment_crossing_the_cylinder_has_zero_gap() {
        let (ps, pc) = closest_points_segment_cylinder(
            Vec3::new(0.0, -5.0, 0.0),
            Vec3::new(0.0, 5.0, 0.0),
            Vec3::ZERO,
            1.0,
            2.0,
        );
        assert!((pc - ps).length() < 1e-5, "gap = {}", (pc - ps).length());
    }

    #[test]
    fn oblique_segment_converges_to_the_rim() {
        // A segment running diagonally past the top rim; the pair must be stationary under
        // reprojection (the fixed-point property), the same check the box corner test uses.
        let a = Vec3::new(2.0, 4.0, 0.0);
        let b = Vec3::new(4.0, 2.0, 0.0);
        let (ps, pc) = closest_points_segment_cylinder(a, b, Vec3::ZERO, 1.0, 2.0);
        let reproject_seg = closest_point_on_segment(a, b, pc);
        let reproject_cyl = closest_point_on_cylinder(Vec3::ZERO, 1.0, 2.0, ps);
        assert!((reproject_seg - ps).length() < 1e-3, "segment point not stationary");
        assert!((reproject_cyl - pc).length() < 1e-3, "cylinder point not stationary");
    }

    #[test]
    fn oblique_segment_converges_to_the_corner() {
        // A segment that runs diagonally past the +x +y corner region; the closest box point should
        // be the corner (1, 1, 0) and the segment point the nearest point on the segment to it.
        let b = box_at(Vec3::ZERO, Vec3::splat(1.0));
        let (ps, pb) = closest_points_segment_aabb(Vec3::new(2.0, 3.0, 0.0), Vec3::new(4.0, 1.0, 0.0), &b);
        // pb is clamped onto the box, so it is on the boundary; ps is on the segment. The pair must
        // realize the minimum distance: moving either endpoint's projection cannot shrink it.
        let reproject_seg = closest_point_on_segment(Vec3::new(2.0, 3.0, 0.0), Vec3::new(4.0, 1.0, 0.0), pb);
        let reproject_box = closest_point_on_aabb(&b, ps);
        assert!((reproject_seg - ps).length() < 1e-3, "segment point not stationary");
        assert!((reproject_box - pb).length() < 1e-3, "box point not stationary");
    }
}
