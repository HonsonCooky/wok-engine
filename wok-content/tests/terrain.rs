//! Terrain mesh generation tests for plan section 7.2b. Fixtures are hand-built
//! `ChunkRuntime`s with non-default terrain widths so we can exercise the algorithm at
//! 2x2, 4x4, and 129x129 grids without faking the wok-scene authored-data side. The
//! samplers in wok-scene operate on whatever `width` the supplied `RuntimeTerrain`
//! carries.

#![allow(clippy::similar_names)]
#![allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
#![allow(clippy::cast_lossless)]
// Tests compare RGB color arrays by exact equality. The palette inputs are constants the
// generator returns verbatim (no arithmetic on them), so float_cmp is not actually risky.
#![allow(clippy::float_cmp)]

use pantry::math::Vec3;
use wok_scene::{
    ChunkCoord, ChunkEagerness, ChunkRuntime, LightStateRef, RuntimeTerrain, Slug,
};
use wok_content::{
    ContentConfig, MeshCpu, MeshVertex, SurfaceTagPalette, terrain,
};

fn slug(s: &str) -> Slug {
    Slug::new(s).expect("valid slug")
}

/// Build a `ChunkRuntime` carrying a flat (constant-height) terrain of the requested width.
/// All cells share a single surface tag at index 0. Used by tests that need an arbitrary
/// terrain size without authored-data plumbing.
fn flat_terrain_runtime(width: u32, height_quant: u16, vertical_range: f32, tag: &str) -> ChunkRuntime {
    let cells = (width as usize) * (width as usize);
    let heights = vec![height_quant; cells].into_boxed_slice();
    let surface_indices = vec![0u16; cells].into_boxed_slice();
    ChunkRuntime {
        coord: ChunkCoord::new(0, 0),
        eagerness: ChunkEagerness::Eager,
        visible: Vec::new(),
        hitboxes: Vec::new(),
        triggers: Vec::new(),
        regions: Vec::new(),
        light_state: LightStateRef::new(slug("l"), 0),
        surface_tag_table: vec![tag.to_string()],
        terrain: Some(RuntimeTerrain {
            heights,
            surface_indices,
            width,
            vertical_range_meters: vertical_range,
        }),
    }
}

/// Build a `ChunkRuntime` whose terrain has a slope along +X. Heights linearly increase from
/// 0 at x = 0 to `peak_meters` at x = width - 1, quantized through `vertical_range`. Returns
/// the runtime and the slope (meters per cell).
fn sloped_terrain_runtime(width: u32, peak_meters: f32, vertical_range: f32) -> ChunkRuntime {
    let cells = (width as usize) * (width as usize);
    let mut heights = vec![0u16; cells];
    let max_x = (width - 1) as f32;
    for j in 0..width {
        for i in 0..width {
            let t = (i as f32) / max_x;
            let h_meters = t * peak_meters;
            let unit = (h_meters + vertical_range) / (2.0 * vertical_range);
            let q = (unit * u16::MAX as f32).round().clamp(0.0, u16::MAX as f32) as u16;
            heights[(j * width + i) as usize] = q;
        }
    }
    ChunkRuntime {
        coord: ChunkCoord::new(0, 0),
        eagerness: ChunkEagerness::Eager,
        visible: Vec::new(),
        hitboxes: Vec::new(),
        triggers: Vec::new(),
        regions: Vec::new(),
        light_state: LightStateRef::new(slug("l"), 0),
        surface_tag_table: vec!["grass".to_string(), "stone".to_string()],
        terrain: Some(RuntimeTerrain {
            heights: heights.into_boxed_slice(),
            surface_indices: vec![0u16; cells].into_boxed_slice(),
            width,
            vertical_range_meters: vertical_range,
        }),
    }
}

fn default_palette() -> SurfaceTagPalette {
    SurfaceTagPalette::default()
}

// §7.2b #1: triangle and vertex counts at multiple sizes.
#[test]
fn t01_triangle_and_vertex_counts() {
    for &width in &[2u32, 5, 9, 129] {
        let chunk = flat_terrain_runtime(width, 32768, 32.0, "grass");
        let mesh = terrain::generate_mesh(&chunk, &default_palette());
        let expected_verts = (width as usize) * (width as usize);
        let quads = (width - 1) as usize;
        let expected_tris = quads * quads * 2;
        assert_eq!(mesh.vertices.len(), expected_verts, "width={width}");
        assert_eq!(mesh.triangle_count(), expected_tris, "width={width}");
    }
}

// §7.2b #2: determinism. Same RuntimeTerrain + same palette → byte-identical MeshCpu.
#[test]
fn t02_determinism() {
    let chunk_a = sloped_terrain_runtime(9, 5.0, 32.0);
    let chunk_b = sloped_terrain_runtime(9, 5.0, 32.0);
    let palette = default_palette();
    let mesh_a = terrain::generate_mesh(&chunk_a, &palette);
    let mesh_b = terrain::generate_mesh(&chunk_b, &palette);
    assert_eq!(mesh_a, mesh_b, "terrain generator must be deterministic");
}

// §7.2b #3: triangulation diagonal is northwest-to-southeast at every quad.
//
// The vertex layout is row-major: vertex (i, j) lives at index `j * width + i`. Each quad
// at (i, j) has corners A=(i, j), B=(i+1, j), C=(i, j+1), D=(i+1, j+1). NW-SE diagonal
// connects A (i_min, j_min) to D (i_max, j_max). The two triangles are (A, B, D) and
// (A, D, C); both share the A-D edge.
//
// We verify by walking the index buffer in 6-index strides and checking the per-quad shape.
#[test]
fn t03_nw_se_triangulation() {
    let width = 4u32;
    let chunk = flat_terrain_runtime(width, 32768, 32.0, "grass");
    let mesh = terrain::generate_mesh(&chunk, &default_palette());

    let row = width;
    let quads = width - 1;
    let mut tri_idx = 0;
    for j in 0..quads {
        for i in 0..quads {
            let a = j * row + i;
            let b = j * row + (i + 1);
            let c = (j + 1) * row + i;
            let d = (j + 1) * row + (i + 1);
            let expected = [a, b, d, a, d, c];
            let start = tri_idx * 6;
            let slice = &mesh.indices[start..start + 6];
            assert_eq!(
                slice, expected,
                "quad at ({i}, {j}) has wrong triangulation: got {slice:?}, expected {expected:?}"
            );
            tri_idx += 1;
        }
    }
}

// §7.2b #4: vertex normals match wok_scene::normal_at within float tolerance.
#[test]
fn t04_vertex_normals_match_sampler() {
    let width = 9u32;
    let chunk = sloped_terrain_runtime(width, 5.0, 32.0);
    let mesh = terrain::generate_mesh(&chunk, &default_palette());

    let row = width;
    // Sample corners and a few interior vertices.
    let sample_points = [
        (0u32, 0u32),
        (width - 1, 0),
        (0, width - 1),
        (width - 1, width - 1),
        (width / 2, width / 2),
        (2, 4),
        (4, 2),
    ];
    let eps = 1e-4;
    for (i, j) in sample_points {
        let vert_idx = (j * row + i) as usize;
        let v = mesh.vertices[vert_idx];
        let actual = Vec3::from_array(v.normal);
        let expected = wok_scene::normal_at(&chunk, i as f32, j as f32)
            .expect("sampler must resolve at integer coords");
        assert!(
            (actual - expected).length() < eps,
            "normal mismatch at ({i}, {j}): got {actual:?}, expected {expected:?}"
        );
    }
}

// §7.2b #5: vertex colors come from palette[surface_at(x, z)]; unknown tags fall back.
#[test]
fn t05_vertex_colors_via_palette() {
    let width = 5u32;
    let mut chunk = flat_terrain_runtime(width, 32768, 32.0, "stone");
    // Make a single cell carry an "unknown-tag" surface so we can verify fallback. Add the
    // tag string at index 1 (but the palette has no entry for it).
    chunk.surface_tag_table.push("missing-tag-xyz".to_string());
    if let Some(t) = chunk.terrain.as_mut() {
        let mut indices = t.surface_indices.to_vec();
        indices[0] = 1; // cell (0, 0) → "missing-tag-xyz"
        t.surface_indices = indices.into_boxed_slice();
    }
    let palette = default_palette();
    let mesh = terrain::generate_mesh(&chunk, &palette);

    // Vertex (0, 0): surface_at(0, 0) → cell (0, 0) → tag index 1 → "missing-tag-xyz" →
    // palette fallback.
    let v0 = mesh.vertices[0];
    assert_eq!(
        v0.color, palette.fallback,
        "unknown tag must resolve to palette fallback"
    );

    // Vertex at the SE corner: cell (width-1, width-1) → tag index 0 → "stone" → known.
    let stone_color = palette.color("stone");
    let v_se = mesh.vertices[(width * width - 1) as usize];
    assert_eq!(v_se.color, stone_color, "known tag must resolve via palette");
}

// §7.2b #6: bounding AABB has x and z spans across the chunk and y spans the height range.
#[test]
fn t06_bounding_aabb_spans_grid() {
    let width = 9u32;
    let chunk = sloped_terrain_runtime(width, 5.0, 32.0);
    let mesh = terrain::generate_mesh(&chunk, &default_palette());
    let max = (width - 1) as f32;
    let aabb = mesh.bounding_aabb;
    let eps = 1e-3;
    assert!(aabb.min.x.abs() < eps, "min.x near 0: {}", aabb.min.x);
    assert!(aabb.min.z.abs() < eps, "min.z near 0: {}", aabb.min.z);
    assert!((aabb.max.x - max).abs() < eps, "max.x near {max}: {}", aabb.max.x);
    assert!((aabb.max.z - max).abs() < eps, "max.z near {max}: {}", aabb.max.z);
    // Heights run 0..5; AABB y should be a strict superset of that range.
    assert!(aabb.min.y <= 0.05, "min.y near 0: {}", aabb.min.y);
    assert!(aabb.max.y >= 4.95, "max.y near 5: {}", aabb.max.y);
}

// §7.2b #8 (#7 GPU upload covered by the worker pipeline + chunk lifecycle tests once a
// terrain-bearing chunk is added in step 10): terrain == None on a ChunkRuntime produces
// no MeshCpu - the caller (worker pipeline) skips generation. Verify the generator's
// precondition by checking that runtimes with `terrain: Some` produce non-empty output;
// the None branch lives in the pipeline (worker/pipeline.rs).
#[test]
fn t08_none_terrain_short_circuits_via_runtime_field() {
    let mut chunk = flat_terrain_runtime(3, 32768, 32.0, "grass");
    chunk.terrain = None;
    // Calling generate_mesh on a None-terrain runtime panics; the pipeline never calls it
    // in that case. Verify via std::panic::catch_unwind so the test stays observable.
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        terrain::generate_mesh(&chunk, &default_palette());
    }))
    .is_err();
    assert!(panicked, "generate_mesh on None-terrain runtime must panic");
}

// Integration with the storage layer: upload the generated mesh and check the GPU
// handle's index_count matches the CPU buffer length. Covers test #7 "GPU upload via
// LoopbackWorker produces non-zero index_count matching the CPU index count".
#[path = "common/mod.rs"]
mod common;

#[test]
fn t07_gpu_upload_matches_cpu_index_count() {
    use wok_content::storage;

    let width = 5u32;
    let chunk = sloped_terrain_runtime(width, 3.0, 32.0);
    let cpu = terrain::generate_mesh(&chunk, &default_palette());
    let (device, queue) = common::init_gpu();
    let gpu = storage::upload(&device, &queue, &cpu, "terrain-test").expect("upload");
    assert!(gpu.index_count > 0, "non-zero index count");
    assert_eq!(gpu.index_count as usize, cpu.indices.len());
}

fn _unused() {
    let _ = MeshVertex::new([0.0; 3], [0.0; 3], [0.0; 3]);
    let _ = MeshCpu::from_vertices_indices;
    let _ = ContentConfig::default;
}
