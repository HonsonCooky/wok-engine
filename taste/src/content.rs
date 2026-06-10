//! Read-only loading of an editor-authored content directory.
//!
//! taste consumes the layout the wok editor writes; until the engine's content-conventions surface
//! exists (HLD, "content conventions and integrity") that layout is caller policy, restated here on
//! the consuming side:
//!
//! - `<root>/scene.json` - the manifest; chunk files are flat siblings and are the source of truth
//!   for which chunks exist.
//! - `<root>/{x}_{z}.json` and `<root>/{x}_{z}.heightmap.bin` - one chunk and its sibling terrain.
//! - `<root>/prefabs/<slug>.json` - prefabs, named by file stem.
//! - `<root>/lighting/<name>.json` - light states, named by file stem (wok-light's convention).
//!
//! taste never writes content and never watches it: the editor authors, the game plays what is on
//! disk at startup. Errors are `Box<dyn Error>` per the wok precedent - the startup path only needs
//! "did it work, and what is the message", never to distinguish failure modes programmatically.

use std::collections::HashMap;
use std::error::Error;
use std::path::{Path, PathBuf};

use wok_light::LightState;
use wok_scene::{Chunk, ChunkCoord, Heightmap, Prefab, PrefabRef, Scene};

/// Path computation for one content root. All layout knowledge lives here.
pub struct ContentPaths {
    pub root: PathBuf,
}

impl ContentPaths {
    pub fn new(root: PathBuf) -> Self {
        ContentPaths { root }
    }

    pub fn scene(&self) -> PathBuf {
        self.root.join("scene.json")
    }

    pub fn chunk(&self, coord: ChunkCoord) -> PathBuf {
        self.root.join(format!("{}_{}.json", coord.x, coord.z))
    }

    pub fn heightmap(&self, coord: ChunkCoord) -> PathBuf {
        self.root.join(format!("{}_{}.heightmap.bin", coord.x, coord.z))
    }

    pub fn prefab_dir(&self) -> PathBuf {
        self.root.join("prefabs")
    }

    pub fn light(&self, name: &str) -> PathBuf {
        self.root.join("lighting").join(format!("{name}.json"))
    }
}

/// Parse a chunk-file stem (`"{x}_{z}"`, e.g. `"0_0"` or `"-3_12"`) into its coordinate.
pub fn chunk_coord_from_stem(stem: &str) -> Option<ChunkCoord> {
    let (x, z) = stem.split_once('_')?;
    Some(ChunkCoord::new(x.parse().ok()?, z.parse().ok()?))
}

/// Everything taste loads from disk at startup: the authored forms plus the resolved light state.
/// Chunks are paired with their optional heightmaps, sorted by coordinate so downstream work (store
/// loads, GPU uploads) happens in a deterministic order.
pub struct LoadedContent {
    pub scene: Scene,
    pub prefabs: HashMap<PrefabRef, Prefab>,
    pub chunks: Vec<(Chunk, Option<Heightmap>)>,
    pub light: LightState,
}

/// Load the whole content directory: manifest, every chunk with its terrain, the prefab library,
/// and the scene's default light state.
pub fn load_all(paths: &ContentPaths) -> Result<LoadedContent, Box<dyn Error>> {
    let scene = wok_scene::load_scene(paths.scene())?;

    let mut coords = scan_chunk_coords(&paths.root)?;
    coords.sort_unstable();
    let mut chunks = Vec::with_capacity(coords.len());
    for coord in coords {
        chunks.push(load_chunk_with_heightmap(paths, coord)?);
    }

    let prefabs = load_prefab_library(paths)?;
    let (_, light) = wok_light::load_light_state(paths.light(scene.default_lighting.as_str()))?;

    Ok(LoadedContent { scene, prefabs, chunks, light })
}

/// The chunk coordinates present on disk, read from the flat `{x}_{z}.json` file names.
fn scan_chunk_coords(root: &Path) -> Result<Vec<ChunkCoord>, Box<dyn Error>> {
    let mut coords = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let path = entry?.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        if let Some(stem) = name.strip_suffix(".json")
            && let Some(coord) = chunk_coord_from_stem(stem)
        {
            coords.push(coord);
        }
    }
    Ok(coords)
}

/// Load one chunk and its sibling heightmap; a missing heightmap file is a chunk without terrain,
/// not an error.
fn load_chunk_with_heightmap(
    paths: &ContentPaths,
    coord: ChunkCoord,
) -> Result<(Chunk, Option<Heightmap>), Box<dyn Error>> {
    let chunk = wok_scene::load_chunk(paths.chunk(coord))?;
    let heightmap_path = paths.heightmap(coord);
    let heightmap = if heightmap_path.exists() {
        Some(wok_scene::load_heightmap(heightmap_path)?)
    } else {
        None
    };
    Ok((chunk, heightmap))
}

/// Load every prefab under `prefabs/`, keyed by file-stem slug.
fn load_prefab_library(paths: &ContentPaths) -> Result<HashMap<PrefabRef, Prefab>, Box<dyn Error>> {
    let mut prefabs = HashMap::new();
    for entry in std::fs::read_dir(paths.prefab_dir())? {
        let path = entry?.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        if let Some(stem) = name.strip_suffix(".json") {
            prefabs.insert(PrefabRef::new(stem), wok_scene::load_prefab(&path)?);
        }
    }
    Ok(prefabs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stem_parses_positive_and_negative_coordinates() {
        assert_eq!(chunk_coord_from_stem("0_0"), Some(ChunkCoord::new(0, 0)));
        assert_eq!(chunk_coord_from_stem("-3_12"), Some(ChunkCoord::new(-3, 12)));
    }

    #[test]
    fn stem_rejects_non_coordinate_names() {
        assert_eq!(chunk_coord_from_stem("scene"), None);
        assert_eq!(chunk_coord_from_stem("oak_tree"), None);
        assert_eq!(chunk_coord_from_stem("1_2_3"), None);
        assert_eq!(chunk_coord_from_stem(""), None);
    }

    #[test]
    fn paths_follow_the_editor_layout() {
        // The structural tie to the layout the wok editor writes: if either side changes shape,
        // this is the test that names the disagreement.
        let paths = ContentPaths::new(PathBuf::from("c"));
        let coord = ChunkCoord::new(2, -7);
        assert_eq!(paths.scene(), PathBuf::from("c").join("scene.json"));
        assert_eq!(paths.chunk(coord), PathBuf::from("c").join("2_-7.json"));
        assert_eq!(paths.heightmap(coord), PathBuf::from("c").join("2_-7.heightmap.bin"));
        assert_eq!(paths.light("noon"), PathBuf::from("c").join("lighting").join("noon.json"));
    }
}
