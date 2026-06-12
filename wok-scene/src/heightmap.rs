//! Per-chunk terrain: the `Heightmap` type, its construction invariant, and sampling.
//!
//! A chunk is 128m x 128m. Terrain is a `CHUNK_GRID_DIM` x `CHUNK_GRID_DIM` (129 x 129) grid of
//! samples at 1m spacing, so a sample sits on every metre line from 0 to 128 inclusive. The
//! extra row and column are the *shared edge*: a chunk's row/column 128 holds the same values
//! as its neighbour's row/column 0, which is what makes terrain seamless across the seam
//! without either chunk needing to read the other.
//!
//! Heights are `u16`, mapped linearly onto `HEIGHT_MIN_M..=HEIGHT_MAX_M` (-32m..+32m): raw 0 is
//! the floor, raw 65535 the ceiling, about 1mm per step. Per-cell surface material is interned:
//! a table of unique `SurfaceTag`s plus a grid of `u16` indices into that table. Interning
//! keeps the per-cell cost to two bytes and writes each material name once.
//!
//! Both grids are flat `Vec`s with a hard `CHUNK_GRID_LEN`-length invariant, and every surface
//! index must point inside the table. Those invariants are what the sampling functions assume,
//! so the only way to build a `Heightmap` is `Heightmap::new`, which checks them and returns
//! `HeightmapError` on violation; the fields stay private. The binary loader
//! (`crate::heightmap_io`) routes the same failures through `LoadError`. Sampling is the public
//! read path; raw-grid accessors are deferred until a consumer (wok-mesh terrain generation)
//! actually needs them.

use glam::Vec3;

use crate::refs::SurfaceTag;

/// Samples per chunk side. Engine constant: 129 = 128 one-metre cells plus the shared edge.
pub const CHUNK_GRID_DIM: usize = 129;

/// Total samples in a chunk grid (`CHUNK_GRID_DIM` squared); the length both grids must have.
pub const CHUNK_GRID_LEN: usize = CHUNK_GRID_DIM * CHUNK_GRID_DIM;

/// Lowest height a raw `0` maps to, in metres.
pub const HEIGHT_MIN_M: f32 = -32.0;

/// Highest height a raw `u16::MAX` maps to, in metres.
pub const HEIGHT_MAX_M: f32 = 32.0;

/// The largest valid chunk-local sample coordinate, in metres (the shared edge).
const MAX_COORD: f32 = (CHUNK_GRID_DIM - 1) as f32;

/// Ways the parts handed to `Heightmap::new` can fail to form a valid heightmap.
///
/// Independent from `crate::LoadError`: this is about the shape of in-memory data, not about
/// reading a file. The binary loader wraps these into `LoadError::Heightmap` once it has a path.
#[derive(Debug, thiserror::Error)]
pub enum HeightmapError {
    #[error("height grid has {got} cells, expected a 129x129 grid ({CHUNK_GRID_LEN} cells)")]
    HeightGridLen { got: usize },

    #[error("surface-index grid has {got} cells, expected a 129x129 grid ({CHUNK_GRID_LEN} cells)")]
    SurfaceGridLen { got: usize },

    #[error("surface index {index} at cell {cell} is past the end of the {table_len}-entry table")]
    SurfaceIndexOutOfRange {
        cell: usize,
        index: usize,
        table_len: usize,
    },
}

/// A chunk's terrain: a height grid plus an interned per-cell surface grid.
///
/// Construct with `Heightmap::new`; load and save with `crate::load_heightmap` /
/// `crate::save_heightmap`. See the module docs for the grid layout and invariants.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heightmap {
    // Row-major, z-major: cell (x, z) lives at index `z * CHUNK_GRID_DIM + x`. Both grids are
    // exactly CHUNK_GRID_LEN long, and every `surface_indices` entry is `< surface_table.len()`;
    // `new` is the only constructor and enforces both.
    pub(crate) heights: Vec<u16>,
    pub(crate) surface_table: Vec<SurfaceTag>,
    pub(crate) surface_indices: Vec<u16>,
}

impl Heightmap {
    /// Build a heightmap from its grids and interned surface table, checking the invariants.
    ///
    /// Both grids must be exactly `CHUNK_GRID_LEN` long and every surface index must point
    /// inside `surface_table`, otherwise the corresponding `HeightmapError` is returned.
    pub fn new(
        heights: Vec<u16>,
        surface_table: Vec<SurfaceTag>,
        surface_indices: Vec<u16>,
    ) -> Result<Self, HeightmapError> {
        if heights.len() != CHUNK_GRID_LEN {
            return Err(HeightmapError::HeightGridLen { got: heights.len() });
        }
        if surface_indices.len() != CHUNK_GRID_LEN {
            return Err(HeightmapError::SurfaceGridLen {
                got: surface_indices.len(),
            });
        }
        for (cell, &index) in surface_indices.iter().enumerate() {
            if index as usize >= surface_table.len() {
                return Err(HeightmapError::SurfaceIndexOutOfRange {
                    cell,
                    index: index as usize,
                    table_len: surface_table.len(),
                });
            }
        }
        Ok(Self {
            heights,
            surface_table,
            surface_indices,
        })
    }

    /// Convert a raw height sample to metres. Inverse of `meters_to_raw` (up to quantization).
    pub fn raw_to_meters(raw: u16) -> f32 {
        HEIGHT_MIN_M + (raw as f32 / u16::MAX as f32) * (HEIGHT_MAX_M - HEIGHT_MIN_M)
    }

    /// Quantize a height in metres to a raw sample, clamping to the representable range.
    pub fn meters_to_raw(meters: f32) -> u16 {
        let t = ((meters - HEIGHT_MIN_M) / (HEIGHT_MAX_M - HEIGHT_MIN_M)).clamp(0.0, 1.0);
        (t * u16::MAX as f32).round() as u16
    }

    /// Interpolated terrain height at chunk-local `(x, z)` in metres, by bilinear blend of the
    /// four surrounding samples. Coordinates outside `0..=128` clamp to the edge.
    pub fn height_at(&self, x: f32, z: f32) -> f32 {
        let (x0, fx) = cell_lerp(x);
        let (z0, fz) = cell_lerp(z);
        let h00 = self.height_m(x0, z0);
        let h10 = self.height_m(x0 + 1, z0);
        let h01 = self.height_m(x0, z0 + 1);
        let h11 = self.height_m(x0 + 1, z0 + 1);
        let h0 = h00 + (h10 - h00) * fx;
        let h1 = h01 + (h11 - h01) * fx;
        h0 + (h1 - h0) * fz
    }

    /// Surface normal at chunk-local `(x, z)`, derived from the height gradient.
    ///
    /// Uses a one-cell central difference: the cheap, faceted-but-adequate default. Revisit
    /// trigger: if cel-band shimmer shows up on rendered sloped terrain, widen the window to
    /// three cells (sample the gradient over a wider neighbourhood to smooth the bands).
    pub fn normal_at(&self, x: f32, z: f32) -> Vec3 {
        // Span is 2m (+/-1 cell), so the per-metre slope is the half-difference. Sampling
        // through `height_at` means the +/-1 reads clamp cleanly at the chunk edge.
        let dx = (self.height_at(x + 1.0, z) - self.height_at(x - 1.0, z)) / 2.0;
        let dz = (self.height_at(x, z + 1.0) - self.height_at(x, z - 1.0)) / 2.0;
        // Normal to the surface y = h(x, z): cross of the two tangents, pointing up.
        Vec3::new(-dx, 1.0, -dz).normalize()
    }

    /// Surface tag of the cell containing chunk-local `(x, z)` (nearest sample, no blend - tags
    /// are discrete). `None` only for a heightmap whose data does not resolve; one built through
    /// `new` or loaded through the loader always resolves, because both validate the indices.
    ///
    /// Per-surface friction and feel driven by these tags is this query's designed use: a game
    /// samples the tag under the body and varies its ground handling per surface (ice slides, mud
    /// drags). Parked until content actually wants differing surfaces; the game-side hook is
    /// taste's ground-friction application (the grounded approach rates in its locomotion step).
    pub fn surface_at(&self, x: f32, z: f32) -> Option<&SurfaceTag> {
        let index = self.surface_indices[idx(nearest_cell(x), nearest_cell(z))] as usize;
        self.surface_table.get(index)
    }

    /// Height in metres at an exact sample cell. Relies on the `CHUNK_GRID_LEN` invariant.
    fn height_m(&self, cx: usize, cz: usize) -> f32 {
        Self::raw_to_meters(self.heights[idx(cx, cz)])
    }
}

/// Flatten a cell coordinate to a grid index (z-major). Both inputs must be `< CHUNK_GRID_DIM`.
fn idx(cx: usize, cz: usize) -> usize {
    cz * CHUNK_GRID_DIM + cx
}

/// Map a chunk-local metre coordinate to its lower sample index and the fraction toward the
/// next one. The lower index is capped at `CHUNK_GRID_DIM - 2` so the upper sample (`+1`) is
/// always in range; at the far edge the fraction reaches 1.0 and lands exactly on the edge
/// sample, which is the shared-edge value.
fn cell_lerp(coord: f32) -> (usize, f32) {
    let clamped = coord.clamp(0.0, MAX_COORD);
    let lower = (clamped.floor() as usize).min(CHUNK_GRID_DIM - 2);
    (lower, clamped - lower as f32)
}

/// Map a chunk-local metre coordinate to the nearest sample cell, clamped to the grid.
fn nearest_cell(coord: f32) -> usize {
    (coord.clamp(0.0, MAX_COORD).round() as usize).min(CHUNK_GRID_DIM - 1)
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    // A grid where every sample holds the same raw height.
    fn uniform(raw: u16) -> Vec<u16> {
        vec![raw; CHUNK_GRID_LEN]
    }

    // A grid that ramps along x by `delta` raw units per cell (independent of z).
    fn ramp_x(base: u16, delta: u16) -> Vec<u16> {
        (0..CHUNK_GRID_LEN)
            .map(|i| base + (i % CHUNK_GRID_DIM) as u16 * delta)
            .collect()
    }

    // ---- quantization ----

    #[test]
    fn quantization_endpoints_are_exact() {
        assert_eq!(Heightmap::raw_to_meters(0), HEIGHT_MIN_M);
        assert_eq!(Heightmap::raw_to_meters(u16::MAX), HEIGHT_MAX_M);
        assert_eq!(Heightmap::meters_to_raw(HEIGHT_MIN_M), 0);
        assert_eq!(Heightmap::meters_to_raw(HEIGHT_MAX_M), u16::MAX);
    }

    #[test]
    fn meters_to_raw_clamps_out_of_range() {
        assert_eq!(Heightmap::meters_to_raw(-1000.0), 0);
        assert_eq!(Heightmap::meters_to_raw(1000.0), u16::MAX);
    }

    #[test]
    fn meters_round_trip_within_one_step() {
        // One raw step is ~0.977mm; quantization error is bounded by half a step.
        let step = (HEIGHT_MAX_M - HEIGHT_MIN_M) / u16::MAX as f32;
        for m in [-32.0, -10.5, 0.0, 7.25, 31.9] {
            let back = Heightmap::raw_to_meters(Heightmap::meters_to_raw(m));
            assert!((back - m).abs() <= step, "m={m} back={back}");
        }
    }

    // ---- new: invariants ----

    #[test]
    fn new_accepts_a_well_formed_grid() {
        let hm = Heightmap::new(uniform(0), vec![SurfaceTag::new("grass")], uniform(0)).unwrap();
        assert_eq!(hm.surface_at(0.0, 0.0), Some(&SurfaceTag::new("grass")));
    }

    #[test]
    fn new_rejects_wrong_height_len() {
        let err = Heightmap::new(vec![0; 10], vec![], uniform(0)).unwrap_err();
        assert!(matches!(err, HeightmapError::HeightGridLen { got: 10 }));
    }

    #[test]
    fn new_rejects_wrong_index_len() {
        let err = Heightmap::new(uniform(0), vec![SurfaceTag::new("g")], vec![0; 5]).unwrap_err();
        assert!(matches!(err, HeightmapError::SurfaceGridLen { got: 5 }));
    }

    #[test]
    fn new_rejects_index_past_table_end() {
        // Table has one entry (valid index 0); cell 0 points at index 1.
        let mut indices = uniform(0);
        indices[0] = 1;
        let err = Heightmap::new(uniform(0), vec![SurfaceTag::new("g")], indices).unwrap_err();
        match err {
            HeightmapError::SurfaceIndexOutOfRange {
                cell,
                index,
                table_len,
            } => {
                assert_eq!((cell, index, table_len), (0, 1, 1));
            }
            other => panic!("expected SurfaceIndexOutOfRange, got {other:?}"),
        }
    }

    #[test]
    fn new_rejects_any_index_into_empty_table() {
        let err = Heightmap::new(uniform(0), vec![], uniform(0)).unwrap_err();
        assert!(matches!(
            err,
            HeightmapError::SurfaceIndexOutOfRange { table_len: 0, .. }
        ));
    }

    // ---- height_at ----

    #[test]
    fn height_at_is_constant_over_flat_ground() {
        let raw = Heightmap::meters_to_raw(5.0);
        let hm = Heightmap::new(uniform(raw), vec![SurfaceTag::new("g")], uniform(0)).unwrap();
        let expected = Heightmap::raw_to_meters(raw);
        for (x, z) in [(0.0, 0.0), (12.5, 70.25), (128.0, 128.0)] {
            assert_eq!(hm.height_at(x, z), expected);
        }
    }

    #[test]
    fn height_at_returns_exact_cell_height_at_integer_coords() {
        let hm = Heightmap::new(ramp_x(0, 100), vec![SurfaceTag::new("g")], uniform(0)).unwrap();
        // Interior integer x lands exactly on a sample: no interpolation.
        assert_eq!(hm.height_at(64.0, 10.0), Heightmap::raw_to_meters(6400));
        assert_eq!(hm.height_at(65.0, 10.0), Heightmap::raw_to_meters(6500));
    }

    #[test]
    fn height_at_interpolates_between_cells() {
        let hm = Heightmap::new(ramp_x(0, 100), vec![SurfaceTag::new("g")], uniform(0)).unwrap();
        let lo = Heightmap::raw_to_meters(6400);
        let hi = Heightmap::raw_to_meters(6500);
        // Halfway between x=64 and x=65 is the average of the two cell heights.
        assert!((hm.height_at(64.5, 10.0) - f32::midpoint(lo, hi)).abs() < 1e-4);
    }

    #[test]
    fn height_at_far_edge_uses_shared_edge_sample() {
        let hm = Heightmap::new(ramp_x(0, 100), vec![SurfaceTag::new("g")], uniform(0)).unwrap();
        // x=128 is the shared edge: it must read sample 128, not clamp short to 127.
        assert_eq!(hm.height_at(128.0, 10.0), Heightmap::raw_to_meters(12800));
    }

    // ---- normal_at ----

    #[test]
    fn normal_at_flat_ground_points_up() {
        let hm = Heightmap::new(uniform(30000), vec![SurfaceTag::new("g")], uniform(0)).unwrap();
        assert_eq!(hm.normal_at(64.0, 64.0), Vec3::Y);
    }

    #[test]
    fn normal_at_slope_matches_gradient() {
        let delta = 100u16;
        let hm = Heightmap::new(ramp_x(0, delta), vec![SurfaceTag::new("g")], uniform(0)).unwrap();
        // The ramp rises `slope` metres per metre in +x; the normal tilts back along -x.
        let slope = delta as f32 * (HEIGHT_MAX_M - HEIGHT_MIN_M) / u16::MAX as f32;
        let expected = Vec3::new(-slope, 1.0, 0.0).normalize();
        let n = hm.normal_at(64.0, 64.0);
        assert!((n - expected).length() < 1e-5, "n={n:?} expected={expected:?}");
    }

    // ---- surface_at ----

    #[test]
    fn surface_at_returns_the_cell_tag() {
        let table = vec![SurfaceTag::new("grass"), SurfaceTag::new("stone")];
        let mut indices = uniform(0); // all grass
        indices[idx(10, 20)] = 1; // one stone cell
        let hm = Heightmap::new(uniform(0), table, indices).unwrap();
        assert_eq!(hm.surface_at(0.0, 0.0), Some(&SurfaceTag::new("grass")));
        assert_eq!(hm.surface_at(10.0, 20.0), Some(&SurfaceTag::new("stone")));
    }

    #[test]
    fn surface_at_picks_the_nearest_cell() {
        let table = vec![SurfaceTag::new("grass"), SurfaceTag::new("stone")];
        let mut indices = uniform(0);
        indices[idx(10, 20)] = 1;
        let hm = Heightmap::new(uniform(0), table, indices).unwrap();
        // Rounds to the nearest sample: 10.4 -> cell 10 (stone), 10.6 -> cell 11 (grass).
        assert_eq!(hm.surface_at(10.4, 20.0), Some(&SurfaceTag::new("stone")));
        assert_eq!(hm.surface_at(10.6, 20.0), Some(&SurfaceTag::new("grass")));
    }
}
