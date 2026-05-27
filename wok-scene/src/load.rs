use std::collections::HashMap;
use std::path::{Path, PathBuf};

use pantry::serde::de::DeserializeOwned;
use pantry::serde_json;

use crate::authored::{Chunk, Prefab, Scene};
use crate::error::LoadError;
use crate::ids::{ChunkCoord, PrefabId};
use crate::serde_format::CURRENT_FORMAT;

/// Load a prefab from a single JSON file.
pub fn load_prefab(path: &Path) -> Result<Prefab, LoadError> {
    load_versioned::<Prefab>(path)
}

/// Load every `.json` file in `dir` as a prefab, keyed by `Prefab.id`. Non-`.json` entries
/// are ignored. Subdirectories are not traversed. Files are loaded in sorted-path order so
/// the result is deterministic; the `HashMap` itself is order-free, but any side effects
/// (logging, fail-fast on error) happen in stable order.
pub fn load_prefab_dir(dir: &Path) -> Result<HashMap<PrefabId, Prefab>, LoadError> {
    let entries = std::fs::read_dir(dir).map_err(|source| LoadError::Io {
        path: dir.to_owned(),
        source,
    })?;
    let mut paths: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| LoadError::Io {
            path: dir.to_owned(),
            source,
        })?;
        let path = entry.path();
        if path.extension().and_then(std::ffi::OsStr::to_str) == Some("json") {
            paths.push(path);
        }
    }
    paths.sort();

    let mut prefabs = HashMap::with_capacity(paths.len());
    for path in paths {
        let prefab = load_prefab(&path)?;
        prefabs.insert(prefab.id.clone(), prefab);
    }
    Ok(prefabs)
}

/// Load the scene manifest at `scene_dir/scene.json`.
pub fn load_scene_manifest(scene_dir: &Path) -> Result<Scene, LoadError> {
    load_versioned::<Scene>(&scene_dir.join("scene.json"))
}

/// Load the chunk file at `scene_dir/{coord.x}_{coord.z}.json`. If the chunk references a
/// sibling terrain heightmap, read it from `scene_dir/{chunk.terrain.heightmap_file}` and
/// populate the in-memory `TerrainData` fields. A missing sibling is reported as
/// `LoadError::TerrainSiblingMissing`; a malformed sibling is reported as
/// `LoadError::TerrainMalformed`.
pub fn load_chunk(scene_dir: &Path, coord: ChunkCoord) -> Result<Chunk, LoadError> {
    let chunk_path = scene_dir.join(format!("{}_{}.json", coord.x, coord.z));
    let mut chunk: Chunk = load_versioned(&chunk_path)?;
    if let Some(terrain) = chunk.terrain.as_mut() {
        let sibling_path = scene_dir.join(&terrain.heightmap_file);
        crate::authored::terrain::read_sibling_into(terrain, &chunk_path, &sibling_path)?;
    }
    Ok(chunk)
}

/// Read a file, validate its `_format`, strip the field, and deserialize the remainder as
/// `T`. See `serde_format.rs` for the reason we strip rather than flatten on the load side.
fn load_versioned<T: DeserializeOwned>(path: &Path) -> Result<T, LoadError> {
    let contents = std::fs::read_to_string(path).map_err(|source| LoadError::Io {
        path: path.to_owned(),
        source,
    })?;

    // Parse to a generic Value first so we can extract and validate `_format` before
    // handing the rest of the data to T's deserializer (which has `deny_unknown_fields`
    // and would reject `_format` if it stayed in the map).
    let mut value: serde_json::Value =
        serde_json::from_str(&contents).map_err(|source| LoadError::Parse {
            path: path.to_owned(),
            source,
        })?;

    let format_u64 = value
        .get("_format")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| LoadError::MissingFormat {
            path: path.to_owned(),
        })?;
    if format_u64 != u64::from(CURRENT_FORMAT) {
        return Err(LoadError::UnsupportedVersion {
            path: path.to_owned(),
            found: format_u64 as u32,
        });
    }

    if let serde_json::Value::Object(map) = &mut value {
        map.remove("_format");
    }

    serde_json::from_value(value).map_err(|source| LoadError::Parse {
        path: path.to_owned(),
        source,
    })
}
