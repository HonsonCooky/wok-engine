//! The project content layout: where authored content lives on disk and how a tool finds it.
//!
//! A wok project is a folder. Its authored content lives under an `assets/` subfolder with a fixed,
//! opinionated shape - the convention the engine owns so references resolve by name with no registry
//! and any tool scans the same tree (HLD, "content conventions and integrity"):
//!
//! ```text
//! <project root>/assets/
//!   scenes/<scene>/   scene.json + {x}_{z}.json + {x}_{z}.heightmap.bin   (one folder per scene)
//!   prefabs/<slug>.json    (project-shared)
//!   lighting/<name>.json   (project-shared; scene-owned lighting still parked)
//! ```
//!
//! Two halves, both free of policy:
//!
//! - Path resolution ([`ContentLayout`] methods returning a `PathBuf`): no I/O, just the convention
//!   applied to a name or a chunk coordinate. The `{x}_{z}` chunk stem matches the file-level loaders
//!   in [`crate::io`] and the heightmap sibling in [`crate::heightmap_io`], so a path this surface
//!   computes is one those functions load.
//! - Discovery (the `*_names` / `*_slugs` scans, plus `chunk_coords` within a scene): a tolerant
//!   `read_dir`. A missing `assets/` or subdirectory is an empty list, never an error - a folder
//!   with no content is simply an empty project. Results are sorted, so a scan is deterministic
//!   regardless of directory order.
//!
//! This surface computes paths and lists what is present. It does not open a project, create the tree
//! (that happens lazily on first save, a later bite), pick a default scene (there is none; a scene is
//! opened explicitly), or check references (the integrity scan is deferred). There is no validation
//! gate: opening is a read-only scan, so there is no "not a project" failure mode here.

use std::path::{Path, PathBuf};

use crate::chunk::ChunkCoord;

// The fixed folder and file names under a project root. Single-sourced here so the path methods and
// the discovery scans below cannot drift from one another or from the documented convention.
const ASSETS_DIR: &str = "assets";
const SCENES_DIR: &str = "scenes";
const PREFABS_DIR: &str = "prefabs";
const LIGHTING_DIR: &str = "lighting";
const SCENE_FILE: &str = "scene.json";

/// The content layout rooted at one project folder. Build it once with the project root, then ask it
/// for paths or for what is on disk; it holds only the root and owns all layout knowledge.
#[derive(Clone, Debug)]
pub struct ContentLayout {
    root: PathBuf,
}

impl ContentLayout {
    /// Wrap a project root. The root need not exist: the path methods are pure, and the discovery
    /// scans treat a missing tree as an empty project.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        ContentLayout { root: root.into() }
    }

    /// The project root this layout is rooted at.
    pub fn root(&self) -> &Path {
        &self.root
    }

    // ---- path resolution (pure; no I/O) ----

    /// `<root>/assets` - the content folder all authored data lives under.
    pub fn assets_dir(&self) -> PathBuf {
        self.root.join(ASSETS_DIR)
    }

    /// `<root>/assets/scenes` - the folder holding one subfolder per scene.
    pub fn scenes_dir(&self) -> PathBuf {
        self.assets_dir().join(SCENES_DIR)
    }

    /// `<root>/assets/scenes/<scene>` - one scene's self-contained folder.
    pub fn scene_dir(&self, scene: &str) -> PathBuf {
        self.scenes_dir().join(scene)
    }

    /// `<root>/assets/scenes/<scene>/scene.json` - a scene's manifest file.
    pub fn scene_json(&self, scene: &str) -> PathBuf {
        self.scene_dir(scene).join(SCENE_FILE)
    }

    /// `<root>/assets/scenes/<scene>/{x}_{z}.json` - one chunk file within a scene.
    pub fn chunk(&self, scene: &str, coord: ChunkCoord) -> PathBuf {
        self.scene_dir(scene).join(format!("{}_{}.json", coord.x, coord.z))
    }

    /// `<root>/assets/scenes/<scene>/{x}_{z}.heightmap.bin` - a chunk's sibling terrain.
    pub fn heightmap(&self, scene: &str, coord: ChunkCoord) -> PathBuf {
        self.scene_dir(scene).join(format!("{}_{}.heightmap.bin", coord.x, coord.z))
    }

    /// `<root>/assets/prefabs` - the project-shared prefab folder.
    pub fn prefabs_dir(&self) -> PathBuf {
        self.assets_dir().join(PREFABS_DIR)
    }

    /// `<root>/assets/prefabs/<slug>.json` - one prefab, keyed by slug.
    pub fn prefab(&self, slug: &str) -> PathBuf {
        self.prefabs_dir().join(format!("{slug}.json"))
    }

    /// `<root>/assets/lighting` - the project-shared lighting folder.
    pub fn lighting_dir(&self) -> PathBuf {
        self.assets_dir().join(LIGHTING_DIR)
    }

    /// `<root>/assets/lighting/<name>.json` - one light state, keyed by name.
    pub fn lighting(&self, name: &str) -> PathBuf {
        self.lighting_dir().join(format!("{name}.json"))
    }

    // ---- discovery (tolerant scan; a missing dir is an empty list, never an error) ----

    /// The scenes present on disk: the names of the subfolders under `assets/scenes`, sorted.
    pub fn scene_names(&self) -> Vec<String> {
        subdir_names(&self.scenes_dir())
    }

    /// The prefab slugs present on disk: the `.json` file stems under `assets/prefabs`, sorted.
    pub fn prefab_slugs(&self) -> Vec<String> {
        json_stems(&self.prefabs_dir())
    }

    /// The light-state names present on disk: the `.json` file stems under `assets/lighting`, sorted.
    pub fn lighting_names(&self) -> Vec<String> {
        json_stems(&self.lighting_dir())
    }

    /// The chunk coordinates present in one scene: every `{x}_{z}.json` file name under
    /// `assets/scenes/<scene>`, parsed back to a [`ChunkCoord`] and sorted (the deterministic load
    /// order downstream relies on). This is the inverse of [`chunk`](Self::chunk) - the same
    /// `{x}_{z}` stem, read rather than written. The manifest (`scene.json`), the sibling
    /// `{x}_{z}.heightmap.bin` terrain, and any other non-`{x}_{z}.json` entry are skipped; a
    /// missing scene folder yields an empty list, never an error.
    pub fn chunk_coords(&self, scene: &str) -> Vec<ChunkCoord> {
        let Ok(entries) = std::fs::read_dir(self.scene_dir(scene)) else {
            return Vec::new();
        };
        let mut coords: Vec<ChunkCoord> = entries
            .filter_map(Result::ok)
            .filter_map(|e| {
                let path = e.path();
                if !path.is_file() {
                    return None;
                }
                let stem = path.file_name().and_then(|n| n.to_str())?.strip_suffix(".json")?;
                chunk_coord_from_stem(stem)
            })
            .collect();
        coords.sort_unstable();
        coords
    }
}

/// Sorted names of the immediate subdirectories of `dir`. A missing or unreadable directory, or an
/// entry that cannot be read, contributes nothing: the result is an empty list, not an error.
fn subdir_names(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    names.sort_unstable();
    names
}

/// Sorted `.json` file stems directly under `dir` - the slug or name each file is keyed by. Non-`.json`
/// files and subdirectories are ignored; a missing or unreadable directory yields an empty list.
fn json_stems(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut stems: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let path = e.path();
            if !path.is_file() {
                return None;
            }
            let name = path.file_name().and_then(|n| n.to_str())?;
            name.strip_suffix(".json").map(str::to_owned)
        })
        .collect();
    stems.sort_unstable();
    stems
}

/// Parse a chunk-file stem (`"{x}_{z}"`, e.g. `"0_0"` or `"-3_12"`) into its coordinate. `None` for
/// anything that is not two `_`-separated integers, so the manifest stem (`scene`) and prefab-style
/// names never read as chunks. The inverse of the `{x}_{z}` stem [`ContentLayout::chunk`] writes.
fn chunk_coord_from_stem(stem: &str) -> Option<ChunkCoord> {
    let (x, z) = stem.split_once('_')?;
    Some(ChunkCoord::new(x.parse().ok()?, z.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    // ---- path resolution (pure) ----

    #[test]
    fn paths_follow_the_assets_convention() {
        // These joins ARE the on-disk convention: taste's loader and the editor's open both resolve
        // content through this surface, so if the shape ever changes this is the test that names the
        // disagreement. Expected paths are built by `join` so the separator is correct per platform.
        let layout = ContentLayout::new("proj");
        let assets = Path::new("proj").join("assets");
        assert_eq!(layout.assets_dir(), assets);
        assert_eq!(layout.scenes_dir(), assets.join("scenes"));
        assert_eq!(layout.scene_dir("village"), assets.join("scenes").join("village"));
        assert_eq!(layout.scene_json("village"), assets.join("scenes").join("village").join("scene.json"));

        // The {x}_{z} stem (negative coords included) matches crate::io / crate::heightmap_io.
        let village = assets.join("scenes").join("village");
        let coord = ChunkCoord::new(2, -7);
        assert_eq!(layout.chunk("village", coord), village.join("2_-7.json"));
        assert_eq!(layout.heightmap("village", coord), village.join("2_-7.heightmap.bin"));

        assert_eq!(layout.prefabs_dir(), assets.join("prefabs"));
        assert_eq!(layout.prefab("oak_tree"), assets.join("prefabs").join("oak_tree.json"));
        assert_eq!(layout.lighting_dir(), assets.join("lighting"));
        assert_eq!(layout.lighting("noon"), assets.join("lighting").join("noon.json"));
    }

    #[test]
    fn root_is_returned_unchanged() {
        let layout = ContentLayout::new("some/proj/root");
        assert_eq!(layout.root(), Path::new("some/proj/root"));
    }

    // ---- discovery (filesystem) ----

    // A unique temp directory per test, on wok-scene's existing pattern (pid + atomic counter, no
    // wall-clock). The directory is not created here; each test seeds and cleans up what it needs.
    fn unique_temp_dir() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let pid = std::process::id();
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("wok-scene-layout-{pid}-{n}"))
    }

    // Seed a full assets/ tree: two scenes (folders, each with a scene.json), two prefabs, two light
    // states, plus stray entries the scans must ignore (a loose file under scenes/, a non-json under
    // prefabs/). Returns nothing; the caller already holds the layout and the root for cleanup.
    fn seed_assets(layout: &ContentLayout) {
        for scene in ["village", "dungeon"] {
            std::fs::create_dir_all(layout.scene_dir(scene)).unwrap();
            std::fs::write(layout.scene_json(scene), b"{}").unwrap();
        }
        std::fs::write(layout.scenes_dir().join("notes.txt"), b"not a scene").unwrap();

        std::fs::create_dir_all(layout.prefabs_dir()).unwrap();
        std::fs::write(layout.prefab("oak_tree"), b"{}").unwrap();
        std::fs::write(layout.prefab("barrel"), b"{}").unwrap();
        std::fs::write(layout.prefabs_dir().join("README.md"), b"not a prefab").unwrap();

        std::fs::create_dir_all(layout.lighting_dir()).unwrap();
        std::fs::write(layout.lighting("noon"), b"{}").unwrap();
        std::fs::write(layout.lighting("dawn"), b"{}").unwrap();
    }

    #[test]
    fn discovery_lists_sorted_names_and_ignores_strays() {
        let root = unique_temp_dir();
        let _ = std::fs::remove_dir_all(&root); // hermetic: clear any leftover from a crashed run
        let layout = ContentLayout::new(&root);
        seed_assets(&layout);

        // Sorted by name, and the loose file / non-json never appear.
        assert_eq!(layout.scene_names(), vec!["dungeon", "village"]);
        assert_eq!(layout.prefab_slugs(), vec!["barrel", "oak_tree"]);
        assert_eq!(layout.lighting_names(), vec!["dawn", "noon"]);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_missing_assets_dir_scans_to_empty_lists() {
        // The empty-project case: the root has no assets/ at all. No gate, no error, just nothing.
        let root = unique_temp_dir();
        let _ = std::fs::remove_dir_all(&root);
        let layout = ContentLayout::new(&root);
        assert!(layout.scene_names().is_empty());
        assert!(layout.prefab_slugs().is_empty());
        assert!(layout.lighting_names().is_empty());
    }

    #[test]
    fn an_empty_assets_dir_scans_to_empty_lists() {
        // assets/ exists but the per-kind subfolders do not: still empty, still no error.
        let root = unique_temp_dir();
        let _ = std::fs::remove_dir_all(&root);
        let layout = ContentLayout::new(&root);
        std::fs::create_dir_all(layout.assets_dir()).unwrap();
        assert!(layout.scene_names().is_empty());
        assert!(layout.prefab_slugs().is_empty());
        assert!(layout.lighting_names().is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    // ---- chunk discovery within a scene ----

    #[test]
    fn chunk_discovery_lists_sorted_coords_and_ignores_non_chunks() {
        let root = unique_temp_dir();
        let _ = std::fs::remove_dir_all(&root);
        let layout = ContentLayout::new(&root);

        // One scene folder seeded with chunk files (written out of sort order), the manifest, a
        // sibling heightmap per chunk, and a stray non-chunk `.json`. Only the `{x}_{z}.json`
        // files are chunks; everything else is skipped.
        let scene = "village";
        std::fs::create_dir_all(layout.scene_dir(scene)).unwrap();
        std::fs::write(layout.scene_json(scene), b"{}").unwrap();
        for coord in [ChunkCoord::new(2, -7), ChunkCoord::new(0, 0), ChunkCoord::new(-3, 12)] {
            std::fs::write(layout.chunk(scene, coord), b"{}").unwrap();
            std::fs::write(layout.heightmap(scene, coord), b"").unwrap();
        }
        std::fs::write(layout.scene_dir(scene).join("notes.json"), b"{}").unwrap();

        assert_eq!(
            layout.chunk_coords(scene),
            vec![ChunkCoord::new(-3, 12), ChunkCoord::new(0, 0), ChunkCoord::new(2, -7)]
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn chunk_discovery_of_a_missing_scene_is_empty() {
        // Same tolerance as the name scans: no folder, no error, just an empty list.
        let root = unique_temp_dir();
        let _ = std::fs::remove_dir_all(&root);
        let layout = ContentLayout::new(&root);
        assert!(layout.chunk_coords("nope").is_empty());
    }

    #[test]
    fn stem_parses_positive_and_negative_coordinates() {
        assert_eq!(chunk_coord_from_stem("0_0"), Some(ChunkCoord::new(0, 0)));
        assert_eq!(chunk_coord_from_stem("-3_12"), Some(ChunkCoord::new(-3, 12)));
    }

    #[test]
    fn stem_rejects_non_coordinate_names() {
        // The manifest stem and prefab-style names must never read as chunks.
        assert_eq!(chunk_coord_from_stem("scene"), None);
        assert_eq!(chunk_coord_from_stem("oak_tree"), None);
        assert_eq!(chunk_coord_from_stem("1_2_3"), None);
        assert_eq!(chunk_coord_from_stem(""), None);
    }
}
