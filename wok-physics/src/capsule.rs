//! The moving shape: a capsule, parameterized by a line segment and a radius.
//!
//! A [`Capsule`] is the set of points within `radius` of the segment from `a` to `b`: a cylinder
//! capped by a hemisphere at each end. This is the player's collision shape (HLD: the Phase 4
//! player is a capsule), and it subsumes a sphere as the zero-length case (`a == b`).
//!
//! ## Parameterization (why a segment, not a unit primitive)
//!
//! Part 1's static hitboxes are unit primitives scaled by a transform (see [`crate::bounds`]); the
//! moving player is not. It is spawned game-side with explicit metres, so the capsule stores its
//! `radius` and segment endpoints directly in the same chunk-local frame as the static [`Aabb`]s it
//! is tested against. No transform, no unit-scaling: the numbers are the shape.
//!
//! Two endpoints (rather than a centre-plus-half-height) is the general form the swept query wants:
//! the sweep reduces the capsule to its segment and tests that against the obstacle, so the segment
//! is the primitive the math is written against. [`Capsule::upright`] is the convenience
//! constructor a character controller reaches for, building the vertical segment from a centre,
//! a total height, and the radius.
//!
//! [`Aabb`]: wok_scene::Aabb

use glam::Vec3;

/// A capsule: every point within `radius` of the segment `a`..`b`.
///
/// Endpoints live in the same (chunk-local) frame as the static geometry the capsule is tested
/// against. A zero-length segment (`a == b`) is a sphere; a zero `radius` is the bare segment. Both
/// are handled by the queries without a special case (the brief's "degenerate is a no-op, not an
/// error" rule), so no constructor rejects them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Capsule {
    pub a: Vec3,
    pub b: Vec3,
    pub radius: f32,
}

impl Capsule {
    /// A capsule spanning the segment `a`..`b` with the given `radius`.
    pub fn new(a: Vec3, b: Vec3, radius: f32) -> Self {
        Capsule { a, b, radius }
    }

    /// An upright (y-axis) capsule of total height `height` (tip to tip) and the given `radius`,
    /// centred on `center`.
    ///
    /// The segment runs from `center - (0, h, 0)` to `center + (0, h, 0)` where `h` is the half
    /// segment length `height / 2 - radius`. When `height < 2 * radius` the shape cannot be that
    /// short, so the segment clamps to zero length and the capsule degrades to a sphere of `radius`
    /// at `center` rather than inverting; this is the graceful-degenerate rule, not an error.
    pub fn upright(center: Vec3, height: f32, radius: f32) -> Self {
        let half_segment = (height * 0.5 - radius).max(0.0);
        let offset = Vec3::new(0.0, half_segment, 0.0);
        Capsule { a: center - offset, b: center + offset, radius }
    }

    /// The segment midpoint: the capsule's reference point, and what the resolve functions report
    /// back as the moved position.
    pub fn center(&self) -> Vec3 {
        (self.a + self.b) * 0.5
    }

    /// The lowest point of the capsule (the bottom of the lower hemisphere): the lower endpoint
    /// dropped by `radius` in y. This is the "feet" the terrain rest keeps above the ground, and
    /// its x/z is where that rest samples the heightmap.
    pub fn base(&self) -> Vec3 {
        let lower = if self.a.y <= self.b.y { self.a } else { self.b };
        Vec3::new(lower.x, lower.y - self.radius, lower.z)
    }

    /// The same capsule translated by `by` (both endpoints move; the radius is unchanged). The
    /// resolve functions advance the capsule with this and never resize it.
    pub fn translated(&self, by: Vec3) -> Self {
        Capsule { a: self.a + by, b: self.b + by, radius: self.radius }
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn new_stores_endpoints_and_radius() {
        let c = Capsule::new(Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.0, 3.0, 0.0), 0.5);
        assert_eq!(c.a, Vec3::new(0.0, 1.0, 0.0));
        assert_eq!(c.b, Vec3::new(0.0, 3.0, 0.0));
        assert_eq!(c.radius, 0.5);
    }

    #[test]
    fn upright_builds_a_vertical_segment_about_the_centre() {
        // height 2, radius 0.5: segment half-length 0.5, so endpoints at center +/- 0.5 in y.
        let c = Capsule::upright(Vec3::new(2.0, 1.0, -3.0), 2.0, 0.5);
        assert_eq!(c.a, Vec3::new(2.0, 0.5, -3.0));
        assert_eq!(c.b, Vec3::new(2.0, 1.5, -3.0));
        assert_eq!(c.radius, 0.5);
        assert_eq!(c.center(), Vec3::new(2.0, 1.0, -3.0));
    }

    #[test]
    fn upright_base_is_the_feet() {
        // feet sit radius below the lower endpoint: 0.5 - 0.5 = 0.0.
        let c = Capsule::upright(Vec3::new(0.0, 1.0, 0.0), 2.0, 0.5);
        assert_eq!(c.base(), Vec3::new(0.0, 0.0, 0.0));
    }

    #[test]
    fn upright_degrades_to_a_sphere_when_too_short() {
        // height 0.5 < 2*radius (1.0): segment collapses to zero length at the centre.
        let c = Capsule::upright(Vec3::new(0.0, 5.0, 0.0), 0.5, 0.5);
        assert_eq!(c.a, c.b);
        assert_eq!(c.a, Vec3::new(0.0, 5.0, 0.0));
    }

    #[test]
    fn base_uses_the_lower_endpoint_regardless_of_order() {
        // b is below a: base must still come off b.
        let c = Capsule::new(Vec3::new(0.0, 4.0, 0.0), Vec3::new(0.0, 1.0, 0.0), 0.25);
        assert_eq!(c.base(), Vec3::new(0.0, 0.75, 0.0));
    }

    #[test]
    fn translated_moves_both_endpoints_and_keeps_radius() {
        let c = Capsule::new(Vec3::ZERO, Vec3::new(0.0, 2.0, 0.0), 0.5);
        let t = c.translated(Vec3::new(1.0, 0.0, -2.0));
        assert_eq!(t.a, Vec3::new(1.0, 0.0, -2.0));
        assert_eq!(t.b, Vec3::new(1.0, 2.0, -2.0));
        assert_eq!(t.radius, 0.5);
    }
}
