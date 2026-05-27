//! Runtime form of an authored `TerrainData`. Produced by `slice_chunk` and attached to the
//! enclosing `ChunkRuntime`. Position-independent: the same `TerrainData` slices into a
//! byte-identical `RuntimeTerrain` regardless of the chunk's `ChunkCoord`. Surface indices
//! reference the merged `ChunkRuntime::surface_tag_table` (not the authored
//! `TerrainData::surface_tags`); the slicer rewrites them through the merge.

/// Per-chunk runtime terrain heightmap.
#[derive(Clone, Debug, PartialEq)]
pub struct RuntimeTerrain {
    /// Heightmap, copied unchanged from `TerrainData::heights`. Length is `width * width`
    /// (always 16641 under the shared-edge convention). Indexing: `heights[z * width + x]`.
    pub heights: Box<[u16]>,

    /// Surface tag index per cell, same length as `heights`. References positions in the
    /// enclosing `ChunkRuntime::surface_tag_table`. The slicer rewrites the authored indices
    /// during the surface-table merge so a downstream consumer never sees authored-terrain
    /// indices in isolation.
    pub surface_indices: Box<[u16]>,

    /// Cell count along each axis. Always 129 under the locked shared-edge convention;
    /// carried explicitly so samplers can bounds-check without referencing a crate constant
    /// and so future authoring resolutions could be supported by varying this value.
    pub width: u32,

    /// Vertical range in meters, copied from `TerrainData`. Sampling functions use this to
    /// dequantize the u16 heights back to meters before interpolation.
    pub vertical_range_meters: f32,
}
