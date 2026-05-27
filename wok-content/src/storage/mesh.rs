//! CPU and GPU mesh storage. `MeshVertex` is the engine's one vertex format for Phase 4: cel
//! shading wants position, normal, and a vertex color (since shipped textures haven't
//! arrived). The 4-byte `_pad` keeps the struct at 40 bytes; the wgpu pipeline binds it as
//! a 40-byte stride.
//!
//! Plan section 3.4: "Lock this in early - adding fields later is expensive."
//!
//! Phase A step 3 introduces `MeshCpu` and `MeshVertex` so primitives can produce them.
//! Step 4 adds `MeshGpu` and the upload helpers.

use pantry::bytemuck;
use pantry::math::Aabb;

/// One vertex of a CPU mesh. `repr(C)` + bytemuck `Pod`/`Zeroable` lets the slice be reused
/// as a wgpu buffer payload without re-packing.
#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq)]
#[allow(clippy::pub_underscore_fields)]
pub struct MeshVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 3],
    /// 16-byte alignment pad so the vertex stride is 40 bytes. Public to allow direct
    /// struct construction in tests and downstream crates; the `_` prefix marks intent
    /// (unused except as filler) while the `pub` keyword keeps the struct
    /// trivially-constructible.
    pub _pad: f32,
}

// SAFETY: `MeshVertex` is `#[repr(C)]` and contains only `f32`/`[f32; N]` fields, all of
// which are Pod. Padding fields are explicit (`_pad: f32`) so there is no implicit
// padding the Pod contract would forbid.
unsafe impl bytemuck::Zeroable for MeshVertex {}
unsafe impl bytemuck::Pod for MeshVertex {}

impl MeshVertex {
    pub const fn new(position: [f32; 3], normal: [f32; 3], color: [f32; 3]) -> Self {
        MeshVertex {
            position,
            normal,
            color,
            _pad: 0.0,
        }
    }
}

/// CPU-side mesh data. The bounding AABB is computed at construction time and stored
/// alongside the vertex/index data so consumers (renderer culling, physics broad-phase) can
/// read it without re-iterating.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshCpu {
    pub vertices: Vec<MeshVertex>,
    pub indices: Vec<u32>,
    pub bounding_aabb: Aabb,
}

impl MeshCpu {
    /// Build a `MeshCpu` from explicit vertices and indices. The bounding AABB is computed
    /// from the vertex positions; empty vertex sets produce a zero-extent AABB at the
    /// origin (the only meaningful answer when no positions exist).
    pub fn from_vertices_indices(vertices: Vec<MeshVertex>, indices: Vec<u32>) -> Self {
        let bounding_aabb = compute_aabb(&vertices);
        MeshCpu {
            vertices,
            indices,
            bounding_aabb,
        }
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }
}

fn compute_aabb(vertices: &[MeshVertex]) -> Aabb {
    if vertices.is_empty() {
        return Aabb::new(
            pantry::math::Vec3::ZERO,
            pantry::math::Vec3::ZERO,
        );
    }
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];
    for v in vertices {
        for i in 0..3 {
            if v.position[i] < min[i] {
                min[i] = v.position[i];
            }
            if v.position[i] > max[i] {
                max[i] = v.position[i];
            }
        }
    }
    Aabb::new(
        pantry::math::Vec3::new(min[0], min[1], min[2]),
        pantry::math::Vec3::new(max[0], max[1], max[2]),
    )
}
