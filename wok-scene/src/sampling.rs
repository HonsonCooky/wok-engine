//! Pure terrain sampling at chunk-local coordinates.
//!
//! All three functions take `&ChunkRuntime` so call sites have a uniform shape (plan §9
//! "Sampling signature uniformity"). The terrain-absent case (`chunk.terrain == None`) is
//! handled identically across the three: a `?` on `chunk.terrain.as_ref()` early-returns
//! `None`. Callers do not need to pre-check `terrain.is_some()`.
//!
//! Domain is chunk-local `[0, 128]` closed-closed (the shared-edge convention: cells at
//! `x = 128` and `z = 128` are valid samples that duplicate the neighbor chunk's `x = 0`
//! and `z = 0` edge). Out-of-domain inputs return `None`. NaN inputs are also out-of-domain
//! by definition (NaN comparisons are false, so the range check rejects them).

use pantry::math::Vec3;

use crate::runtime::{ChunkRuntime, RuntimeTerrain};

/// Sample interpolated height at a chunk-local position. Returns chunk-local height in
/// meters, bilinearly interpolated between the four enclosing cells. Returns `None` if the
/// chunk has no terrain or if `(x, z)` is outside `[0, 128]` on either axis.
pub fn height_at(chunk: &ChunkRuntime, chunk_local_x: f32, chunk_local_z: f32) -> Option<f32> {
    let terrain = chunk.terrain.as_ref()?;
    sample_height(terrain, chunk_local_x, chunk_local_z)
}

/// Sample surface normal at a chunk-local position. Returned `Vec3` is unit-length and in
/// chunk-local space; consumers compose with the chunk transform for world space. Computed
/// from a 1-cell central-difference gradient (forward or backward one-sided at the chunk
/// boundary). Returns `None` if the chunk has no terrain or if `(x, z)` is outside the valid
/// domain.
#[allow(clippy::similar_names)]
pub fn normal_at(chunk: &ChunkRuntime, chunk_local_x: f32, chunk_local_z: f32) -> Option<Vec3> {
    let terrain = chunk.terrain.as_ref()?;
    if !in_domain(terrain, chunk_local_x, chunk_local_z) {
        return None;
    }
    let max = max_coord(terrain);
    // Cell size is fixed at 1m (the authoring resolution). Use central-difference where both
    // neighbors are in-domain, one-sided forward/backward at the chunk boundary. dx_span is
    // 2.0 in the interior and 1.0 at a boundary; dividing by it produces the correct slope
    // either way. The shared-edge convention means cells 0 and `max` are duplicated to the
    // neighbor, so a one-sided difference at the boundary is consistent with the neighbor's
    // value (no cross-chunk reach needed). Don't smooth across the boundary - that
    // restriction is a 3-cell-window concern that does not apply here.
    let cell = 1.0_f32;
    let xm = (chunk_local_x - cell).max(0.0);
    let xp = (chunk_local_x + cell).min(max);
    let zm = (chunk_local_z - cell).max(0.0);
    let zp = (chunk_local_z + cell).min(max);
    let h_xm = sample_height(terrain, xm, chunk_local_z)?;
    let h_xp = sample_height(terrain, xp, chunk_local_z)?;
    let h_zm = sample_height(terrain, chunk_local_x, zm)?;
    let h_zp = sample_height(terrain, chunk_local_x, zp)?;
    let dh_dx = (h_xp - h_xm) / (xp - xm);
    let dh_dz = (h_zp - h_zm) / (zp - zm);
    Some(Vec3::new(-dh_dx, 1.0, -dh_dz).normalize())
}

/// Look up the surface tag at a chunk-local position. Returns a borrowed string from
/// `chunk.surface_tag_table` (lifetime tied to the borrowed `ChunkRuntime` via elision).
/// Returns `None` if the chunk has no terrain, `(x, z)` is outside the valid domain, or the
/// cell's surface index does not resolve in the runtime table (which would indicate a
/// slicer bug; surfaced rather than panicked).
#[allow(clippy::cast_sign_loss)]
pub fn surface_at(chunk: &ChunkRuntime, chunk_local_x: f32, chunk_local_z: f32) -> Option<&str> {
    let terrain = chunk.terrain.as_ref()?;
    if !in_domain(terrain, chunk_local_x, chunk_local_z) {
        return None;
    }
    let max = max_coord(terrain);
    // Cell index is the integer floor of the sample coordinate, clamped to the last valid
    // index. The cast is sign-safe: in_domain guarantees the floor is in `[0, max]`, all
    // non-negative.
    let i = (chunk_local_x.floor().clamp(0.0, max)) as u32;
    let j = (chunk_local_z.floor().clamp(0.0, max)) as u32;
    let cell_idx = (j * terrain.width + i) as usize;
    let tag_idx = *terrain.surface_indices.get(cell_idx)?;
    chunk
        .surface_tag_table
        .get(usize::from(tag_idx))
        .map(String::as_str)
}

#[inline]
#[allow(clippy::cast_precision_loss)]
fn max_coord(terrain: &RuntimeTerrain) -> f32 {
    // CELLS_PER_AXIS - 1 in meters. For the locked 129-cell convention this is 128.0. The
    // cast is precision-safe: u32 values up to 16777216 (2^24) round-trip exactly through f32;
    // CELLS_PER_AXIS is 129.
    (terrain.width as f32) - 1.0
}

#[inline]
fn in_domain(terrain: &RuntimeTerrain, x: f32, z: f32) -> bool {
    let max = max_coord(terrain);
    (0.0..=max).contains(&x) && (0.0..=max).contains(&z)
}

/// Bilinear interpolation of dequantized heights. Assumes the caller has not pre-checked
/// the domain; performs its own check and returns `None` if `(x, z)` is out of range.
#[allow(clippy::cast_sign_loss, clippy::similar_names)]
fn sample_height(terrain: &RuntimeTerrain, x: f32, z: f32) -> Option<f32> {
    if !in_domain(terrain, x, z) {
        return None;
    }
    let max = max_coord(terrain);
    let x_floor = x.floor();
    let z_floor = z.floor();
    // The shared-edge sample at x = max collapses x_floor == x_next cleanly: x_next clamps
    // to max, fx is 0, and the interpolation degenerates to h_x0 along that axis.
    let x_next = (x_floor + 1.0).min(max);
    let z_next = (z_floor + 1.0).min(max);
    let fx = x - x_floor;
    let fz = z - z_floor;
    // Casts are sign-safe: in_domain guarantees the floors are in `[0, max]`, all
    // non-negative.
    let xa = x_floor as u32;
    let xb = x_next as u32;
    let za = z_floor as u32;
    let zb = z_next as u32;
    let w = terrain.width;
    let vr = terrain.vertical_range_meters;
    let h_a_a = dequantize(terrain.heights[(za * w + xa) as usize], vr);
    let h_b_a = dequantize(terrain.heights[(za * w + xb) as usize], vr);
    let h_a_b = dequantize(terrain.heights[(zb * w + xa) as usize], vr);
    let h_b_b = dequantize(terrain.heights[(zb * w + xb) as usize], vr);
    let h_along_x_at_za = h_a_a * (1.0 - fx) + h_b_a * fx;
    let h_along_x_at_zb = h_a_b * (1.0 - fx) + h_b_b * fx;
    Some(h_along_x_at_za * (1.0 - fz) + h_along_x_at_zb * fz)
}

/// Map a u16-quantized height back to meters using the terrain's vertical range. u16 0 maps
/// to `-vertical_range_meters`; u16 `u16::MAX` maps to `+vertical_range_meters`; the mapping
/// is linear in between.
#[inline]
fn dequantize(h: u16, vertical_range_meters: f32) -> f32 {
    let t = f32::from(h) / f32::from(u16::MAX);
    -vertical_range_meters + t * 2.0 * vertical_range_meters
}
