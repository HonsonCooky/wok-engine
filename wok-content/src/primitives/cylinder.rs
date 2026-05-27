//! Cylinder primitive. Axis is +Y. `segments` is the longitude ring count (default 24 from
//! `ContentConfig::cylinder_segments`). Three vertex groups: side wall (smoothly shaded
//! around the ring; seam duplicated), top cap (flat shading, fan from center), bottom cap
//! (flat shading, fan from center).
//!
//! Side wall vertices duplicate the seam (segments + 1 verts per ring) so the normal does
//! not interpolate across the seam. Top and bottom rings on the side wall have separate
//! vertices from the cap rings - the cap vertex normal is +Y/-Y, the side vertex normal is
//! radial, and the discontinuity is preserved by separate vertices.

use pantry::math::Vec3;

use crate::primitives::PLACEHOLDER_COLOR;
use crate::storage::{MeshCpu, MeshVertex};

pub fn build(radius: f32, half_height: f32, segments: u32) -> MeshCpu {
    // segments < 3 collapses the ring; clamp.
    let seg = segments.max(3);
    let ring_span = seg + 1;
    let pi = std::f32::consts::PI;
    let two_pi = 2.0 * pi;

    let mut vertices: Vec<MeshVertex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // --- Side wall ---
    let side_top_start = vertices.len() as u32;
    for j in 0..=seg {
        let theta = (j as f32) * two_pi / (seg as f32);
        let cx = theta.cos();
        let sz = theta.sin();
        // Top ring vertex
        vertices.push(MeshVertex::new(
            [cx * radius, half_height, sz * radius],
            Vec3::new(cx, 0.0, sz).normalize_or_zero().to_array(),
            PLACEHOLDER_COLOR,
        ));
    }
    let side_bottom_start = vertices.len() as u32;
    for j in 0..=seg {
        let theta = (j as f32) * two_pi / (seg as f32);
        let cx = theta.cos();
        let sz = theta.sin();
        vertices.push(MeshVertex::new(
            [cx * radius, -half_height, sz * radius],
            Vec3::new(cx, 0.0, sz).normalize_or_zero().to_array(),
            PLACEHOLDER_COLOR,
        ));
    }
    for j in 0..seg {
        let a0 = side_top_start + j;
        let a1 = side_top_start + j + 1;
        let b0 = side_bottom_start + j;
        let b1 = side_bottom_start + j + 1;
        // CCW viewed from outside the cylinder.
        indices.extend_from_slice(&[a0, b0, b1, a0, b1, a1]);
    }

    // --- Top cap ---
    let top_center = vertices.len() as u32;
    vertices.push(MeshVertex::new(
        [0.0, half_height, 0.0],
        [0.0, 1.0, 0.0],
        PLACEHOLDER_COLOR,
    ));
    let top_ring_start = vertices.len() as u32;
    for j in 0..=seg {
        let theta = (j as f32) * two_pi / (seg as f32);
        let cx = theta.cos();
        let sz = theta.sin();
        vertices.push(MeshVertex::new(
            [cx * radius, half_height, sz * radius],
            [0.0, 1.0, 0.0],
            PLACEHOLDER_COLOR,
        ));
    }
    for j in 0..seg {
        // Fan: center, ring[j+1], ring[j]. CCW viewed from +Y.
        indices.push(top_center);
        indices.push(top_ring_start + j + 1);
        indices.push(top_ring_start + j);
    }

    // --- Bottom cap ---
    let bot_center = vertices.len() as u32;
    vertices.push(MeshVertex::new(
        [0.0, -half_height, 0.0],
        [0.0, -1.0, 0.0],
        PLACEHOLDER_COLOR,
    ));
    let bot_ring_start = vertices.len() as u32;
    for j in 0..=seg {
        let theta = (j as f32) * two_pi / (seg as f32);
        let cx = theta.cos();
        let sz = theta.sin();
        vertices.push(MeshVertex::new(
            [cx * radius, -half_height, sz * radius],
            [0.0, -1.0, 0.0],
            PLACEHOLDER_COLOR,
        ));
    }
    for j in 0..seg {
        // Fan: center, ring[j], ring[j+1]. CCW viewed from -Y (opposite winding to top).
        indices.push(bot_center);
        indices.push(bot_ring_start + j);
        indices.push(bot_ring_start + j + 1);
    }

    let _ = ring_span; // ring_span is documentation-grade; the explicit loops use seg+1 directly
    MeshCpu::from_vertices_indices(vertices, indices)
}
