//! The editor's content-directory layout, loading, and hot-reload classification.
//!
//! The engine fixes naming conventions in a content-conventions surface that is not built yet
//! (HLD, "content conventions and integrity"), so v0 picks the minimal layout and owns it as
//! caller policy until that surface lands:
//!
//! - `<root>/scene.json` - the manifest, with chunk files as flat siblings (wok-scene's documented
//!   convention: the chunk files are the source of truth for which chunks exist).
//! - `<root>/{x}_{z}.json` and `<root>/{x}_{z}.heightmap.bin` - one chunk and its sibling terrain.
//! - `<root>/prefabs/<slug>.json` - prefabs, named by file stem.
//! - `<root>/lighting/<name>.json` - light states, named by file stem (wok-light's convention).
//!
//! Subdirectories per kind keep hot-reload classification mechanical: a changed path maps to a
//! [`Reload`] by where it sits, not by parsing its contents.
//!
//! Errors are `Box<dyn Error>`: the editor's startup path only needs "did it work, and what is the
//! message", never to distinguish failure modes programmatically, so per canon no error enum has
//! earned its place (and `anyhow` would be a new dependency solving the same non-problem).

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

    pub fn prefab(&self, name: &str) -> PathBuf {
        self.prefab_dir().join(format!("{name}.json"))
    }

    pub fn lighting_dir(&self) -> PathBuf {
        self.root.join("lighting")
    }

    pub fn light(&self, name: &str) -> PathBuf {
        self.lighting_dir().join(format!("{name}.json"))
    }
}

/// Parse a chunk-file stem (`"{x}_{z}"`, e.g. `"0_0"` or `"-3_12"`) into its coordinate.
pub fn chunk_coord_from_stem(stem: &str) -> Option<ChunkCoord> {
    let (x, z) = stem.split_once('_')?;
    Some(ChunkCoord::new(x.parse().ok()?, z.parse().ok()?))
}

/// What a changed file means for the running editor. Produced by [`classify`] from a watcher path;
/// the editor applies these, it never re-derives meaning from paths anywhere else.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reload {
    /// `scene.json` changed: re-read the manifest (name, default lighting).
    Scene,
    /// A chunk's JSON or heightmap changed: release and re-transform that chunk.
    Chunk(ChunkCoord),
    /// A prefab changed: re-read the library and re-transform every loaded chunk (any chunk may
    /// place it; the runtime arrays do not retain which prefabs a chunk used).
    Prefabs,
    /// A light state changed: re-read that state. Per-frame data; never triggers a chunk reload.
    Light(String),
}

/// Map a changed path (as reported by wok-scene's watcher) to its reload action, or `None` for
/// paths the editor does not care about (directories, foreign files, files outside the root).
pub fn classify(root: &Path, changed: &Path) -> Option<Reload> {
    let relative = changed.strip_prefix(root).ok()?;
    let file_name = relative.file_name()?.to_str()?;

    let mut components = relative.components();
    let first = components.next()?.as_os_str().to_str()?;
    let in_subdir = components.next().is_some();

    if in_subdir {
        let stem = file_name.strip_suffix(".json")?;
        return match first {
            "prefabs" => Some(Reload::Prefabs),
            "lighting" => Some(Reload::Light(stem.to_string())),
            _ => None,
        };
    }

    if file_name == "scene.json" {
        return Some(Reload::Scene);
    }
    if let Some(stem) = file_name.strip_suffix(".json") {
        return chunk_coord_from_stem(stem).map(Reload::Chunk);
    }
    if let Some(stem) = file_name.strip_suffix(".heightmap.bin") {
        return chunk_coord_from_stem(stem).map(Reload::Chunk);
    }
    None
}

/// Everything the editor loads from disk at startup: the authored forms plus the resolved light
/// state. Chunks are paired with their optional heightmaps, sorted by coordinate so downstream
/// work (store loads, GPU uploads) happens in a deterministic order.
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
pub fn scan_chunk_coords(root: &Path) -> Result<Vec<ChunkCoord>, Box<dyn Error>> {
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
pub fn load_chunk_with_heightmap(
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
pub fn load_prefab_library(paths: &ContentPaths) -> Result<HashMap<PrefabRef, Prefab>, Box<dyn Error>> {
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

    // ---- chunk_coord_from_stem ----

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

    // ---- classify ----

    fn root() -> PathBuf {
        PathBuf::from("content")
    }

    #[test]
    fn scene_json_classifies_as_scene() {
        assert_eq!(classify(&root(), &root().join("scene.json")), Some(Reload::Scene));
    }

    #[test]
    fn chunk_json_and_heightmap_classify_as_the_same_chunk() {
        let coord = ChunkCoord::new(-1, 4);
        assert_eq!(classify(&root(), &root().join("-1_4.json")), Some(Reload::Chunk(coord)));
        assert_eq!(classify(&root(), &root().join("-1_4.heightmap.bin")), Some(Reload::Chunk(coord)));
    }

    #[test]
    fn prefab_json_classifies_as_prefabs() {
        let path = root().join("prefabs").join("crate.json");
        assert_eq!(classify(&root(), &path), Some(Reload::Prefabs));
    }

    #[test]
    fn light_json_classifies_with_its_name() {
        let path = root().join("lighting").join("default.json");
        assert_eq!(classify(&root(), &path), Some(Reload::Light("default".to_string())));
    }

    #[test]
    fn foreign_and_outside_paths_classify_as_none() {
        assert_eq!(classify(&root(), &root().join("notes.txt")), None);
        assert_eq!(classify(&root(), &root().join("prefabs")), None);
        assert_eq!(classify(&root(), &root().join("textures").join("a.json")), None);
        assert_eq!(classify(&root(), Path::new("elsewhere/scene.json")), None);
    }

    // ---- ContentPaths ----

    #[test]
    fn paths_follow_the_documented_layout() {
        let paths = ContentPaths::new(PathBuf::from("c"));
        let coord = ChunkCoord::new(2, -7);
        assert_eq!(paths.scene(), PathBuf::from("c").join("scene.json"));
        assert_eq!(paths.chunk(coord), PathBuf::from("c").join("2_-7.json"));
        assert_eq!(paths.heightmap(coord), PathBuf::from("c").join("2_-7.heightmap.bin"));
        assert_eq!(paths.prefab("crate"), PathBuf::from("c").join("prefabs").join("crate.json"));
        assert_eq!(paths.light("noon"), PathBuf::from("c").join("lighting").join("noon.json"));
    }

    #[test]
    fn classify_round_trips_the_layout_paths() {
        // The layout writer (ContentPaths) and the layout reader (classify) must agree; this is
        // the structural tie between them.
        let paths = ContentPaths::new(root());
        let coord = ChunkCoord::new(0, 0);
        assert_eq!(classify(&paths.root, &paths.scene()), Some(Reload::Scene));
        assert_eq!(classify(&paths.root, &paths.chunk(coord)), Some(Reload::Chunk(coord)));
        assert_eq!(classify(&paths.root, &paths.heightmap(coord)), Some(Reload::Chunk(coord)));
        assert_eq!(classify(&paths.root, &paths.prefab("crate")), Some(Reload::Prefabs));
        assert_eq!(
            classify(&paths.root, &paths.light("default")),
            Some(Reload::Light("default".to_string()))
        );
    }
}
