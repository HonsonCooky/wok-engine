//! The unit `Cylinder`: radius [`UNIT_HALF_EXTENT`], spanning `+/-`[`UNIT_HALF_EXTENT`] in `y`.
//!
//! The curved side wants smooth (radial) normals; the two end caps want flat (`+/-Y`) normals. Where
//! the side meets a cap is a hard edge, so the rim is emitted twice: once on the side ring (radial
//! normal) and once on the cap ring (`+/-Y` normal). The caps are triangle fans to a centre vertex,
//! reusing the same fan helpers the sphere's poles use.

use std::f32::consts::TAU;

use glam::Vec3;
use wok_scene::UNIT_HALF_EXTENT;

use crate::mesh::{MeshCpu, Vertex};
use crate::surface::{bottom_fan, connect_rings, top_fan};

/// Push one `segments`-vertex ring at height `y` and the given `radius`, taking each vertex's normal
/// from its longitude via `normal_of(cos_lon, sin_lon)`. Returns the ring's first-vertex index.
fn ring(
    vertices: &mut Vec<Vertex>,
    segments: usize,
    y: f32,
    radius: f32,
    normal_of: impl Fn(f32, f32) -> Vec3,
) -> u32 {
    let start = vertices.len() as u32;
    for j in 0..segments {
        let lon = TAU * (j as f32 / segments as f32);
        let (sin_lon, cos_lon) = lon.sin_cos();
        let position = Vec3::new(radius * cos_lon, y, radius * sin_lon);
        vertices.push(Vertex::new(position, normal_of(cos_lon, sin_lon)));
    }
    start
}

/// The unit cylinder. `segments` is the number of radial divisions of the circle (clamped up to 3
/// so a degenerate parameter yields a triangular prism rather than nothing). The default the
/// dispatcher uses is [`crate::DEFAULT_SEGMENTS`].
pub fn cylinder(segments: usize) -> MeshCpu {
    let segments = segments.max(3);
    let r = UNIT_HALF_EXTENT;
    let h = UNIT_HALF_EXTENT;
    let seg_u32 = segments as u32;

    let mut vertices = Vec::with_capacity(4 * segments + 2);
    let mut indices = Vec::with_capacity(12 * segments);

    // Curved side: bottom and top rings with radial (horizontal, unit) normals.
    let side_bottom = ring(&mut vertices, segments, -h, r, |c, s| Vec3::new(c, 0.0, s));
    let side_top = ring(&mut vertices, segments, h, r, |c, s| Vec3::new(c, 0.0, s));
    connect_rings(&mut indices, side_bottom, side_top, seg_u32);

    // Top cap: a +Y ring plus its centre, fanned so the disc faces up.
    let top_ring = ring(&mut vertices, segments, h, r, |_, _| Vec3::Y);
    let top_center = vertices.len() as u32;
    vertices.push(Vertex::new(Vec3::new(0.0, h, 0.0), Vec3::Y));
    top_fan(&mut indices, top_ring, top_center, seg_u32);

    // Bottom cap: a -Y ring plus its centre, fanned so the disc faces down.
    let bottom_ring = ring(&mut vertices, segments, -h, r, |_, _| Vec3::NEG_Y);
    let bottom_center = vertices.len() as u32;
    vertices.push(Vertex::new(Vec3::new(0.0, -h, 0.0), Vec3::NEG_Y));
    bottom_fan(&mut indices, bottom_center, bottom_ring, seg_u32);

    MeshCpu::new(vertices, indices)
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use crate::mesh::faces_outward;

    #[test]
    fn cylinder_vertices_stay_within_the_unit_cube_surface() {
        let mesh = cylinder(24);
        for v in &mesh.vertices {
            let p = v.position;
            // y stays on +/-0.5; horizontal distance from the axis is 0 (centres) or 0.5 (rims).
            assert!((p.y.abs() - 0.5).abs() < 1e-6, "y off the caps: {p:?}");
            let horiz = (p.x * p.x + p.z * p.z).sqrt();
            assert!(horiz < 1e-6 || (horiz - 0.5).abs() < 1e-6, "horizontal radius {horiz} at {p:?}");
        }
    }

    #[test]
    fn cylinder_side_normals_are_radial_and_caps_are_axial() {
        let mesh = cylinder(24);
        for v in &mesh.vertices {
            assert!((v.normal.length() - 1.0).abs() < 1e-6, "non-unit normal {:?}", v.normal);
            if v.normal.y.abs() < 1e-6 {
                // Side vertex: normal is the outward horizontal direction of its position.
                let radial = Vec3::new(v.position.x, 0.0, v.position.z).normalize();
                assert!((v.normal - radial).length() < 1e-6, "side normal not radial {:?}", v.normal);
            } else {
                // Cap vertex: normal is exactly +/-Y and agrees with which cap it sits on.
                assert!(v.normal == Vec3::Y || v.normal == Vec3::NEG_Y, "cap normal {:?}", v.normal);
                assert_eq!(v.normal.y.signum(), v.position.y.signum(), "cap normal faces wrong way");
            }
        }
    }

    #[test]
    fn cylinder_triangles_all_wind_outward() {
        assert!(faces_outward(&cylinder(24), Vec3::ZERO));
    }

    #[test]
    fn cylinder_bounds_are_the_unit_cube() {
        let b = cylinder(24).bounds();
        assert!((b.min - Vec3::splat(-0.5)).length() < 1e-6, "min {:?}", b.min);
        assert!((b.max - Vec3::splat(0.5)).length() < 1e-6, "max {:?}", b.max);
    }

    #[test]
    fn cylinder_clamps_degenerate_segments() {
        let mesh = cylinder(0);
        assert!(faces_outward(&mesh, Vec3::ZERO));
        assert!(mesh.triangle_count() > 0);
    }

    #[test]
    fn cylinder_regenerates_bitwise() {
        assert_eq!(cylinder(24), cylinder(24));
    }
}
