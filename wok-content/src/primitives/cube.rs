//! Cube primitive. 24 vertices (4 per face, so per-face normals are crisp), 36 indices
//! (12 triangles). The 8-vertex variant shares normals across faces and produces a
//! lit-football look; cel shading wants flat per-face shading, hence the 24-vertex form.
//!
//! Face order: +X, -X, +Y, -Y, +Z, -Z. Vertex winding is CCW when viewed from outside
//! along the face normal, so the back-face cull pass keeps the outside faces and discards
//! the inside.

use pantry::math::Vec3;

use crate::primitives::PLACEHOLDER_COLOR;
use crate::storage::{MeshCpu, MeshVertex};

pub fn build(half_extents: Vec3) -> MeshCpu {
    let h = half_extents;
    let (hx, hy, hz) = (h.x, h.y, h.z);

    // For each face, four corner positions in CCW order when viewed along +normal, plus the
    // outward normal vector. The macro expands each face into four MeshVertex entries plus
    // a two-triangle index pair (0-1-2 and 0-2-3 against the base index).
    let faces: [([[f32; 3]; 4], [f32; 3]); 6] = [
        // +X face: normal (1, 0, 0). Corners visit y- then z- order (bottom-front, bottom-
        // back, top-back, top-front when looking down +X).
        (
            [
                [hx, -hy, -hz],
                [hx, -hy, hz],
                [hx, hy, hz],
                [hx, hy, -hz],
            ],
            [1.0, 0.0, 0.0],
        ),
        // -X face: normal (-1, 0, 0). CCW when viewed along +normal means reversing the +X
        // winding.
        (
            [
                [-hx, -hy, hz],
                [-hx, -hy, -hz],
                [-hx, hy, -hz],
                [-hx, hy, hz],
            ],
            [-1.0, 0.0, 0.0],
        ),
        // +Y face: normal (0, 1, 0).
        (
            [
                [-hx, hy, -hz],
                [hx, hy, -hz],
                [hx, hy, hz],
                [-hx, hy, hz],
            ],
            [0.0, 1.0, 0.0],
        ),
        // -Y face: normal (0, -1, 0).
        (
            [
                [-hx, -hy, hz],
                [hx, -hy, hz],
                [hx, -hy, -hz],
                [-hx, -hy, -hz],
            ],
            [0.0, -1.0, 0.0],
        ),
        // +Z face: normal (0, 0, 1).
        (
            [
                [hx, -hy, hz],
                [-hx, -hy, hz],
                [-hx, hy, hz],
                [hx, hy, hz],
            ],
            [0.0, 0.0, 1.0],
        ),
        // -Z face: normal (0, 0, -1).
        (
            [
                [-hx, -hy, -hz],
                [hx, -hy, -hz],
                [hx, hy, -hz],
                [-hx, hy, -hz],
            ],
            [0.0, 0.0, -1.0],
        ),
    ];

    let mut vertices = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(36);
    for (corners, normal) in &faces {
        let base = vertices.len() as u32;
        for pos in corners {
            vertices.push(MeshVertex::new(*pos, *normal, PLACEHOLDER_COLOR));
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    MeshCpu::from_vertices_indices(vertices, indices)
}
