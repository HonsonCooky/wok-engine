//! Plane primitive. One quad lying in the XZ plane at y = 0, oriented with normal +Y.
//! 4 vertices, 2 triangles, 6 indices. Half-extents define the quad's reach along X and Z.

use pantry::math::Vec2;

use crate::primitives::PLACEHOLDER_COLOR;
use crate::storage::{MeshCpu, MeshVertex};

pub fn build(half_extents: Vec2) -> MeshCpu {
    let hx = half_extents.x;
    let hz = half_extents.y; // Vec2.y is the Z extent (the plane lives in XZ)
    let normal = [0.0, 1.0, 0.0];
    let vertices = vec![
        // CCW from above (+Y).
        MeshVertex::new([-hx, 0.0, -hz], normal, PLACEHOLDER_COLOR),
        MeshVertex::new([hx, 0.0, -hz], normal, PLACEHOLDER_COLOR),
        MeshVertex::new([hx, 0.0, hz], normal, PLACEHOLDER_COLOR),
        MeshVertex::new([-hx, 0.0, hz], normal, PLACEHOLDER_COLOR),
    ];
    let indices = vec![0, 1, 2, 0, 2, 3];
    MeshCpu::from_vertices_indices(vertices, indices)
}
