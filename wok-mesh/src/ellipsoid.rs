//! The unit `Ellipsoid`: the radius-[`UNIT_HALF_EXTENT`] sphere inscribed in the unit cube.
//!
//! At unit size the shape is a sphere; it only reads as an ellipsoid once a placement's non-uniform
//! scale stretches it, exactly as the convention intends (the mesh is the unit shape, the transform
//! supplies size). Normals are smooth and radial, so cel shading bands cleanly across the surface.

use wok_scene::UNIT_HALF_EXTENT;

use crate::mesh::MeshCpu;
use crate::surface::uv_sphere;

/// The unit ellipsoid (a radius-[`UNIT_HALF_EXTENT`] sphere). `segments` is the longitude division
/// count and `rings` the latitude stack count; both clamp up to a sane minimum (see
/// [`uv_sphere`](crate::surface)). The dispatcher's defaults are [`crate::DEFAULT_SEGMENTS`] and
/// [`crate::DEFAULT_RINGS`].
pub fn ellipsoid(segments: usize, rings: usize) -> MeshCpu {
    uv_sphere(segments, rings, UNIT_HALF_EXTENT)
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use crate::mesh::faces_outward;
    use glam::Vec3;

    #[test]
    fn ellipsoid_vertices_lie_on_the_radius_half_sphere() {
        let mesh = ellipsoid(24, 16);
        for v in &mesh.vertices {
            assert!((v.position.length() - 0.5).abs() < 1e-6, "off-surface vertex {:?}", v.position);
            assert!(v.normal.dot(v.position) > 0.0, "inward normal {:?}", v.normal);
        }
    }

    #[test]
    fn ellipsoid_triangles_all_wind_outward() {
        assert!(faces_outward(&ellipsoid(24, 16), Vec3::ZERO));
    }

    #[test]
    fn ellipsoid_bounds_are_the_unit_cube() {
        let b = ellipsoid(24, 16).bounds();
        assert!((b.min - Vec3::splat(-0.5)).length() < 1e-6, "min {:?}", b.min);
        assert!((b.max - Vec3::splat(0.5)).length() < 1e-6, "max {:?}", b.max);
    }

    #[test]
    fn ellipsoid_regenerates_bitwise() {
        assert_eq!(ellipsoid(24, 16), ellipsoid(24, 16));
    }
}
