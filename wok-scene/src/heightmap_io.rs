//! Binary load and save for `Heightmap`.
//!
//! Heightmaps live in a sibling binary file next to the chunk JSON (`{x}_{z}.heightmap.bin`
//! beside `{x}_{z}.json`). Binary rather than JSON because the grids are ~33KB each and JSON
//! would balloon them; the format is small and fully specified here.
//!
//! Layout, in order, all multi-byte integers little-endian (`to_le_bytes` / `from_le_bytes`,
//! never raw memory casts, so a file written on one machine is byte-identical to one written on
//! another):
//!
//! 1. Header: 4-byte magic `WKHM`, `u16` version, `u32` surface-table length.
//! 2. Surface table: for each entry, a `u32` byte length then that many UTF-8 bytes. Written in
//!    table order, which is the stable order the determinism contract asks for (the table is an
//!    ordered `Vec`, not a map, so there is nothing to sort).
//! 3. Height grid: `CHUNK_GRID_LEN` `u16` samples, z-major.
//! 4. Surface-index grid: `CHUNK_GRID_LEN` `u16` indices, z-major.
//!
//! Encoding is deterministic: identical input yields byte-identical output. Decoding validates
//! the magic, version, every length, the UTF-8 of each tag, and (via `Heightmap::new`) that no
//! index points past the table, surfacing each as a distinct `LoadError`.

use std::path::Path;

use crate::error::{LoadError, SaveError};
use crate::heightmap::{CHUNK_GRID_LEN, Heightmap};
use crate::refs::SurfaceTag;

const MAGIC: [u8; 4] = *b"WKHM";
const VERSION: u16 = 1;

pub fn load_heightmap(path: impl AsRef<Path>) -> Result<Heightmap, LoadError> {
    let path = path.as_ref();
    let bytes = std::fs::read(path).map_err(|source| LoadError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    decode_heightmap(&bytes, path)
}

pub fn save_heightmap(heightmap: &Heightmap, path: impl AsRef<Path>) -> Result<(), SaveError> {
    let path = path.as_ref();
    let bytes = encode_heightmap(heightmap);
    std::fs::write(path, bytes).map_err(|source| SaveError::Io {
        path: path.to_path_buf(),
        source,
    })
}

pub(crate) fn encode_heightmap(hm: &Heightmap) -> Vec<u8> {
    let table_bytes: usize = hm.surface_table.iter().map(|t| 4 + t.as_str().len()).sum();
    let mut out = Vec::with_capacity(10 + table_bytes + hm.heights.len() * 2 + hm.surface_indices.len() * 2);

    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&(hm.surface_table.len() as u32).to_le_bytes());
    for tag in &hm.surface_table {
        let s = tag.as_str().as_bytes();
        out.extend_from_slice(&(s.len() as u32).to_le_bytes());
        out.extend_from_slice(s);
    }
    for &h in &hm.heights {
        out.extend_from_slice(&h.to_le_bytes());
    }
    for &i in &hm.surface_indices {
        out.extend_from_slice(&i.to_le_bytes());
    }
    out
}

pub(crate) fn decode_heightmap(bytes: &[u8], path: &Path) -> Result<Heightmap, LoadError> {
    let mut r = Reader::new(bytes);

    if r.take(4).ok_or_else(|| truncated(path))? != MAGIC.as_slice() {
        return Err(LoadError::BadMagic {
            path: path.to_path_buf(),
        });
    }
    let version = r.u16().ok_or_else(|| truncated(path))?;
    if version != VERSION {
        return Err(LoadError::UnsupportedVersion {
            path: path.to_path_buf(),
            version,
        });
    }

    let table_len = r.u32().ok_or_else(|| truncated(path))? as usize;
    let mut surface_table = Vec::with_capacity(table_len);
    for _ in 0..table_len {
        let len = r.u32().ok_or_else(|| truncated(path))? as usize;
        let raw = r.take(len).ok_or_else(|| truncated(path))?;
        let name = std::str::from_utf8(raw).map_err(|source| LoadError::Utf8 {
            path: path.to_path_buf(),
            source,
        })?;
        surface_table.push(SurfaceTag::new(name));
    }

    let heights = read_grid(&mut r, path)?;
    let surface_indices = read_grid(&mut r, path)?;

    Heightmap::new(heights, surface_table, surface_indices).map_err(|source| LoadError::Heightmap {
        path: path.to_path_buf(),
        source,
    })
}

fn read_grid(r: &mut Reader, path: &Path) -> Result<Vec<u16>, LoadError> {
    let mut grid = Vec::with_capacity(CHUNK_GRID_LEN);
    for _ in 0..CHUNK_GRID_LEN {
        grid.push(r.u16().ok_or_else(|| truncated(path))?);
    }
    Ok(grid)
}

fn truncated(path: &Path) -> LoadError {
    LoadError::Truncated {
        path: path.to_path_buf(),
    }
}

/// A forward-only cursor that yields `None` rather than panicking when the buffer runs out, so
/// every short read becomes a `Truncated` error at the call site.
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.pos.checked_add(n)?;
        let slice = self.bytes.get(self.pos..end)?;
        self.pos = end;
        Some(slice)
    }

    fn u16(&mut self) -> Option<u16> {
        let b = self.take(2)?;
        Some(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Option<u32> {
        let b = self.take(4)?;
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::heightmap::CHUNK_GRID_DIM;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};

    fn unique_temp() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let pid = std::process::id();
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("wok-scene-hm-{pid}-{n}.heightmap.bin"))
    }

    // A heightmap with a non-trivial table and a recognizable height/index pattern.
    fn sample() -> Heightmap {
        let table = vec![
            SurfaceTag::new("grass"),
            SurfaceTag::new("stone"),
            SurfaceTag::new("sand"),
        ];
        let heights = (0..CHUNK_GRID_LEN).map(|i| (i % 1000) as u16).collect();
        let indices = (0..CHUNK_GRID_LEN).map(|i| (i % 3) as u16).collect();
        Heightmap::new(heights, table, indices).unwrap()
    }

    // Independent encoder mirroring the documented layout, used to craft malformed inputs.
    fn raw_file(
        magic: [u8; 4],
        version: u16,
        table: &[&[u8]],
        heights: &[u16],
        indices: &[u16],
    ) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&magic);
        b.extend_from_slice(&version.to_le_bytes());
        b.extend_from_slice(&(table.len() as u32).to_le_bytes());
        for t in table {
            b.extend_from_slice(&(t.len() as u32).to_le_bytes());
            b.extend_from_slice(t);
        }
        for &h in heights {
            b.extend_from_slice(&h.to_le_bytes());
        }
        for &i in indices {
            b.extend_from_slice(&i.to_le_bytes());
        }
        b
    }

    #[test]
    fn round_trip_is_byte_identical() {
        let hm = sample();
        let bytes = encode_heightmap(&hm);
        let back = decode_heightmap(&bytes, Path::new("mem")).unwrap();
        // The decoded value equals the original (table order and indices included)...
        assert_eq!(back, hm);
        // ...and re-encoding it reproduces the exact same bytes (determinism contract).
        assert_eq!(encode_heightmap(&back), bytes);
    }

    #[test]
    fn save_then_load_round_trips_through_a_file() {
        let path = unique_temp();
        let hm = sample();
        save_heightmap(&hm, &path).unwrap();
        let back = load_heightmap(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(back, hm);
    }

    #[test]
    fn load_missing_file_is_io_error() {
        let err = load_heightmap(unique_temp()).unwrap_err();
        assert!(matches!(err, LoadError::Io { .. }));
    }

    #[test]
    fn bad_magic_is_load_error() {
        let zeros = vec![0u16; CHUNK_GRID_LEN];
        let bytes = raw_file(*b"XXXX", VERSION, &[b"g"], &zeros, &zeros);
        assert!(matches!(
            decode_heightmap(&bytes, Path::new("bad")).unwrap_err(),
            LoadError::BadMagic { .. }
        ));
    }

    #[test]
    fn unsupported_version_is_load_error() {
        let zeros = vec![0u16; CHUNK_GRID_LEN];
        let bytes = raw_file(MAGIC, 99, &[b"g"], &zeros, &zeros);
        match decode_heightmap(&bytes, Path::new("v")).unwrap_err() {
            LoadError::UnsupportedVersion { version, .. } => assert_eq!(version, 99),
            other => panic!("expected UnsupportedVersion, got {other:?}"),
        }
    }

    #[test]
    fn truncated_file_is_load_error() {
        let bytes = encode_heightmap(&sample());
        // Cut off mid-grid: header and table survive, the height grid does not.
        let err = decode_heightmap(&bytes[..20], Path::new("short")).unwrap_err();
        assert!(matches!(err, LoadError::Truncated { .. }));
    }

    #[test]
    fn non_utf8_surface_tag_is_load_error() {
        let zeros = vec![0u16; CHUNK_GRID_LEN];
        // 0xFF is never a valid UTF-8 byte.
        let bytes = raw_file(MAGIC, VERSION, &[&[0xFF]], &zeros, &zeros);
        assert!(matches!(
            decode_heightmap(&bytes, Path::new("utf8")).unwrap_err(),
            LoadError::Utf8 { .. }
        ));
    }

    #[test]
    fn index_past_table_end_is_load_error() {
        let zeros = vec![0u16; CHUNK_GRID_LEN];
        let mut indices = zeros.clone();
        indices[0] = 1; // table has one entry (valid index 0 only)
        let bytes = raw_file(MAGIC, VERSION, &[b"g"], &zeros, &indices);
        match decode_heightmap(&bytes, Path::new("idx")).unwrap_err() {
            LoadError::Heightmap { source, .. } => {
                let msg = source.to_string();
                assert!(msg.contains("past the end"), "got {msg:?}");
            }
            other => panic!("expected Heightmap error, got {other:?}"),
        }
    }

    #[test]
    fn index_into_empty_table_is_load_error() {
        // A zero-length table must decode cleanly (the table loop runs zero times), but then no
        // cell index can be valid - index 0 has nothing to point at. This checks both: the
        // empty-table header is read without choking, and validation still rejects the indices.
        let zeros = vec![0u16; CHUNK_GRID_LEN];
        let bytes = raw_file(MAGIC, VERSION, &[], &zeros, &zeros);
        assert!(matches!(
            decode_heightmap(&bytes, Path::new("empty")).unwrap_err(),
            LoadError::Heightmap { .. }
        ));
    }

    #[test]
    fn magic_is_the_documented_bytes() {
        assert_eq!(&MAGIC, b"WKHM");
        assert_eq!(CHUNK_GRID_LEN, CHUNK_GRID_DIM * CHUNK_GRID_DIM);
    }
}
