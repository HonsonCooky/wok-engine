//! The unit `Capsule`: a radius-[`UNIT_HALF_EXTENT`] capsule inscribed in the unit cube.
//!
//! ## Why the unit capsule is a sphere
//!
//! A capsule is a cylinder of radius `r` and body height `b` capped by a hemisphere of radius `r` at
//! each end, so its total height is `b + 2r` and its width is `2r`. To match the unit-primitive
//! convention the shape must fill the unit cube: width `2r = 1`, so `r =` [`UNIT_HALF_EXTENT`]`= 0.5`,
//! and total height `b + 2r = 1`, so the body height `b = 1 - 2r = 0`. A zero-height body leaves just
//! the two hemispheres meeting at the equator: a sphere of radius 0.5. This is forced by the
//! convention, not a shortcut, and it is what keeps the mesh's bounds equal to the cube
//! `world_aabb` wok-physics uses for a `Capsule` hitbox (any non-zero body would push past `+/-0.5`
//! in `y`, and a smaller radius would fall short in `x`/`z`, either way disagreeing with collision).
//!
//! So the unit capsule mesh coincides with the unit ellipsoid mesh. They are kept as distinct
//! generators because they are distinct primitives the convention happens to collapse at unit size;
//! a future step that needs a capsule to render with a visible body (a taller placeholder) would
//! introduce a body-height parameter here, and would have to revisit the AABB convention with it.

use wok_scene::UNIT_HALF_EXTENT;

use crate::mesh::MeshCpu;
use crate::surface::uv_sphere;

/// The unit capsule (a radius-[`UNIT_HALF_EXTENT`] sphere; see the module docs for why the body
/// height is zero). `segments` is the longitude division count and `rings` the latitude stacks *per
/// hemisphere*, so the sphere is built with `2 * rings` stacks; both clamp up to a sane minimum. The
/// dispatcher's defaults are [`crate::DEFAULT_SEGMENTS`] and [`crate::DEFAULT_RINGS`]` / 2`.
pub fn capsule(segments: usize, rings: usize) -> MeshCpu {
    uv_sphere(segments, rings.max(1) * 2, UNIT_HALF_EXTENT)
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use crate::mesh::faces_outward;
    use glam::Vec3;

    #[test]
    fn capsule_vertices_lie_on_the_radius_half_surface() {
        let mesh = capsule(24, 8);
        for v in &mesh.vertices {
            assert!((v.position.length() - 0.5).abs() < 1e-6, "off-surface vertex {:?}", v.position);
            assert!(v.normal.dot(v.position) > 0.0, "inward normal {:?}", v.normal);
        }
    }

    #[test]
    fn capsule_triangles_all_wind_outward() {
        assert!(faces_outward(&capsule(24, 8), Vec3::ZERO));
    }

    #[test]
    fn capsule_bounds_are_the_unit_cube() {
        let b = capsule(24, 8).bounds();
        assert!((b.min - Vec3::splat(-0.5)).length() < 1e-6, "min {:?}", b.min);
        assert!((b.max - Vec3::splat(0.5)).length() < 1e-6, "max {:?}", b.max);
    }

    #[test]
    fn capsule_uses_two_stacks_per_hemisphere_ring_count() {
        // rings-per-hemisphere maps to 2*rings latitude stacks: matches an ellipsoid with that many.
        assert_eq!(capsule(24, 8), crate::ellipsoid(24, 16));
    }

    #[test]
    fn capsule_regenerates_bitwise() {
        assert_eq!(capsule(24, 8), capsule(24, 8));
    }
}
