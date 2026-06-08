//! Geometry prep: reducing a transformed primitive to a world-space AABB.
//!
//! wok-scene's primitives ([`Primitive`]) are dimensionless unit shapes; the parent shape's
//! transform supplies their size and placement. To collide them in the AABB-only pass, the caller
//! needs each sliced hitbox (a primitive plus a `Mat4`) as a world-space [`Aabb`]; [`world_aabb`]
//! does that reduction.
//!
//! ## Unit-primitive convention (owned by wok-scene)
//!
//! The convention - the unit half-extent and each primitive's unit bounds - is defined once in
//! wok-scene (`wok_scene::UNIT_HALF_EXTENT` and `Primitive::unit_aabb`; see the wok-scene section of
//! `designs/high-level-design.md`). [`world_aabb`] reads it directly via `Primitive::unit_aabb`, not
//! by restating it, so collision bounds and wok-mesh's drawn meshes cannot drift apart. In short: a
//! unit primitive is inscribed in the cube centred at the origin, so the `Cube` is a literal 1m cube
//! (scale 3 -> a 3m box), `Ellipsoid` / `Cylinder` / `Capsule` are the unit shapes inscribed in that
//! cube (their conservative box is the cube too), and `Plane` is the flat 1m x 1m square at y = 0.
//!
//! Because every volumetric primitive shares the unit cube as its bound, the AABB-only pass treats
//! them identically; the per-primitive shape (a sphere is not its box) only starts to matter at the
//! capsule / ellipsoid step, which is deferred.

use glam::Vec3;
use wok_scene::{Aabb, Mat4, Primitive};

/// Conservative world-space (or chunk-local: the matrix decides the frame) AABB of a transformed
/// primitive. Pass a sliced hitbox's `primitive` and `transform` to get the box the AABB pass uses
/// in its place.
///
/// Computed by transforming the eight corners of the primitive's [`Primitive::unit_aabb`] (wok-scene's
/// canonical unit bounds) and taking their component-wise min and max. This stays correct under
/// rotation and non-uniform scale: the result is the tightest axis-aligned box around the transformed
/// local box, which in turn contains the primitive (it is inscribed in that box). Conservative, never
/// an under-estimate, so the AABB pass cannot tunnel through a reduced hitbox.
pub fn world_aabb(primitive: Primitive, transform: Mat4) -> Aabb {
    let local = primitive.unit_aabb();
    let corners = [
        Vec3::new(local.min.x, local.min.y, local.min.z),
        Vec3::new(local.max.x, local.min.y, local.min.z),
        Vec3::new(local.min.x, local.max.y, local.min.z),
        Vec3::new(local.max.x, local.max.y, local.min.z),
        Vec3::new(local.min.x, local.min.y, local.max.z),
        Vec3::new(local.max.x, local.min.y, local.max.z),
        Vec3::new(local.min.x, local.max.y, local.max.z),
        Vec3::new(local.max.x, local.max.y, local.max.z),
    ];
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for corner in corners {
        let p = transform.transform_point3(corner);
        min = min.min(p);
        max = max.max(p);
    }
    Aabb::new(min, max)
}

/// Centre of an AABB. The corrected "position" the resolve functions hand back is this point of the
/// returned box.
pub fn aabb_center(b: &Aabb) -> Vec3 {
    (b.min + b.max) * 0.5
}

/// Half-extents of an AABB (half its size on each axis).
pub fn aabb_half_extents(b: &Aabb) -> Vec3 {
    (b.max - b.min) * 0.5
}

/// An AABB translated by `by`. Internal: resolution only ever moves a box, never resizes it.
pub(crate) fn aabb_translated(b: &Aabb, by: Vec3) -> Aabb {
    Aabb::new(b.min + by, b.max + by)
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use wok_scene::Transform;

    // ---- world_aabb ----

    #[test]
    fn identity_cube_is_the_unit_cube() {
        let a = world_aabb(Primitive::Cube, Mat4::IDENTITY);
        assert_eq!(a.min, Vec3::splat(-0.5));
        assert_eq!(a.max, Vec3::splat(0.5));
    }

    #[test]
    fn translated_and_scaled_cube_matches_expected_bounds() {
        // Scale 2x (half-extent 0.5 -> 1.0) then translate +10 in x.
        let t = Transform {
            translation: Vec3::new(10.0, 0.0, 0.0),
            rotation: glam::Quat::IDENTITY,
            scale: Vec3::splat(2.0),
        };
        let a = world_aabb(Primitive::Cube, t.to_mat4());
        assert_eq!(a.min, Vec3::new(9.0, -1.0, -1.0));
        assert_eq!(a.max, Vec3::new(11.0, 1.0, 1.0));
    }

    #[test]
    fn yaw_rotated_cube_grows_its_footprint() {
        // A unit cube spun 45 degrees about y has its x/z bound widened to the half-diagonal,
        // 0.5 * sqrt(2); y is untouched. This is the conservative box of the rotated box.
        let t = Transform {
            translation: Vec3::ZERO,
            rotation: glam::Quat::from_rotation_y(std::f32::consts::FRAC_PI_4),
            scale: Vec3::ONE,
        };
        let a = world_aabb(Primitive::Cube, t.to_mat4());
        let half_diag = 0.5 * std::f32::consts::SQRT_2;
        let eps = 1e-6;
        assert!((a.max.x - half_diag).abs() < eps, "max.x = {}", a.max.x);
        assert!((a.min.x + half_diag).abs() < eps, "min.x = {}", a.min.x);
        assert!((a.max.z - half_diag).abs() < eps, "max.z = {}", a.max.z);
        assert!((a.max.y - 0.5).abs() < eps, "max.y = {}", a.max.y);
    }

    #[test]
    fn plane_world_aabb_keeps_zero_thickness_under_translation() {
        let t = Transform {
            translation: Vec3::new(3.0, 7.0, -2.0),
            ..Transform::IDENTITY
        };
        let a = world_aabb(Primitive::Plane, t.to_mat4());
        assert_eq!(a.min, Vec3::new(2.5, 7.0, -2.5));
        assert_eq!(a.max, Vec3::new(3.5, 7.0, -1.5));
    }

    // ---- helpers ----

    #[test]
    fn center_and_half_extents_round_trip() {
        let a = Aabb::from_center_extents(Vec3::new(1.0, 2.0, 3.0), Vec3::new(4.0, 5.0, 6.0));
        assert_eq!(aabb_center(&a), Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(aabb_half_extents(&a), Vec3::new(4.0, 5.0, 6.0));
    }

    #[test]
    fn translated_moves_both_corners() {
        let a = Aabb::new(Vec3::ZERO, Vec3::ONE);
        let b = aabb_translated(&a, Vec3::new(2.0, 0.0, -1.0));
        assert_eq!(b.min, Vec3::new(2.0, 0.0, -1.0));
        assert_eq!(b.max, Vec3::new(3.0, 1.0, 0.0));
    }
}
