//! Per-chunk authored terrain heightmap.
//!
//! On disk a chunk's terrain is split across two files: the chunk JSON, which carries only
//! the `heightmap_file` reference, and a sibling binary file at that path, which carries the
//! heights, surface indices, surface tag table, and vertical range. The in-memory `TerrainData`
//! holds all of these together; `serde` only sees the reference (`Serialize` emits just
//! `heightmap_file`, `Deserialize` parses just `heightmap_file` and leaves the rest as
//! placeholders). `load_chunk` and `save_chunk` are responsible for the sibling-binary I/O
//! that fills the placeholders in or writes the binary out.
//!
//! Determinism contract: `save_chunk` sorts `surface_tags` alphabetically before write and
//! rewrites `surface_indices` through the resulting permutation, so the on-disk binary is
//! byte-identical across two saves of equivalent in-memory data regardless of authored tag
//! order. The in-memory `TerrainData` is not mutated; sorting operates on a local copy.

use std::path::Path;

use pantry::serde::de::Error as DeError;
use pantry::serde::ser::SerializeStruct;
use pantry::serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::{LoadError, SaveError};

/// Per-chunk authored terrain heightmap data.
#[derive(Clone, Debug)]
pub struct TerrainData {
    /// Heightmap, row-major. Length is `CELL_COUNT`. Indexing: `heights[z * CELLS_PER_AXIS + x]`.
    /// Origin is chunk-local `(0, 0)`; `+x` and `+z` run along the chunk's primary axes.
    /// Values are u16-quantized to the range `[-vertical_range_meters, +vertical_range_meters]`:
    /// value 0 maps to `-vertical_range_meters`, value 65535 maps to `+vertical_range_meters`.
    pub heights: Box<[u16]>,
    /// Surface tag index per cell, same length as `heights`. References positions in
    /// `surface_tags`. Values must be `< surface_tags.len()` for a well-formed instance.
    pub surface_indices: Box<[u16]>,
    /// Per-chunk authored surface tag table. Sorted alphabetically by the save path; in-memory
    /// order is whatever the author or loader produced.
    pub surface_tags: Vec<String>,
    /// Vertical range in meters. Heights are quantized into `[-vertical_range_meters, +v]`.
    pub vertical_range_meters: f32,
    /// Filename (relative, no directory component) of the sibling binary that holds the
    /// heightmap data, e.g. `"0_0.heightmap.bin"`. Resolved against the chunk file's directory
    /// by `load_chunk` / `save_chunk`. Persists through round-trips so the on-disk JSON stays
    /// byte-identical.
    pub heightmap_file: String,
}

impl TerrainData {
    /// Cells along each axis under the shared-edge convention (boundary rows are part of the
    /// chunk's data, duplicated to the neighbor). 129 = 128m + a shared right/bottom edge.
    pub const CELLS_PER_AXIS: u32 = 129;
    /// Total cell count = `CELLS_PER_AXIS * CELLS_PER_AXIS` = 16641.
    pub const CELL_COUNT: usize =
        (Self::CELLS_PER_AXIS as usize) * (Self::CELLS_PER_AXIS as usize);
    /// Sibling binary magic: ASCII `WTRN` (Wok Terrain).
    pub const MAGIC: [u8; 4] = *b"WTRN";
    /// Sibling binary format version. Bumped together with this crate's `_format`.
    pub const FORMAT_VERSION: u16 = 1;
    /// Fixed 16-byte header size.
    pub const HEADER_SIZE: usize = 16;
}

impl PartialEq for TerrainData {
    fn eq(&self, other: &Self) -> bool {
        self.heights == other.heights
            && self.surface_indices == other.surface_indices
            && self.surface_tags == other.surface_tags
            && self.vertical_range_meters == other.vertical_range_meters
            && self.heightmap_file == other.heightmap_file
    }
}

// Serialize emits just `{"heightmap_file": "<name>"}`; the heightmap bytes live in the sibling
// binary and are handled by `save_chunk`, not serde.
impl Serialize for TerrainData {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut s = serializer.serialize_struct("TerrainData", 1)?;
        s.serialize_field("heightmap_file", &self.heightmap_file)?;
        s.end()
    }
}

// Deserialize parses just `{"heightmap_file": "<name>"}` and leaves heights, surface_indices,
// surface_tags, and vertical_range_meters as placeholders. `load_chunk` fills them in by
// reading the referenced sibling binary; consumers that bypass `load_chunk` get a partially
// populated `TerrainData` that is only useful as a reference, not as heightmap data.
impl<'de> Deserialize<'de> for TerrainData {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(crate = "pantry::serde", deny_unknown_fields)]
        struct Wire {
            heightmap_file: String,
        }
        let wire = Wire::deserialize(deserializer)?;
        // Plan section 4: absolute paths are rejected at load time. The wire-form parse is the
        // earliest catch; surfaces as a `serde::de::Error` so the outer `LoadError::Parse`
        // wraps it with the chunk path for context.
        if Path::new(&wire.heightmap_file).is_absolute() {
            return Err(D::Error::custom(format!(
                "terrain heightmap_file must be relative, got absolute path {:?}",
                wire.heightmap_file
            )));
        }
        if wire.heightmap_file.contains('/') || wire.heightmap_file.contains('\\') {
            return Err(D::Error::custom(format!(
                "terrain heightmap_file must be a bare filename with no directory component, got {:?}",
                wire.heightmap_file
            )));
        }
        Ok(TerrainData {
            heights: Box::new([]),
            surface_indices: Box::new([]),
            surface_tags: Vec::new(),
            vertical_range_meters: 0.0,
            heightmap_file: wire.heightmap_file,
        })
    }
}

/// Read a sibling binary file into the heightmap fields of `terrain`. Treats a missing file
/// as `LoadError::TerrainSiblingMissing` (the JSON references a binary that does not exist on
/// disk) and any structural problem (bad magic, wrong version, length mismatch, surface index
/// out of range) as `LoadError::TerrainMalformed`.
pub(crate) fn read_sibling_into(
    terrain: &mut TerrainData,
    chunk_path: &Path,
    sibling_path: &Path,
) -> Result<(), LoadError> {
    let bytes = match std::fs::read(sibling_path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(LoadError::TerrainSiblingMissing {
                chunk_path: chunk_path.to_owned(),
                terrain_path: sibling_path.to_owned(),
            });
        }
        Err(source) => {
            return Err(LoadError::Io {
                path: sibling_path.to_owned(),
                source,
            });
        }
    };
    parse_sibling(&bytes, terrain, sibling_path)
}

fn parse_sibling(bytes: &[u8], terrain: &mut TerrainData, path: &Path) -> Result<(), LoadError> {
    let malformed = |reason: String| LoadError::TerrainMalformed {
        terrain_path: path.to_owned(),
        reason,
    };

    if bytes.len() < TerrainData::HEADER_SIZE {
        return Err(malformed(format!(
            "binary truncated: {} bytes, need at least {} for the header",
            bytes.len(),
            TerrainData::HEADER_SIZE
        )));
    }
    if bytes[0..4] != TerrainData::MAGIC {
        return Err(malformed(format!(
            "magic mismatch: expected {:?}, got {:?}",
            std::str::from_utf8(&TerrainData::MAGIC).unwrap_or("WTRN"),
            String::from_utf8_lossy(&bytes[0..4])
        )));
    }
    let format_version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if format_version != TerrainData::FORMAT_VERSION {
        return Err(malformed(format!(
            "unsupported format_version {format_version}, expected {}",
            TerrainData::FORMAT_VERSION
        )));
    }
    let width = u16::from_le_bytes([bytes[6], bytes[7]]);
    let height_along_z = u16::from_le_bytes([bytes[8], bytes[9]]);
    if u32::from(width) != TerrainData::CELLS_PER_AXIS
        || u32::from(height_along_z) != TerrainData::CELLS_PER_AXIS
    {
        return Err(malformed(format!(
            "resolution mismatch: header says {width}x{height_along_z}, expected {0}x{0}",
            TerrainData::CELLS_PER_AXIS
        )));
    }
    let surface_tag_count = u16::from_le_bytes([bytes[10], bytes[11]]);
    let vertical_range_mm =
        u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);

    let mut offset = TerrainData::HEADER_SIZE;

    let mut tags: Vec<String> = Vec::with_capacity(usize::from(surface_tag_count));
    for tag_idx in 0..usize::from(surface_tag_count) {
        if offset + 2 > bytes.len() {
            return Err(malformed(format!(
                "binary truncated: surface tag {tag_idx} length prefix at offset {offset} runs past end ({})",
                bytes.len()
            )));
        }
        let tag_len = usize::from(u16::from_le_bytes([bytes[offset], bytes[offset + 1]]));
        offset += 2;
        if offset + tag_len > bytes.len() {
            return Err(malformed(format!(
                "binary truncated: surface tag {tag_idx} bytes at offset {offset}..{} run past end ({})",
                offset + tag_len,
                bytes.len()
            )));
        }
        let tag = std::str::from_utf8(&bytes[offset..offset + tag_len])
            .map_err(|e| malformed(format!("surface tag {tag_idx} is not valid UTF-8: {e}")))?
            .to_string();
        offset += tag_len;
        tags.push(tag);
    }

    let cell_count = TerrainData::CELL_COUNT;
    let cells_bytes = cell_count * 2;
    let expected_end = offset + cells_bytes * 2;
    if bytes.len() < expected_end {
        return Err(malformed(format!(
            "binary truncated: heights+indices need {} bytes from offset {}, have {}",
            cells_bytes * 2,
            offset,
            bytes.len() - offset
        )));
    }
    if bytes.len() > expected_end {
        return Err(malformed(format!(
            "binary has {} trailing bytes after expected end {expected_end}",
            bytes.len() - expected_end
        )));
    }

    let mut heights: Vec<u16> = Vec::with_capacity(cell_count);
    for _ in 0..cell_count {
        heights.push(u16::from_le_bytes([bytes[offset], bytes[offset + 1]]));
        offset += 2;
    }
    let mut surface_indices: Vec<u16> = Vec::with_capacity(cell_count);
    for cell_idx in 0..cell_count {
        let v = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]);
        if usize::from(v) >= tags.len() {
            return Err(malformed(format!(
                "surface index at cell {cell_idx} references tag {v} but only {} tag(s) declared",
                tags.len()
            )));
        }
        surface_indices.push(v);
        offset += 2;
    }
    debug_assert_eq!(offset, bytes.len());

    terrain.heights = heights.into_boxed_slice();
    terrain.surface_indices = surface_indices.into_boxed_slice();
    terrain.surface_tags = tags;
    // u32 millimeters -> f32 meters. Fits exactly for vertical_range_mm <= 2^24, which covers
    // any sane authoring range (32000 default is six orders of magnitude smaller).
    #[allow(clippy::cast_precision_loss)]
    {
        terrain.vertical_range_meters = (vertical_range_mm as f32) / 1000.0;
    }
    Ok(())
}

/// Write `terrain`'s heightmap fields to `sibling_path`. Surface tags are sorted
/// alphabetically and surface_indices are remapped through the resulting permutation. The
/// in-memory `terrain` is not mutated. Returns `SaveError::Io` on filesystem failures.
///
/// Invariants assumed of `terrain`: `heights.len() == surface_indices.len() == CELL_COUNT`,
/// every entry of `surface_indices` is `< surface_tags.len()`, and `surface_tags.len()` fits
/// in `u16`. Violations are programmer bugs; debug builds fire `debug_assert!`, release builds
/// produce a binary that fails to load back.
pub(crate) fn write_sibling(terrain: &TerrainData, sibling_path: &Path) -> Result<(), SaveError> {
    debug_assert_eq!(terrain.heights.len(), TerrainData::CELL_COUNT);
    debug_assert_eq!(terrain.surface_indices.len(), TerrainData::CELL_COUNT);
    debug_assert!(u16::try_from(terrain.surface_tags.len()).is_ok());

    let n_tags = terrain.surface_tags.len();
    let mut order: Vec<usize> = (0..n_tags).collect();
    order.sort_by(|&a, &b| terrain.surface_tags[a].cmp(&terrain.surface_tags[b]));
    // remap[old_idx] = new_idx after sort.
    let mut remap = vec![0u16; n_tags];
    for (new_idx, &old_idx) in order.iter().enumerate() {
        remap[old_idx] = new_idx as u16;
    }
    let sorted_tags: Vec<&str> = order
        .iter()
        .map(|&i| terrain.surface_tags[i].as_str())
        .collect();

    let payload_bytes: usize = sorted_tags.iter().map(|t| 2 + t.len()).sum::<usize>()
        + TerrainData::CELL_COUNT * 4;
    let mut out: Vec<u8> = Vec::with_capacity(TerrainData::HEADER_SIZE + payload_bytes);

    out.extend_from_slice(&TerrainData::MAGIC);
    out.extend_from_slice(&TerrainData::FORMAT_VERSION.to_le_bytes());
    let cells_per_axis = TerrainData::CELLS_PER_AXIS as u16;
    out.extend_from_slice(&cells_per_axis.to_le_bytes());
    out.extend_from_slice(&cells_per_axis.to_le_bytes());
    out.extend_from_slice(&(n_tags as u16).to_le_bytes());
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    let vertical_range_mm = (terrain.vertical_range_meters * 1000.0).round() as u32;
    out.extend_from_slice(&vertical_range_mm.to_le_bytes());

    for tag in &sorted_tags {
        let bytes = tag.as_bytes();
        let len = u16::try_from(bytes.len()).expect("surface tag length fits in u16");
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(bytes);
    }

    for &h in &*terrain.heights {
        out.extend_from_slice(&h.to_le_bytes());
    }
    for &idx in &*terrain.surface_indices {
        let mapped = remap.get(usize::from(idx)).copied().unwrap_or(idx);
        out.extend_from_slice(&mapped.to_le_bytes());
    }

    std::fs::write(sibling_path, &out).map_err(|source| SaveError::Io {
        path: sibling_path.to_owned(),
        source,
    })
}
