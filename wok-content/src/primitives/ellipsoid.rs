//! Ellipsoid (UV-sphere) primitive. `subdivisions` is the latitude band count; the longitude
//! ring count is `2 * subdivisions` (so the sphere has roughly square quads near the
//! equator). Pole rings degenerate to fans.
//!
//! Vertex count: `(subdivisions - 1) * (2 * subdivisions + 1) + 2`. The `+1` on the
//! longitude ring duplicates the seam vertex so the UV-like ring closes without index-
//! sharing across the seam; without this the renderer would interpolate vertex attributes
//! linearly across the seam. The `+2` is the two pole vertices.
//!
//! Test §7.2 #5 references `ellipsoid_subdivisions = 16 → expected vertex count`; the
//! formula above gives 15 * 33 + 2 = 497 vertices for subdivisions = 16.

use pantry::math::Vec3;

use crate::primitives::PLACEHOLDER_COLOR;
use crate::storage::{MeshCpu, MeshVertex};

pub fn build(radii: Vec3, subdivisions: u32) -> MeshCpu {
    // `subdivisions < 3` would collapse the ring geometry to a degenerate disc; clamp to
    // the smallest meaningful tessellation. The default is 16; explicit overrides below 3
    // are programmer error or hostile input, both surfaced by silently snapping rather
    // than panicking (this is mesh generation, not file I/O).
    let stacks = subdivisions.max(3);
    let slices = stacks * 2;

    let mut vertices: Vec<MeshVertex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // Top pole
    let top = MeshVertex::new(
        [0.0, radii.y, 0.0],
        [0.0, 1.0, 0.0],
        PLACEHOLDER_COLOR,
    );
    vertices.push(top);
    let top_idx = 0u32;

    // Latitude bands: i in 1..stacks. The angle phi runs from 0 (top) to PI (bottom);
    // ring index i corresponds to phi = i * PI / stacks. Ring vertex count is slices + 1
    // (seam duplicated). Note: stacks is u32 from config; the cast to f32 is precision-safe
    // for stack counts up to 2^24.
    let pi = std::f32::consts::PI;
    let ring_count = stacks - 1;
    for i in 1..stacks {
        let phi = (i as f32) * pi / (stacks as f32);
        let y = phi.cos();
        let r_xz = phi.sin();
        for j in 0..=slices {
            let theta = (j as f32) * 2.0 * pi / (slices as f32);
            let x_unit = r_xz * theta.cos();
            let z_unit = r_xz * theta.sin();
            // Position is on the ellipsoid: scale unit-sphere position by per-axis radii.
            let pos = [x_unit * radii.x, y * radii.y, z_unit * radii.z];
            // Normal: the gradient of the implicit (x/rx)^2 + (y/ry)^2 + (z/rz)^2 = 1, which
            // is (x/rx^2, y/ry^2, z/rz^2). Normalized.
            let n_unnorm = Vec3::new(
                x_unit / radii.x,
                y / radii.y,
                z_unit / radii.z,
            );
            let n = n_unnorm.normalize_or_zero();
            vertices.push(MeshVertex::new(pos, n.to_array(), PLACEHOLDER_COLOR));
        }
    }

    // Bottom pole
    vertices.push(MeshVertex::new(
        [0.0, -radii.y, 0.0],
        [0.0, -1.0, 0.0],
        PLACEHOLDER_COLOR,
    ));
    let bottom_idx = (vertices.len() - 1) as u32;

    // Top fan: top pole -> ring 0 (the first lat band).
    let ring0_start = 1u32; // top_idx is 0; ring 0 starts at index 1
    let ring_span = slices + 1; // vertices per ring including seam duplicate
    for j in 0..slices {
        indices.push(top_idx);
        indices.push(ring0_start + j + 1);
        indices.push(ring0_start + j);
    }

    // Middle bands: for each consecutive pair of rings, emit a quad as two triangles.
    for ring in 0..(ring_count - 1) {
        let row_a = ring0_start + ring * ring_span;
        let row_b = row_a + ring_span;
        for j in 0..slices {
            let a0 = row_a + j;
            let a1 = row_a + j + 1;
            let b0 = row_b + j;
            let b1 = row_b + j + 1;
            // CCW winding viewed from outside (+normal). The quad spans (a0, a1, b1, b0).
            indices.extend_from_slice(&[a0, a1, b1, a0, b1, b0]);
        }
    }

    // Bottom fan: last ring -> bottom pole.
    let last_ring_start = ring0_start + (ring_count - 1) * ring_span;
    for j in 0..slices {
        indices.push(bottom_idx);
        indices.push(last_ring_start + j);
        indices.push(last_ring_start + j + 1);
    }

    MeshCpu::from_vertices_indices(vertices, indices)
}
