//! Terrain mesh generation: vertex grid + NW-SE triangulation.
//!
//! Vertex layout: row-major over (i, j) where `i` runs along +X and `j` along +Z. Vertex
//! index `j * width + i`. Position is `(i, height, j)` in chunk-local meters; height comes
//! from `wok_scene::height_at`, normal from `wok_scene::normal_at`, color from the supplied
//! palette via `wok_scene::surface_at`. Out-of-domain samples are unreachable for any
//! integer (i, j) within `[0, width - 1]` (the samplers' domain is `[0, max]` closed-closed
//! per `wok-scene` `sampling.rs`); panicking on `None` therefore signals an invariant
//! violation rather than authoring error.
//!
//! Triangulation: each unit quad with corners `A = (i, j)`, `B = (i+1, j)`,
//! `C = (i, j+1)`, `D = (i+1, j+1)` splits along the A-D diagonal (i.e. NW corner to SE
//! corner). Two triangles, both wound CCW when viewed from +Y so the surface faces up:
//!
//! ```text
//!   A ---- B          A ---- B
//!   |\     |          |   /  |
//!   |  \   |    -->   | /    |  (triangle 1: A, B, D)
//!   |    \ |          A      D
//!   C ---- D                       (triangle 2: A, D, C)
//! ```
//!
//! This matches the cube primitive's +Y face winding so a renderer using a single back-face
//! cull pass treats both surfaces consistently. Plan section 9.18 pins the diagonal
//! direction.

use wok_scene::ChunkRuntime;

use crate::config::SurfaceTagPalette;
use crate::storage::{MeshCpu, MeshVertex};

/// Generate a terrain mesh from a `ChunkRuntime` with `terrain.is_some()`. Panics if the
/// chunk carries no terrain - call sites must check `chunk.terrain.is_some()` first. The
/// `panic` here is preferred over `Option<MeshCpu>` because the caller in the load pipeline
/// already has that test as part of its decision tree.
pub fn generate_mesh(chunk: &ChunkRuntime, palette: &SurfaceTagPalette) -> MeshCpu {
    let terrain = chunk
        .terrain
        .as_ref()
        .expect("terrain::generate_mesh requires chunk.terrain.is_some()");
    let width = terrain.width;
    debug_assert!(width >= 2, "terrain grid must be at least 2x2 (one quad)");
    let vertex_count = (width as usize) * (width as usize);
    let mut vertices: Vec<MeshVertex> = Vec::with_capacity(vertex_count);

    for j in 0..width {
        for i in 0..width {
            let x = i as f32;
            let z = j as f32;
            let h = wok_scene::height_at(chunk, x, z)
                .expect("height_at must resolve at integer grid coords");
            let n = wok_scene::normal_at(chunk, x, z)
                .expect("normal_at must resolve at integer grid coords");
            let color = match wok_scene::surface_at(chunk, x, z) {
                Some(tag) => palette.color(tag),
                None => palette.fallback,
            };
            vertices.push(MeshVertex::new([x, h, z], n.to_array(), color));
        }
    }

    let quads = width - 1;
    let triangle_count = (quads as usize) * (quads as usize) * 2;
    let mut indices: Vec<u32> = Vec::with_capacity(triangle_count * 3);

    let row = width;
    for j in 0..quads {
        for i in 0..quads {
            // A = (i, j); B = (i+1, j); C = (i, j+1); D = (i+1, j+1)
            let a = j * row + i;
            let b = j * row + (i + 1);
            let c = (j + 1) * row + i;
            let d = (j + 1) * row + (i + 1);
            // NW-SE diagonal A-D; CCW with +Y normal: (A, B, D) then (A, D, C).
            indices.extend_from_slice(&[a, b, d, a, d, c]);
        }
    }

    MeshCpu::from_vertices_indices(vertices, indices)
}
