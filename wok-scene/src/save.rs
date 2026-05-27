use std::path::Path;

use pantry::serde::Serialize;
use pantry::serde_json;

use crate::authored::{Chunk, Prefab, Scene};
use crate::error::SaveError;
use crate::serde_format::{CURRENT_FORMAT, Versioned};

/// Save a prefab to a JSON file at `path`.
pub fn save_prefab(prefab: &Prefab, path: &Path) -> Result<(), SaveError> {
    save_versioned(prefab, path)
}

/// Save the scene manifest as `scene_dir/scene.json`.
pub fn save_scene_manifest(scene: &Scene, scene_dir: &Path) -> Result<(), SaveError> {
    save_versioned(scene, &scene_dir.join("scene.json"))
}

/// Save a chunk as `scene_dir/{chunk.coord.x}_{chunk.coord.z}.json`. If `chunk.terrain` is
/// `Some`, the sibling binary is written FIRST at `scene_dir/{terrain.heightmap_file}` and
/// then the JSON; the ordering ensures a successful JSON write always references a binary
/// that exists on disk. A crash between the two leaves an updated binary referenced by an
/// old or absent JSON, which the next save overwrites cleanly (plan section 9, "Atomic save
/// crash recovery").
pub fn save_chunk(scene_dir: &Path, chunk: &Chunk) -> Result<(), SaveError> {
    if let Some(terrain) = chunk.terrain.as_ref() {
        let sibling_path = scene_dir.join(&terrain.heightmap_file);
        crate::authored::terrain::write_sibling(terrain, &sibling_path)?;
    }
    save_versioned(
        chunk,
        &scene_dir.join(format!("{}_{}.json", chunk.coord.x, chunk.coord.z)),
    )
}

/// Wrap `value` with the current `_format` header and write the resulting JSON to `path`.
/// The parent directory must already exist; save does not create it.
fn save_versioned<T: Serialize>(value: &T, path: &Path) -> Result<(), SaveError> {
    let wrapped = Versioned {
        format: CURRENT_FORMAT,
        inner: value,
    };
    let json =
        serde_json::to_string_pretty(&wrapped).map_err(|source| SaveError::Encode { source })?;
    std::fs::write(path, json).map_err(|source| SaveError::Io {
        path: path.to_owned(),
        source,
    })
}
