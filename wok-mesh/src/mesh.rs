//! The CPU mesh: a vertex list and a triangle index list. Pure data, no GPU.
//!
//! A [`Vertex`] carries the minimum cel-shaded rendering needs: a position and an outward normal.
//! No UV, colour, or per-vertex surface attribute yet (see the crate docs: the renderer's vertex
//! format is not pinned). Indices are `u32`: it is the format the wgpu upload (`crate::gpu`) wants
//! and it does not cap mesh size, so loaded meshes (GLTF, later) never force a widening.
//!
//! Triangles use one consistent winding across the whole crate: counter-clockwise front faces with
//! the normal pointing out of the surface, so downstream backface culling and lighting are correct
//! without per-mesh special-casing.

use glam::Vec3;
use wok_scene::Aabb;

/// One mesh vertex: a position and the unit outward normal at that position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vertex {
    pub position: Vec3,
    pub normal: Vec3,
}

impl Vertex {
    pub fn new(position: Vec3, normal: Vec3) -> Self {
        Vertex { position, normal }
    }
}

/// A CPU-side triangle mesh: parallel vertex and index lists.
///
/// `indices` are taken three at a time, each triple a triangle wound counter-clockwise when viewed
/// from the front (the side its vertices' normals face). Pure data; the fields are public because
/// there is nothing to maintain beyond "every index is a valid vertex slot", which the generators
/// uphold by construction.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeshCpu {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl MeshCpu {
    pub fn new(vertices: Vec<Vertex>, indices: Vec<u32>) -> Self {
        MeshCpu { vertices, indices }
    }

    /// Number of triangles: one per three indices.
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// The axis-aligned box that exactly contains every vertex position (an empty mesh gives the
    /// degenerate box at the origin). This is the value the unit-primitive convention is checked
    /// against: a generated unit primitive's bounds must equal wok-physics's `world_aabb` for the
    /// same shape.
    pub fn bounds(&self) -> Aabb {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for v in &self.vertices {
            min = min.min(v.position);
            max = max.max(v.position);
        }
        if self.vertices.is_empty() {
            Aabb::new(Vec3::ZERO, Vec3::ZERO)
        } else {
            Aabb::new(min, max)
        }
    }
}

/// True when every triangle is wound counter-clockwise as seen from outside a shape centred on
/// `center`: its geometric normal (from the vertex order) points away from `center`. For a convex
/// shape this is exactly "consistent winding and outward-facing", checked in one pass; the
/// generators' tests use it. Lives here so each generator's tests can share it.
#[cfg(test)]
pub(crate) fn faces_outward(mesh: &MeshCpu, center: Vec3) -> bool {
    mesh.indices.chunks_exact(3).all(|t| {
        let a = mesh.vertices[t[0] as usize].position;
        let b = mesh.vertices[t[1] as usize].position;
        let c = mesh.vertices[t[2] as usize].position;
        let face = (b - a).cross(c - a);
        let centroid = (a + b + c) / 3.0;
        face.dot(centroid - center) > 0.0
    })
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn triangle_count_is_indices_over_three() {
        let mesh = MeshCpu::new(vec![], vec![0, 1, 2, 0, 2, 3]);
        assert_eq!(mesh.triangle_count(), 2);
    }

    #[test]
    fn bounds_spans_the_extreme_vertices() {
        let mesh = MeshCpu::new(
            vec![
                Vertex::new(Vec3::new(-1.0, 2.0, 0.0), Vec3::Y),
                Vertex::new(Vec3::new(3.0, -4.0, 5.0), Vec3::Y),
                Vertex::new(Vec3::new(0.0, 0.0, -2.0), Vec3::Y),
            ],
            vec![],
        );
        let b = mesh.bounds();
        assert_eq!(b.min, Vec3::new(-1.0, -4.0, -2.0));
        assert_eq!(b.max, Vec3::new(3.0, 2.0, 5.0));
    }

    #[test]
    fn empty_mesh_bounds_is_the_origin_box() {
        let b = MeshCpu::default().bounds();
        assert_eq!(b.min, Vec3::ZERO);
        assert_eq!(b.max, Vec3::ZERO);
    }

    #[test]
    fn faces_outward_flags_a_flipped_triangle() {
        // One outward triangle on the +x side of the origin (CCW seen from +x), then the same
        // triangle reversed (inward). The first passes, the second fails.
        let v = vec![
            Vertex::new(Vec3::new(1.0, -1.0, -1.0), Vec3::X),
            Vertex::new(Vec3::new(1.0, 1.0, -1.0), Vec3::X),
            Vertex::new(Vec3::new(1.0, 0.0, 1.0), Vec3::X),
        ];
        let outward = MeshCpu::new(v.clone(), vec![0, 1, 2]);
        let inward = MeshCpu::new(v, vec![0, 2, 1]);
        assert!(faces_outward(&outward, Vec3::ZERO));
        assert!(!faces_outward(&inward, Vec3::ZERO));
    }
}
