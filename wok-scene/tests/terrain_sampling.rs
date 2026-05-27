//! Plan §7 sampling tests for `height_at`, `normal_at`, `surface_at`. All fixtures construct
//! a `ChunkRuntime` directly with a populated terrain so the sampler path is exercised in
//! isolation from the slicer.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation
)]

use wok_scene::pantry::math::Vec3;
use wok_scene::{
    ChunkCoord, ChunkEagerness, ChunkRuntime, LightStateRef, RuntimeTerrain, Slug, TerrainData,
    height_at, normal_at, surface_at,
};

const CELLS: u32 = TerrainData::CELLS_PER_AXIS;
const COUNT: usize = TerrainData::CELL_COUNT;
const VR: f32 = 32.0;

fn slug(s: &str) -> Slug {
    Slug::new(s).unwrap()
}

/// Encode a height (meters) into u16 the same way the binary format does. Inverse of the
/// sampler's dequantization. Mirrors `sampling::quantize` so tests don't have to know the
/// quantization step.
fn encode(height_m: f32) -> u16 {
    let t = ((height_m + VR) / (2.0 * VR)).clamp(0.0, 1.0);
    (t * f32::from(u16::MAX)).round() as u16
}

fn fresh_chunk_runtime(terrain: Option<RuntimeTerrain>, tag_table: Vec<String>) -> ChunkRuntime {
    ChunkRuntime {
        coord: ChunkCoord::new(0, 0),
        eagerness: ChunkEagerness::Eager,
        visible: Vec::new(),
        hitboxes: Vec::new(),
        triggers: Vec::new(),
        regions: Vec::new(),
        light_state: LightStateRef::new(slug("l"), 1),
        surface_tag_table: tag_table,
        terrain,
    }
}

fn flat_terrain(height_m: f32) -> RuntimeTerrain {
    RuntimeTerrain {
        heights: vec![encode(height_m); COUNT].into_boxed_slice(),
        surface_indices: vec![0u16; COUNT].into_boxed_slice(),
        width: CELLS,
        vertical_range_meters: VR,
    }
}

fn sloped_terrain_along_x(slope_per_m: f32) -> RuntimeTerrain {
    let mut heights = vec![0u16; COUNT];
    for z in 0..CELLS as usize {
        for x in 0..CELLS as usize {
            let h_m = (x as f32) * slope_per_m;
            heights[z * CELLS as usize + x] = encode(h_m);
        }
    }
    RuntimeTerrain {
        heights: heights.into_boxed_slice(),
        surface_indices: vec![0u16; COUNT].into_boxed_slice(),
        width: CELLS,
        vertical_range_meters: VR,
    }
}

fn flat_chunk(height_m: f32) -> ChunkRuntime {
    fresh_chunk_runtime(Some(flat_terrain(height_m)), vec!["grass".to_string()])
}

// ---- height_at ----

#[test]
fn height_at_returns_authored_at_integer_cells() {
    // Set a small grid of integer cell heights and verify height_at returns them at the
    // exact authored positions. The dequantize step round-trips through the u16 quantization
    // grid, so the assertions tolerate quantization slop only.
    let mut terrain = flat_terrain(0.0);
    let authored = [(0u32, 0u32, 1.5_f32), (5, 7, -3.25), (12, 3, 0.0)];
    let w = terrain.width;
    for (x, z, h) in authored {
        terrain.heights[(z * w + x) as usize] = encode(h);
    }
    let chunk = fresh_chunk_runtime(Some(terrain), vec!["grass".to_string()]);

    let quantum_m = 2.0 * VR / f32::from(u16::MAX);
    for (x, z, h) in authored {
        let sample = height_at(&chunk, x as f32, z as f32).expect("in-domain sample");
        assert!(
            (sample - h).abs() <= quantum_m,
            "height_at({x}, {z}) = {sample}, expected ~{h} (quantum {quantum_m})"
        );
    }
}

#[test]
fn height_at_interpolates_between_cells() {
    // Two adjacent cells at known heights along x. Sample at the midpoint must equal the
    // average. The dequantize tolerance accounts for the u16 quantum.
    let mut terrain = flat_terrain(0.0);
    let w = terrain.width;
    terrain.heights[(3 * w + 4) as usize] = encode(10.0);
    terrain.heights[(3 * w + 5) as usize] = encode(20.0);
    let chunk = fresh_chunk_runtime(Some(terrain), vec!["grass".to_string()]);

    let sample = height_at(&chunk, 4.5, 3.0).expect("midpoint in-domain");
    let quantum_m = 2.0 * VR / f32::from(u16::MAX);
    assert!(
        (sample - 15.0).abs() < 2.0 * quantum_m,
        "expected ~15.0, got {sample}"
    );
}

#[test]
fn height_at_out_of_bounds_returns_none() {
    let chunk = flat_chunk(0.0);
    assert_eq!(height_at(&chunk, -0.1, 5.0), None);
    assert_eq!(height_at(&chunk, 128.1, 5.0), None);
    assert_eq!(height_at(&chunk, 5.0, -0.1), None);
    assert_eq!(height_at(&chunk, 5.0, 128.1), None);
    // Shared-edge convention: 128.0 is in-domain.
    assert!(
        height_at(&chunk, 128.0, 5.0).is_some(),
        "x = 128.0 is in-domain under shared-edge"
    );
    assert!(
        height_at(&chunk, 5.0, 128.0).is_some(),
        "z = 128.0 is in-domain under shared-edge"
    );
    assert!(
        height_at(&chunk, 128.0, 128.0).is_some(),
        "corner (128.0, 128.0) is in-domain"
    );
}

#[test]
fn height_at_nan_returns_none() {
    let chunk = flat_chunk(0.0);
    assert_eq!(height_at(&chunk, f32::NAN, 5.0), None);
    assert_eq!(height_at(&chunk, 5.0, f32::NAN), None);
}

#[test]
fn height_at_no_terrain_returns_none() {
    let chunk = fresh_chunk_runtime(None, vec!["unused".to_string()]);
    assert_eq!(height_at(&chunk, 5.0, 5.0), None);
}

// ---- normal_at ----

#[test]
fn normal_at_flat_terrain_is_up() {
    let chunk = flat_chunk(7.5);
    let n = normal_at(&chunk, 64.0, 64.0).expect("center in-domain");
    let eps = 1e-5_f32;
    assert!((n.x).abs() < eps, "expected ~0 x, got {}", n.x);
    assert!((n.y - 1.0).abs() < eps, "expected ~1 y, got {}", n.y);
    assert!((n.z).abs() < eps, "expected ~0 z, got {}", n.z);
}

#[test]
fn normal_at_sloped_terrain_tilts() {
    // Heights rise linearly along x: h(x) = slope * x. dh/dx = slope, dh/dz = 0. Normal is
    // normalize((-slope, 1.0, 0.0)). The slope is kept small enough that h(128) stays inside
    // [-VR, +VR] (the quantization clamps anything outside, which would corrupt the gradient
    // near the clipped region). 0.1 m/m yields h(128) = 12.8 m, well within VR = 32.
    let slope = 0.1_f32;
    let chunk = fresh_chunk_runtime(
        Some(sloped_terrain_along_x(slope)),
        vec!["grass".to_string()],
    );
    let n = normal_at(&chunk, 64.0, 64.0).expect("center in-domain");

    let expected = Vec3::new(-slope, 1.0, 0.0).normalize();
    let tol = 1e-3_f32;
    assert!(
        (n - expected).length() < tol,
        "expected {expected:?}, got {n:?}"
    );
    assert!(
        ((n.length() - 1.0).abs()) < 1e-6,
        "normal must be unit length"
    );
}

#[test]
fn normal_at_out_of_bounds_returns_none() {
    let chunk = flat_chunk(0.0);
    assert_eq!(normal_at(&chunk, -0.1, 5.0), None);
    assert_eq!(normal_at(&chunk, 128.1, 5.0), None);
}

#[test]
fn normal_at_boundary_uses_one_sided_difference() {
    // At x = 0 there is no cell at x = -1, so the implementation falls back to one-sided
    // forward difference. The returned normal must still be valid (unit length, no NaNs).
    let chunk = fresh_chunk_runtime(
        Some(sloped_terrain_along_x(0.2)),
        vec!["grass".to_string()],
    );
    let n = normal_at(&chunk, 0.0, 0.0).expect("corner in-domain");
    assert!(n.x.is_finite() && n.y.is_finite() && n.z.is_finite());
    assert!((n.length() - 1.0).abs() < 1e-5);
    assert!(n.y > 0.0, "y component should still be positive");
}

#[test]
fn normal_at_no_terrain_returns_none() {
    let chunk = fresh_chunk_runtime(None, vec!["unused".to_string()]);
    assert_eq!(normal_at(&chunk, 5.0, 5.0), None);
}

// ---- surface_at ----

#[test]
fn surface_at_returns_borrowed_str() {
    let mut terrain = flat_terrain(0.0);
    let w = terrain.width;
    terrain.surface_indices[(3 * w + 4) as usize] = 1;
    let chunk = fresh_chunk_runtime(
        Some(terrain),
        vec!["grass".to_string(), "stone".to_string()],
    );
    assert_eq!(surface_at(&chunk, 4.0, 3.0), Some("stone"));
    assert_eq!(surface_at(&chunk, 0.0, 0.0), Some("grass"));
    // Lifetime spot-check: the returned &str is tied to chunk's surface_tag_table.
    let tag = surface_at(&chunk, 4.0, 3.0).unwrap();
    assert_eq!(tag.as_ptr(), chunk.surface_tag_table[1].as_ptr());
}

#[test]
fn surface_at_uses_floor_for_cell_index() {
    // A sample at (4.7, 3.2) falls inside cell (4, 3). The tag at that cell is what surface_at
    // returns; the fractional part does not interpolate (surface tags are discrete).
    let mut terrain = flat_terrain(0.0);
    let w = terrain.width;
    terrain.surface_indices[(3 * w + 4) as usize] = 1;
    terrain.surface_indices[(3 * w + 5) as usize] = 0;
    let chunk = fresh_chunk_runtime(
        Some(terrain),
        vec!["grass".to_string(), "stone".to_string()],
    );
    assert_eq!(surface_at(&chunk, 4.7, 3.2), Some("stone"));
    assert_eq!(surface_at(&chunk, 5.0, 3.2), Some("grass"));
}

#[test]
fn surface_at_out_of_bounds_returns_none() {
    let chunk = flat_chunk(0.0);
    assert_eq!(surface_at(&chunk, -0.1, 5.0), None);
    assert_eq!(surface_at(&chunk, 128.1, 5.0), None);
    // Shared-edge: 128.0 is in-domain.
    assert!(surface_at(&chunk, 128.0, 5.0).is_some());
}

#[test]
fn surface_at_no_terrain_returns_none() {
    let chunk = fresh_chunk_runtime(None, vec!["unused".to_string()]);
    assert_eq!(surface_at(&chunk, 5.0, 5.0), None);
}

#[test]
fn surface_at_with_unresolvable_tag_index_returns_none() {
    // A RuntimeTerrain whose surface_indices reference a tag beyond the runtime table is
    // malformed (the slicer should never produce this). surface_at surfaces None rather than
    // panicking; this locks the "surface rather than panic" contract.
    let mut terrain = flat_terrain(0.0);
    terrain.surface_indices[0] = 7;
    let chunk = fresh_chunk_runtime(Some(terrain), vec!["grass".to_string()]);
    assert_eq!(surface_at(&chunk, 0.0, 0.0), None);
}
