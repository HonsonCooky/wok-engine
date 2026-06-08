//! The flat-faced primitives: the unit `Cube` and the unit `Plane`.
//!
//! Both want hard (per-face) normals, not smooth ones, so a corner shared by three cube faces is
//! emitted three times, once per face with that face's outward normal. This is the opposite choice
//! from the round shapes (which share vertices for smooth normals) and the reason the cube has 24
//! vertices rather than 8.

use glam::Vec3;
use wok_scene::UNIT_HALF_EXTENT;

use crate::mesh::{MeshCpu, Vertex};

/// The unit cube: a 1m cube spanning `+/-`[`UNIT_HALF_EXTENT`] on every axis, with hard face
/// normals. 24 vertices (4 per face) and 12 triangles, wound counter-clockwise from outside.
///
/// Each face is built from its outward normal `n` and two in-plane axes `u`, `v` chosen so that
/// `u x v = n`; laying the quad out as `center +/- u +/- v` then makes the triangles `(0,1,2)` and
/// `(0,2,3)` come out counter-clockwise when seen from `+n`, i.e. front-facing from outside.
pub fn cube() -> MeshCpu {
    let h = UNIT_HALF_EXTENT;
    // (normal, u, v) with u x v = normal, one per face.
    let faces = [
        (Vec3::X, Vec3::Y, Vec3::Z),
        (Vec3::NEG_X, Vec3::Z, Vec3::Y),
        (Vec3::Y, Vec3::Z, Vec3::X),
        (Vec3::NEG_Y, Vec3::X, Vec3::Z),
        (Vec3::Z, Vec3::X, Vec3::Y),
        (Vec3::NEG_Z, Vec3::Y, Vec3::X),
    ];

    let mut vertices = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(36);
    for (normal, u, v) in faces {
        let base = vertices.len() as u32;
        let center = normal * h;
        vertices.push(Vertex::new(center - u * h - v * h, normal));
        vertices.push(Vertex::new(center + u * h - v * h, normal));
        vertices.push(Vertex::new(center + u * h + v * h, normal));
        vertices.push(Vertex::new(center - u * h + v * h, normal));
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    MeshCpu::new(vertices, indices)
}

/// The unit plane: the flat `1m x 1m` square in the xz plane at `y = 0`, single-sided with normal
/// `+Y`. 4 vertices, 2 triangles. Backface culling means it is only seen from above, which is the
/// convention's intent (a ground/quad placeholder).
pub fn plane() -> MeshCpu {
    let h = UNIT_HALF_EXTENT;
    let n = Vec3::Y;
    let vertices = vec![
        Vertex::new(Vec3::new(-h, 0.0, -h), n),
        Vertex::new(Vec3::new(-h, 0.0, h), n),
        Vertex::new(Vec3::new(h, 0.0, h), n),
        Vertex::new(Vec3::new(h, 0.0, -h), n),
    ];
    MeshCpu::new(vertices, vec![0, 1, 2, 0, 2, 3])
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use crate::mesh::faces_outward;
    use std::collections::BTreeSet;

    #[test]
    fn cube_has_24_vertices_and_36_indices() {
        let mesh = cube();
        assert_eq!(mesh.vertices.len(), 24);
        assert_eq!(mesh.indices.len(), 36);
        assert_eq!(mesh.triangle_count(), 12);
    }

    #[test]
    fn cube_has_eight_corners_at_plus_minus_half() {
        // The 24 face vertices collapse to exactly 8 distinct corner positions, each +/-0.5.
        let mesh = cube();
        let corners: BTreeSet<(i32, i32, i32)> = mesh
            .vertices
            .iter()
            .map(|v| {
                // Exact: every coordinate is +/-0.5, so *2 lands on +/-1 with no rounding error.
                ((v.position.x * 2.0) as i32, (v.position.y * 2.0) as i32, (v.position.z * 2.0) as i32)
            })
            .collect();
        assert_eq!(corners.len(), 8);
        for &(x, y, z) in &corners {
            assert!(x.abs() == 1 && y.abs() == 1 && z.abs() == 1, "corner not at +/-0.5: {x},{y},{z}");
        }
    }

    #[test]
    fn cube_face_normals_are_axis_aligned_and_outward() {
        let mesh = cube();
        for v in &mesh.vertices {
            let n = v.normal;
            // Axis-aligned unit normal: exactly one component is +/-1, the rest 0.
            let nonzero = [n.x, n.y, n.z].iter().filter(|c| **c != 0.0).count();
            assert_eq!(nonzero, 1, "normal not axis-aligned: {n:?}");
            assert_eq!(n.length(), 1.0, "normal not unit: {n:?}");
            // Outward: the normal agrees in sign with the corner it sits on (cube is origin-centred).
            assert!(n.dot(v.position) > 0.0, "inward face normal {n:?} at {:?}", v.position);
        }
    }

    #[test]
    fn cube_triangles_all_wind_outward() {
        assert!(faces_outward(&cube(), Vec3::ZERO));
    }

    #[test]
    fn cube_bounds_are_the_unit_cube() {
        let b = cube().bounds();
        assert_eq!(b.min, Vec3::splat(-0.5));
        assert_eq!(b.max, Vec3::splat(0.5));
    }

    #[test]
    fn plane_is_a_single_upward_quad_at_y_zero() {
        let mesh = plane();
        assert_eq!(mesh.vertices.len(), 4);
        assert_eq!(mesh.triangle_count(), 2);
        for v in &mesh.vertices {
            assert_eq!(v.position.y, 0.0);
            assert_eq!(v.normal, Vec3::Y);
        }
    }

    #[test]
    fn plane_front_face_points_up() {
        // The single quad must be wound so its geometric normal is +Y (front face up).
        let mesh = plane();
        let t = &mesh.indices[0..3];
        let a = mesh.vertices[t[0] as usize].position;
        let b = mesh.vertices[t[1] as usize].position;
        let c = mesh.vertices[t[2] as usize].position;
        let face = (b - a).cross(c - a).normalize();
        assert!((face - Vec3::Y).length() < 1e-6, "face normal {face:?}");
    }

    #[test]
    fn plane_bounds_are_flat_in_y() {
        let b = plane().bounds();
        assert_eq!(b.min, Vec3::new(-0.5, 0.0, -0.5));
        assert_eq!(b.max, Vec3::new(0.5, 0.0, 0.5));
    }

    #[test]
    fn cube_and_plane_regenerate_bitwise() {
        assert_eq!(cube(), cube());
        assert_eq!(plane(), plane());
    }
}
