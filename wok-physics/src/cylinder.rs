//! The moving flat-bottomed shape: a vertical cylinder, parameterized by centre, radius, and
//! half-height.
//!
//! A [`Cylinder`] is the solid of points within `radius` of the vertical axis through `center` and
//! within `half_height` of `center.y`: a flat-capped can, always upright. It is the player's
//! collision shape from the player-collider brief onward: the capsule's rounded bottom made tilted
//! faces unstandable (the curvature pulled the bearing point away from the axis faster than any
//! tolerance could follow) and rolled bodies off edges; a flat bottom bears on anything under its
//! disc, which is what face-aware support needs.
//!
//! ## Parameterization (why centre-plus-extents, not two endpoints)
//!
//! The capsule stores a segment because its sweeps reduce to segment-vs-shape distance. The
//! cylinder has no such reduction (its Minkowski "core" would be a segment plus a horizontal
//! disc, and shape-plus-disc distance has no closed form against a box), so its sweeps work on
//! the solid directly ([`crate::sweep_cyl`]) and the natural parameters are the solid's own:
//! centre, radius, half-height - the same triple the static [`crate::Collider::VertCylinder`]
//! carries. Always vertical by scope, like the static one: the player never tilts.
//!
//! [`Cylinder::upright`] mirrors `Capsule::upright`'s signature (centre, total height, radius) so
//! a character controller migrating from the capsule reads the same at the call site.

use glam::Vec3;

/// A solid vertical cylinder: `radius` in the horizontal plane, extending `half_height` above and
/// below `center`. Flat caps - the bottom is the disc at `center.y - half_height`.
///
/// Lives in the same (chunk-local) frame as the static geometry it is tested against. A zero
/// `radius` is the bare axis segment and a zero `half_height` the bare disc; both are handled by
/// the queries without a special case (degenerate is a no-op, not an error).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cylinder {
    pub center: Vec3,
    pub radius: f32,
    pub half_height: f32,
}

impl Cylinder {
    /// A cylinder of the given dimensions about `center`.
    pub fn new(center: Vec3, radius: f32, half_height: f32) -> Self {
        Cylinder { center, radius, half_height }
    }

    /// An upright cylinder of total height `height` (cap to cap) and the given `radius`, centred
    /// on `center`: the capsule constructor's signature, for drop-in migration. A negative height
    /// clamps to zero (the bare disc) rather than inverting.
    pub fn upright(center: Vec3, height: f32, radius: f32) -> Self {
        Cylinder { center, radius, half_height: (height * 0.5).max(0.0) }
    }

    /// The lowest point under the axis (the centre of the bottom disc): the "feet" the terrain
    /// rest keeps above the ground, and where that rest samples the heightmap. Unlike the
    /// capsule's base there is no radius drop - the bottom is flat at `half_height` below centre.
    pub fn base(&self) -> Vec3 {
        Vec3::new(self.center.x, self.center.y - self.half_height, self.center.z)
    }

    /// The same cylinder translated by `by` (the centre moves; radius and half-height are
    /// unchanged). The sweeps advance the cylinder with this and never resize it.
    pub fn translated(&self, by: Vec3) -> Self {
        Cylinder { center: self.center + by, radius: self.radius, half_height: self.half_height }
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn new_stores_the_triple() {
        let c = Cylinder::new(Vec3::new(1.0, 2.0, 3.0), 0.45, 0.75);
        assert_eq!(c.center, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(c.radius, 0.45);
        assert_eq!(c.half_height, 0.75);
    }

    #[test]
    fn upright_halves_the_height_and_keeps_the_radius() {
        let c = Cylinder::upright(Vec3::new(2.0, 1.0, -3.0), 1.5, 0.45);
        assert_eq!(c.half_height, 0.75);
        assert_eq!(c.radius, 0.45);
        assert_eq!(c.center, Vec3::new(2.0, 1.0, -3.0));
    }

    #[test]
    fn the_base_is_the_flat_bottom_under_the_axis() {
        let c = Cylinder::upright(Vec3::new(0.0, 1.0, 0.0), 1.5, 0.45);
        assert_eq!(c.base(), Vec3::new(0.0, 0.25, 0.0));
    }

    #[test]
    fn upright_clamps_a_negative_height_to_the_bare_disc() {
        let c = Cylinder::upright(Vec3::new(0.0, 5.0, 0.0), -1.0, 0.45);
        assert_eq!(c.half_height, 0.0);
        assert_eq!(c.base(), c.center);
    }

    #[test]
    fn translated_moves_the_centre_and_changes_nothing_else() {
        let c = Cylinder::new(Vec3::ZERO, 0.45, 0.75);
        let t = c.translated(Vec3::new(1.0, 0.0, -2.0));
        assert_eq!(t.center, Vec3::new(1.0, 0.0, -2.0));
        assert_eq!(t.radius, 0.45);
        assert_eq!(t.half_height, 0.75);
    }
}
