//! The editor's loaded scene: the per-project residency the viewport draws, reconciled to the open
//! project and dropped when it closes.
//!
//! Composition only, per the HLD's application layer. [`LoadedScene::open`] loads a project's
//! content through `crate::content` (generating the starter scene into a fresh or empty folder, and
//! refusing a non-empty folder that is not a wok project, so opening never scatters sample files into
//! an unrelated directory), transforms every chunk into a [`ChunkStore`] (the authored-to-runtime
//! data-flow transition), and starts a `wok-scene` watcher on the root. Data
//! flows authored -> runtime only; nothing here writes back to disk (save returns with the editing
//! brief). The authored chunks are not retained: a hot reload re-reads the changed files from disk,
//! which is correct while there are no in-memory edits to preserve.
//!
//! Chunk-origin composition (runtime arrays are chunk-local; the viewport draws in world space) is
//! application policy the engine leaves to the caller, so [`chunk_origin`] and [`scene_bounds`] live
//! here - the same constants taste and the renderer derive.

use std::collections::{BTreeSet, HashMap};
use std::error::Error;
use std::path::{Path, PathBuf};

use glam::{Mat4, Vec3};
use wok_content::{ChunkState, ChunkStore};
use wok_light::LightState;
use wok_scene::{Aabb, CHUNK_GRID_DIM, ChunkCoord, Prefab, PrefabRef, Scene, VisibleItem, Watcher};

use crate::camera::FlyCamera;
use crate::content::{self, ContentPaths, Reload};
use crate::place::world_aabb;
use crate::sample;

/// Chunk side in metres, derived from the heightmap grid (128 one-metre cells; the 129th sample is
/// the shared edge). wok-scene deliberately does not bake the chunk size into ChunkCoord, so this
/// composition is application policy.
pub const CHUNK_SIZE_M: f32 = (CHUNK_GRID_DIM - 1) as f32;

/// World-space origin of a chunk: its grid coordinate times the chunk size.
pub fn chunk_origin(coord: ChunkCoord) -> Vec3 {
    Vec3::new(coord.x as f32 * CHUNK_SIZE_M, 0.0, coord.z as f32 * CHUNK_SIZE_M)
}

/// The open project's loaded content: the manifest, the prefab library, the runtime chunk store,
/// the active light state, and the listings the content browser shows. The watcher feeds the hot
/// reload. Created by [`open`](LoadedScene::open) and held as an `Option` on the app for as long as
/// a project is open.
pub struct LoadedScene {
    pub paths: ContentPaths,
    pub scene: Scene,
    pub prefabs: HashMap<PrefabRef, Prefab>,
    pub store: ChunkStore,
    pub light: LightState,
    /// Prefab slugs under `prefabs/`, sorted, for the content browser. Re-scanned on a prefab reload.
    pub prefab_names: Vec<String>,
    /// Light-state names under `lighting/`, sorted, for the content browser. Re-scanned on a light reload.
    pub light_names: Vec<String>,
    watcher: Watcher,
}

impl LoadedScene {
    /// Load the project at `root`. An existing `scene.json` loads; a fresh or empty folder gets the
    /// starter scene generated and loaded; a non-empty folder without `scene.json` is an error (not a
    /// wok project) and writes nothing. Then transforms every chunk into the store, scans the browser
    /// listings, and starts the hot-reload watcher.
    pub fn open(root: PathBuf) -> Result<LoadedScene, Box<dyn Error>> {
        let paths = ContentPaths::new(root);
        if !paths.scene().exists() {
            // No scene.json: generate the starter scene only into a fresh place (a path that does not
            // exist yet, or an existing empty folder). A folder that already holds content is some
            // directory picked by mistake - report it and write nothing, never scatter sample files.
            if is_fresh_project_dir(&paths.root) {
                sample::generate(&paths)?;
            } else {
                return Err(Box::new(NotAProject { root: paths.root.clone() }));
            }
        }
        let loaded = content::load_all(&paths)?;

        let mut store = ChunkStore::new();
        for (chunk, heightmap) in loaded.chunks {
            store.load(chunk, heightmap, &loaded.prefabs)?;
        }
        let prefab_names = sorted_names(loaded.prefabs.keys().map(|r| r.as_str().to_string()));
        let light_names = content::scan_light_names(&paths);
        let watcher = Watcher::new(&paths.root)?;

        Ok(LoadedScene {
            paths,
            scene: loaded.scene,
            prefabs: loaded.prefabs,
            store,
            light: loaded.light,
            prefab_names,
            light_names,
            watcher,
        })
    }

    /// Spawn the god-cam over the first loaded chunk, mid-south looking north across it, a little
    /// above the terrain there (or above the origin plane when the chunk has no terrain).
    pub fn spawn_camera(&self) -> FlyCamera {
        let half = CHUNK_SIZE_M * 0.5;
        let south = CHUNK_SIZE_M * 0.8;
        let (origin, ground) = self.store.iter_loaded().next().map_or((Vec3::ZERO, 0.0), |(coord, runtime)| {
            let ground = runtime.heightmap.as_ref().map_or(0.0, |h| h.height_at(half, south));
            (chunk_origin(coord), ground)
        });
        FlyCamera { position: origin + Vec3::new(half, ground + 12.0, south), yaw: 0.0, pitch: -0.15 }
    }

    /// Fog distance sets render distance (HLD); the far plane sits past full occlusion.
    pub fn far_plane(&self) -> f32 {
        (self.light.fog.end * 1.2).max(50.0)
    }

    /// World-space bounds of everything the loaded chunks draw - terrain plus placed visible extents
    /// - the shadow region the render pass passes (caller policy per the render contract). Falls back
    /// to a small box around the origin when nothing is loaded, so the shadow fit stays well-formed.
    /// Conservative AABBs are the right tool: a shadow region wants cover, not fit.
    pub fn scene_bounds(&self) -> Aabb {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        let mut grow = |b: Aabb| {
            min = min.min(b.min);
            max = max.max(b.max);
        };
        for (coord, runtime) in self.store.iter_loaded() {
            let origin = chunk_origin(coord);
            let origin_mat = Mat4::from_translation(origin);
            if let Some(mesh) = runtime.terrain_mesh.as_ref() {
                let b = mesh.bounds();
                grow(Aabb::new(b.min + origin, b.max + origin));
            }
            for item in &runtime.visible {
                if let VisibleItem::Primitive { primitive, transform, .. } = item {
                    grow(world_aabb(*primitive, origin_mat * *transform));
                }
            }
        }
        if min.x > max.x {
            return Aabb::new(Vec3::splat(-1.0), Vec3::splat(1.0));
        }
        Aabb::new(min, max)
    }

    /// Apply one frame's worth of watcher-reported file changes, re-reading the changed authored
    /// forms from disk and re-applying them. Returns whether the per-chunk terrain GPU meshes must be
    /// rebuilt - true when any chunk file changed (its heightmap may have), false for prefab, scene,
    /// or light changes (those alter the runtime arrays the render reads each frame, or the light
    /// state, but never the terrain meshes). Failures are printed and skipped rather than crashing: a
    /// watcher event can arrive mid-write, and the completing write delivers a second event that
    /// retries.
    pub fn poll_reload(&mut self) -> bool {
        let changed = self.watcher.poll();
        if changed.is_empty() {
            return false;
        }

        let mut scene_changed = false;
        let mut prefabs_changed = false;
        let mut light_changed = false;
        let mut chunks: BTreeSet<ChunkCoord> = BTreeSet::new();
        for path in changed {
            match content::classify(&self.paths.root, &path) {
                Some(Reload::Scene) => scene_changed = true,
                Some(Reload::Prefabs) => prefabs_changed = true,
                Some(Reload::Chunk(coord)) => {
                    chunks.insert(coord);
                }
                Some(Reload::Light(name)) if name == self.scene.default_lighting.as_str() => light_changed = true,
                _ => {}
            }
        }

        if scene_changed {
            light_changed |= self.reload_scene();
        }
        if prefabs_changed {
            self.reload_prefabs();
        }
        let terrain_dirty = !chunks.is_empty();
        for coord in chunks {
            self.reload_chunk(coord);
        }
        if light_changed {
            self.reload_light();
        }
        terrain_dirty
    }

    /// Re-read the scene manifest, adopting it when it differs. Returns whether the default lighting
    /// changed (so the caller reloads the light state).
    fn reload_scene(&mut self) -> bool {
        match wok_scene::load_scene(self.paths.scene()) {
            Ok(scene) if scene != self.scene => {
                let lighting_changed = scene.default_lighting != self.scene.default_lighting;
                self.scene = scene;
                println!("wok: reloaded scene manifest");
                lighting_changed
            }
            Ok(_) => false,
            Err(err) => {
                eprintln!("wok: scene reload failed, keeping previous: {err}");
                false
            }
        }
    }

    /// Re-read the prefab library; on a change, re-transform every loaded chunk from disk (the
    /// runtime arrays do not retain which prefabs a chunk placed, so any chunk may be affected) and
    /// re-scan the browser listing.
    fn reload_prefabs(&mut self) {
        match content::load_prefab_library(&self.paths) {
            Ok(prefabs) if prefabs != self.prefabs => {
                self.prefabs = prefabs;
                self.prefab_names = sorted_names(self.prefabs.keys().map(|r| r.as_str().to_string()));
                let coords: Vec<ChunkCoord> = self.store.iter_loaded().map(|(c, _)| c).collect();
                for coord in coords {
                    self.reload_chunk(coord);
                }
                println!("wok: reloaded prefab library");
            }
            Ok(_) => {}
            Err(err) => eprintln!("wok: prefab reload failed, keeping previous: {err}"),
        }
    }

    /// Re-read one chunk and its terrain from disk and replace its store entry: release the old
    /// runtime arrays, then load fresh ones (a deleted chunk file just releases). Uses the current
    /// prefab library, so this also serves the prefab-change re-transform.
    fn reload_chunk(&mut self, coord: ChunkCoord) {
        if self.store.state(coord) == ChunkState::Loaded {
            let _ = self.store.release(coord);
        }
        if !self.paths.chunk(coord).exists() {
            return;
        }
        match content::load_chunk_with_heightmap(&self.paths, coord) {
            Ok((chunk, heightmap)) => {
                if let Err(err) = self.store.load(chunk, heightmap, &self.prefabs) {
                    eprintln!("wok: chunk {}_{} reload failed, now unloaded: {err}", coord.x, coord.z);
                } else {
                    println!("wok: reloaded chunk {}_{}", coord.x, coord.z);
                }
            }
            Err(err) => eprintln!("wok: chunk {}_{} reload failed, keeping previous: {err}", coord.x, coord.z),
        }
    }

    /// Re-read the scene's default light state and re-scan the browser listing.
    fn reload_light(&mut self) {
        let name = self.scene.default_lighting.as_str();
        match wok_light::load_light_state(self.paths.light(name)) {
            Ok((_, state)) => {
                self.light = state;
                self.light_names = content::scan_light_names(&self.paths);
                println!("wok: reloaded light state {name:?}");
            }
            Err(err) => eprintln!("wok: light reload failed, keeping previous: {err}"),
        }
    }
}

/// The content-browser's read-only view of a loaded scene: the scene name (the one clickable entry
/// that opens the Scene tab) and the prefab and lighting listings (inert until those views exist).
/// Borrowed from the residency so the view needs no copy, and cheap to construct by hand in a
/// snapshot test (it holds only references).
#[derive(Clone, Copy)]
pub struct ContentView<'a> {
    pub scene_name: &'a str,
    pub prefabs: &'a [String],
    pub lights: &'a [String],
}

impl LoadedScene {
    /// The content-browser view over this scene's listings.
    pub fn content_view(&self) -> ContentView<'_> {
        ContentView { scene_name: &self.scene.name, prefabs: &self.prefab_names, lights: &self.light_names }
    }
}

/// Collect names into a sorted, deduplicated `Vec` for a content-browser listing.
fn sorted_names(names: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut names: Vec<String> = names.into_iter().collect();
    names.sort_unstable();
    names.dedup();
    names
}

/// Whether `root` is a fresh place to create a project: it does not exist yet, or it exists and is
/// empty. A folder that already holds content is not fresh, so [`LoadedScene::open`] refuses to
/// generate into it. A path that cannot be read (e.g. permissions) is treated as fresh, so the open
/// surfaces generate's own IO error rather than a misleading "not a wok project".
fn is_fresh_project_dir(root: &Path) -> bool {
    match std::fs::read_dir(root) {
        Ok(mut entries) => entries.next().is_none(),
        Err(_) => true,
    }
}

/// The picked folder is not a wok project (no `scene.json`) and is not empty, so opening it would
/// neither load existing content nor safely create new content. Carries the root for the message the
/// editor surfaces.
#[derive(Debug)]
struct NotAProject {
    root: PathBuf,
}

impl std::fmt::Display for NotAProject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "not a wok project: no scene.json in {}", self.root.display())
    }
}

impl Error for NotAProject {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[allow(clippy::float_cmp)]
    #[test]
    fn chunk_origin_scales_by_the_chunk_size() {
        assert_eq!(chunk_origin(ChunkCoord::new(0, 0)), Vec3::ZERO);
        assert_eq!(chunk_origin(ChunkCoord::new(2, -1)), Vec3::new(256.0, 0.0, -128.0));
    }

    fn unique_temp_dir() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("wok-scene-test-{}-{}", std::process::id(), n))
    }

    #[test]
    fn open_generates_the_sample_in_an_empty_dir_and_loads_it() {
        // An existing empty directory has no scene.json, so open generates the sample and loads it:
        // one chunk in the store, the prefab and lighting listings populated, and a finite
        // scene-bounds box for the shadow region.
        let dir = unique_temp_dir();
        std::fs::create_dir_all(&dir).expect("create the empty project dir");
        let loaded = LoadedScene::open(dir.clone()).expect("an empty dir generates and loads");
        assert_eq!(loaded.store.iter_loaded().count(), 1, "the sample has one chunk");
        assert_eq!(loaded.prefab_names, ["boulder", "crate", "marker", "pillar"]);
        assert_eq!(loaded.light_names, [sample::LIGHT_NAME]);
        assert!(loaded.scene_bounds().min.is_finite() && loaded.scene_bounds().max.is_finite());
        // The camera spawns above the terrain, finite and looking down a little.
        let cam = loaded.spawn_camera();
        assert!(cam.position.is_finite() && cam.pitch < 0.0);
        drop(loaded);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reopening_an_existing_project_loads_without_regenerating() {
        // Second open of the same dir: scene.json now exists, so it loads the written content rather
        // than regenerating - the same store shape, proving the load path (not just generation) works.
        let dir = unique_temp_dir();
        let first = LoadedScene::open(dir.clone()).expect("first open generates");
        let scene_name = first.scene.name.clone();
        drop(first);
        let second = LoadedScene::open(dir.clone()).expect("second open loads");
        assert_eq!(second.scene.name, scene_name);
        assert_eq!(second.store.iter_loaded().count(), 1);
        drop(second);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn open_a_non_wok_folder_errors_and_writes_nothing() {
        // A non-empty folder without scene.json is some directory picked by mistake: open reports it
        // as not a wok project and writes nothing - it neither loads nor scatters sample content in.
        let dir = unique_temp_dir();
        std::fs::create_dir_all(&dir).expect("create the non-wok dir");
        std::fs::write(dir.join("README.md"), b"not a wok project").expect("seed a stray file");
        let message = match LoadedScene::open(dir.clone()) {
            Ok(_) => panic!("a non-wok folder must not open"),
            Err(err) => format!("{err}"),
        };
        assert!(message.contains("no scene.json"), "the error should name the missing scene.json: {message}");
        assert!(!dir.join("scene.json").exists(), "open must not generate a scene into a non-wok folder");
        assert!(!dir.join("prefabs").exists(), "open must not scatter sample prefabs into a non-wok folder");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
