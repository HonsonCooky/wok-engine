//! Capsule primitive. Cylinder body of half-height `half_height` along +Y, capped by two
//! hemispheres of radius `radius` at y = +half_height and y = -half_height. Total tip-to-tip
//! length is `2 * (half_height + radius)`.
//!
//! Tessellation reuses the cylinder's `segments` for the side ring count and the ellipsoid's
//! `subdivisions` for the hemisphere stack count. Hemispheres are built half-by-half rather
//! than reusing `ellipsoid::build`'s full sphere because we need the equator ring vertices
//! to match the cylinder's top/bottom ring vertices exactly.

use pantry::math::Vec3;

use crate::primitives::PLACEHOLDER_COLOR;
use crate::storage::{MeshCpu, MeshVertex};

pub fn build(radius: f32, half_height: f32, segments: u32, subdivisions: u32) -> MeshCpu {
    let seg = segments.max(3);
    let stacks = subdivisions.max(3);
    let pi = std::f32::consts::PI;
    let two_pi = 2.0 * pi;

    let mut vertices: Vec<MeshVertex> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // --- Cylinder side wall (smoothly shaded around the ring) ---
    let side_top_start = vertices.len() as u32;
    for j in 0..=seg {
        let theta = (j as f32) * two_pi / (seg as f32);
        let cx = theta.cos();
        let sz = theta.sin();
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
        indices.extend_from_slice(&[a0, b0, b1, a0, b1, a1]);
    }

    // --- Top hemisphere ---
    // We build phi running from 0 (the equator) up to PI/2 (the pole). At phi = 0 we re-use
    // the cylinder's top ring positions (the equator ring) but with hemisphere normals (radial
    // through the hemisphere's center, which is at y = half_height). The cylinder's side
    // ring at y = half_height has radial-in-XZ normals; the hemisphere's equator ring has
    // the same XZ normal (the y-component is 0 at the equator). So the vertices match
    // exactly, and we re-emit them anyway (separate vertices) to keep the index buffer
    // self-contained.
    let top_equator_start = vertices.len() as u32;
    for j in 0..=seg {
        let theta = (j as f32) * two_pi / (seg as f32);
        let cx = theta.cos();
        let sz = theta.sin();
        vertices.push(MeshVertex::new(
            [cx * radius, half_height, sz * radius],
            Vec3::new(cx, 0.0, sz).normalize_or_zero().to_array(),
            PLACEHOLDER_COLOR,
        ));
    }
    // Intermediate rings: phi in (0, PI/2). We use `stacks` total stacks for the half-sphere,
    // so the band index runs 1..stacks (stack=0 is the equator we just emitted, stack=stacks
    // is the pole). Per ring, vertices include the seam duplicate.
    let top_ring_starts: Vec<u32> = (1..stacks)
        .map(|i| {
            let phi = (i as f32) * (pi / 2.0) / (stacks as f32);
            let y_off = phi.sin();
            let r_xz = phi.cos();
            let start = vertices.len() as u32;
            for j in 0..=seg {
                let theta = (j as f32) * two_pi / (seg as f32);
                let cx = theta.cos();
                let sz = theta.sin();
                let pos = [
                    cx * radius * r_xz,
                    half_height + y_off * radius,
                    sz * radius * r_xz,
                ];
                let n = Vec3::new(cx * r_xz, y_off, sz * r_xz)
                    .normalize_or_zero()
                    .to_array();
                vertices.push(MeshVertex::new(pos, n, PLACEHOLDER_COLOR));
            }
            start
        })
        .collect();
    // Pole
    let top_pole_idx = vertices.len() as u32;
    vertices.push(MeshVertex::new(
        [0.0, half_height + radius, 0.0],
        [0.0, 1.0, 0.0],
        PLACEHOLDER_COLOR,
    ));
    // Stitch: equator -> ring 1 -> ring 2 -> ... -> pole.
    let mut prev_start = top_equator_start;
    for &start in &top_ring_starts {
        for j in 0..seg {
            let a0 = prev_start + j;
            let a1 = prev_start + j + 1;
            let b0 = start + j;
            let b1 = start + j + 1;
            // CCW viewed from outside the top hemisphere.
            indices.extend_from_slice(&[a0, b1, a1, a0, b0, b1]);
        }
        prev_start = start;
    }
    // Pole fan
    for j in 0..seg {
        indices.push(top_pole_idx);
        indices.push(prev_start + j);
        indices.push(prev_start + j + 1);
    }

    // --- Bottom hemisphere ---
    // Mirror of the top: phi runs 0..PI/2 (equator down to the south pole).
    let bot_equator_start = vertices.len() as u32;
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
    let bot_ring_starts: Vec<u32> = (1..stacks)
        .map(|i| {
            let phi = (i as f32) * (pi / 2.0) / (stacks as f32);
            let y_off = phi.sin();
            let r_xz = phi.cos();
            let start = vertices.len() as u32;
            for j in 0..=seg {
                let theta = (j as f32) * two_pi / (seg as f32);
                let cx = theta.cos();
                let sz = theta.sin();
                let pos = [
                    cx * radius * r_xz,
                    -half_height - y_off * radius,
                    sz * radius * r_xz,
                ];
                let n = Vec3::new(cx * r_xz, -y_off, sz * r_xz)
                    .normalize_or_zero()
                    .to_array();
                vertices.push(MeshVertex::new(pos, n, PLACEHOLDER_COLOR));
            }
            start
        })
        .collect();
    let bot_pole_idx = vertices.len() as u32;
    vertices.push(MeshVertex::new(
        [0.0, -half_height - radius, 0.0],
        [0.0, -1.0, 0.0],
        PLACEHOLDER_COLOR,
    ));
    let mut prev_start = bot_equator_start;
    for &start in &bot_ring_starts {
        for j in 0..seg {
            let a0 = prev_start + j;
            let a1 = prev_start + j + 1;
            let b0 = start + j;
            let b1 = start + j + 1;
            // CCW viewed from outside the bottom hemisphere (opposite of top).
            indices.extend_from_slice(&[a0, a1, b1, a0, b1, b0]);
        }
        prev_start = start;
    }
    for j in 0..seg {
        indices.push(bot_pole_idx);
        indices.push(prev_start + j + 1);
        indices.push(prev_start + j);
    }

    MeshCpu::from_vertices_indices(vertices, indices)
}
