//! Terrain mesh generation: triangulate a chunk [`Heightmap`] into a [`MeshCpu`].
//!
//! The heightmap is a [`CHUNK_GRID_DIM`] x [`CHUNK_GRID_DIM`] (129 x 129) grid of samples at 1m
//! spacing, so the mesh has one vertex per sample and covers the chunk's local `[0, 128]m` extent in
//! x and z, in the heightmap's own frame (chunk-local metres, corner origin, up is `+Y`). That is
//! the frame wok-physics's terrain queries and wok-scene's sampling all use, so the drawn surface
//! and the collided surface line up without the renderer rebasing anything.
//!
//! Vertex height comes from [`Heightmap::height_at`] at the integer sample coordinate, which returns
//! that cell's quantized height exactly (no interpolation on a grid line), and the vertex normal from
//! [`Heightmap::normal_at`], the heightmap-gradient normal the cel shader wants for smooth shading on
//! faceted ground. Surface tags are deliberately not baked in (crate docs): terrain colouring by
//! surface is the renderer's concern and it samples the heightmap's surface grid directly.
//!
//! Determinism: the two nested loops run in a fixed order over deterministic sampling functions, so
//! the same heightmap yields a bitwise-identical mesh.

use glam::Vec3;
use wok_scene::{CHUNK_GRID_DIM, Heightmap};

use crate::mesh::{MeshCpu, Vertex};

/// Triangulate `terrain` into a mesh in the heightmap's chunk-local frame.
///
/// One vertex per grid sample (129 x 129 = 16641), and two triangles per grid cell (128 x 128 cells,
/// 32768 triangles) so the whole chunk surface is covered. Each cell's quad splits along the
/// `(x, z+1)`-`(x+1, z)` diagonal into triangles wound counter-clockwise seen from above (front face
/// up, `+Y`), matching the crate-wide winding.
pub fn terrain_mesh(terrain: &Heightmap) -> MeshCpu {
    let dim = CHUNK_GRID_DIM;
    let mut vertices = Vec::with_capacity(dim * dim);
    for z in 0..dim {
        for x in 0..dim {
            let (xf, zf) = (x as f32, z as f32);
            let position = Vec3::new(xf, terrain.height_at(xf, zf), zf);
            let normal = terrain.normal_at(xf, zf);
            vertices.push(Vertex::new(position, normal));
        }
    }

    let stride = dim as u32;
    let mut indices = Vec::with_capacity((dim - 1) * (dim - 1) * 6);
    for z in 0..stride - 1 {
        for x in 0..stride - 1 {
            // Cell corners by grid index (z-major, matching the heightmap's own layout).
            let a = z * stride + x; // (x,   z)
            let b = a + 1; // (x+1, z)
            let c = a + stride; // (x,   z+1)
            let d = c + 1; // (x+1, z+1)
            // Two +Y-facing triangles: (a, c, b) and (b, c, d).
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }

    MeshCpu::new(vertices, indices)
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use wok_scene::{CHUNK_GRID_LEN, SurfaceTag};

    // Flat terrain at a single raw height across every sample.
    fn flat(raw: u16) -> Heightmap {
        Heightmap::new(vec![raw; CHUNK_GRID_LEN], vec![SurfaceTag::new("g")], vec![0; CHUNK_GRID_LEN])
            .unwrap()
    }

    // Terrain ramping along +x by `delta` raw units per cell, independent of z (mirrors the wok-scene
    // and wok-physics ramp fixtures, so the height at an integer x is exact).
    fn ramp_x(delta: u16) -> Heightmap {
        let heights = (0..CHUNK_GRID_LEN)
            .map(|i| (i % CHUNK_GRID_DIM) as u16 * delta)
            .collect();
        Heightmap::new(heights, vec![SurfaceTag::new("g")], vec![0; CHUNK_GRID_LEN]).unwrap()
    }

    fn vertex_at(mesh: &MeshCpu, x: usize, z: usize) -> Vertex {
        mesh.vertices[z * CHUNK_GRID_DIM + x]
    }

    #[test]
    fn full_grid_coverage_with_expected_counts() {
        let mesh = terrain_mesh(&flat(0));
        let cells = (CHUNK_GRID_DIM - 1) * (CHUNK_GRID_DIM - 1);
        assert_eq!(mesh.vertices.len(), CHUNK_GRID_DIM * CHUNK_GRID_DIM); // 16641 samples
        assert_eq!(mesh.triangle_count(), cells * 2); // 32768 triangles, two per cell
        assert_eq!(mesh.indices.len(), cells * 6);
    }

    #[test]
    fn every_index_points_at_a_real_vertex() {
        let mesh = terrain_mesh(&ramp_x(50));
        let count = mesh.vertices.len() as u32;
        assert!(mesh.indices.iter().all(|&i| i < count));
    }

    #[test]
    fn vertex_positions_carry_the_grid_coordinate_and_sampled_height() {
        let terrain = ramp_x(100);
        let mesh = terrain_mesh(&terrain);
        // Spot-check interior and far-edge samples: x/z are the grid coordinate in metres, y is the
        // heightmap's own sample there (bitwise, since that is exactly how the vertex was built).
        for (x, z) in [(0, 0), (64, 10), (128, 128), (127, 64)] {
            let v = vertex_at(&mesh, x, z);
            assert_eq!(v.position.x, x as f32);
            assert_eq!(v.position.z, z as f32);
            assert_eq!(v.position.y, terrain.height_at(x as f32, z as f32));
        }
        // And the sampled height really tracks the ramp: cell x=64 is 64*100 raw units up.
        assert_eq!(vertex_at(&mesh, 64, 10).position.y, Heightmap::raw_to_meters(6400));
    }

    #[test]
    fn vertex_normals_match_the_heightmap_gradient() {
        let terrain = ramp_x(100);
        let mesh = terrain_mesh(&terrain);
        for (x, z) in [(1, 1), (64, 64), (100, 30)] {
            let v = vertex_at(&mesh, x, z);
            assert_eq!(v.normal, terrain.normal_at(x as f32, z as f32));
        }
    }

    #[test]
    fn flat_terrain_is_planar_with_upward_normals() {
        let terrain = flat(30000);
        let mesh = terrain_mesh(&terrain);
        let ground = terrain.height_at(0.0, 0.0);
        for v in &mesh.vertices {
            assert_eq!(v.position.y, ground);
            assert_eq!(v.normal, Vec3::Y);
        }
        // Front faces point up: every triangle's geometric normal is +Y on flat ground.
        for t in mesh.indices.chunks_exact(3) {
            let a = mesh.vertices[t[0] as usize].position;
            let b = mesh.vertices[t[1] as usize].position;
            let c = mesh.vertices[t[2] as usize].position;
            let face = (b - a).cross(c - a);
            assert!(face.y > 0.0, "triangle {t:?} does not face up: {face:?}");
        }
    }

    #[test]
    fn terrain_regenerates_bitwise() {
        let terrain = ramp_x(75);
        assert_eq!(terrain_mesh(&terrain), terrain_mesh(&terrain));
    }
}
