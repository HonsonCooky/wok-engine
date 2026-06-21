//! Read-only loading of an editor-authored content directory.
//!
//! taste consumes the engine-owned content layout (`wok_scene::ContentLayout`, the `assets/`
//! convention from HLD "content conventions and integrity"): a project root holds an `assets/`
//! folder, and under it `scenes/<scene>/` is one self-contained folder per scene (its `scene.json`,
//! the `{x}_{z}.json` chunk files, and the sibling `{x}_{z}.heightmap.bin` terrain), with
//! project-shared `prefabs/<slug>.json` and `lighting/<name>.json`. The layout - path resolution
//! and the discovery scans - is the engine's, so taste neither restates the convention nor parses
//! file names itself.
//!
//! A demo plays one scene, so taste loads the first scene name (sorted) under `assets/scenes`: a
//! single-scene project just works, and a multi-scene one is deterministic. taste never writes
//! content and never watches it: the editor authors, the game plays what is on disk at startup.
//! Errors are `Box<dyn Error>` per the wok precedent - the startup path only needs "did it work,
//! and what is the message", never to distinguish failure modes programmatically.

use std::collections::HashMap;
use std::error::Error;

use wok_light::LightState;
use wok_scene::{Chunk, ContentLayout, Heightmap, Prefab, PrefabRef, Scene};

/// Everything taste loads from disk at startup: the authored forms plus the resolved light state.
/// Chunks are paired with their optional heightmaps, sorted by coordinate so downstream work (store
/// loads, GPU uploads) happens in a deterministic order.
pub struct LoadedContent {
    pub scene: Scene,
    pub prefabs: HashMap<PrefabRef, Prefab>,
    pub chunks: Vec<(Chunk, Option<Heightmap>)>,
    pub light: LightState,
}

/// Load the project's content: the first scene (its manifest and every chunk with its terrain), the
/// project-shared prefab library, and the scene's default light state. The scene played is the
/// first name under `assets/scenes` sorted, since a demo plays one scene.
pub fn load_all(layout: &ContentLayout) -> Result<LoadedContent, Box<dyn Error>> {
    let scene_name = first_scene(layout)?;
    let scene = wok_scene::load_scene(layout.scene_json(&scene_name))?;

    let mut chunks = Vec::new();
    for coord in layout.chunk_coords(&scene_name) {
        let chunk = wok_scene::load_chunk(layout.chunk(&scene_name, coord))?;
        let heightmap_path = layout.heightmap(&scene_name, coord);
        let heightmap = if heightmap_path.exists() {
            Some(wok_scene::load_heightmap(heightmap_path)?)
        } else {
            None
        };
        chunks.push((chunk, heightmap));
    }

    let prefabs = load_prefab_library(layout)?;
    let (_, light) = wok_light::load_light_state(layout.lighting(scene.default_lighting.as_str()))?;

    Ok(LoadedContent { scene, prefabs, chunks, light })
}

/// The scene a demo plays: the first name (sorted) under `assets/scenes`. taste never generates
/// content, so an empty project is the one failure with advice attached - run the editor first.
fn first_scene(layout: &ContentLayout) -> Result<String, Box<dyn Error>> {
    layout.scene_names().into_iter().next().ok_or_else(|| {
        format!(
            "no scenes under {}; run the wok editor first to author content",
            layout.scenes_dir().display()
        )
        .into()
    })
}

/// Load every prefab under `assets/prefabs/`, keyed by file-stem slug. The discovery is the engine's
/// (sorted, tolerant of a missing folder); a project without shared prefabs is an empty library.
fn load_prefab_library(layout: &ContentLayout) -> Result<HashMap<PrefabRef, Prefab>, Box<dyn Error>> {
    let mut prefabs = HashMap::new();
    for slug in layout.prefab_slugs() {
        prefabs.insert(PrefabRef::new(&slug), wok_scene::load_prefab(layout.prefab(&slug))?);
    }
    Ok(prefabs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use wok_scene::ChunkCoord;

    // A unique temp root per test (pid + atomic counter, no wall-clock - wok-scene's pattern). The
    // fixture below seeds an assets/ tree under it, so the loader is exercised without the
    // gitignored /content.
    fn unique_temp_dir() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let pid = std::process::id();
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("taste-content-{pid}-{n}"))
    }

    // Minimal-but-valid authored files, matching the on-disk shapes the engine loaders parse.
    const SCENE_JSON: &str = r#"{
        "name": "sample",
        "default_lighting": "default",
        "default_streaming": { "load_radius": 2, "default_eagerness": "Eager" },
        "next_instance_id": 1
    }"#;
    const CHUNK_JSON: &str = r#"{ "coord": { "x": 0, "z": 0 } }"#;
    const PREFAB_JSON: &str = r#"{ "states": [{ "name": "default", "shapes": [] }], "default_state": "default" }"#;
    const LIGHT_JSON: &str = r#"{
        "sun": { "direction": [-0.4, -1.0, -0.3], "color": [1.0, 0.95, 0.85] },
        "ambient": [0.12, 0.12, 0.16],
        "fog": { "color": [0.65, 0.7, 0.8], "start": 60.0, "end": 260.0 },
        "sky": { "horizon": [0.65, 0.7, 0.8], "zenith": [0.25, 0.45, 0.85] },
        "cel": { "band_count": 32, "transition_softness": 0.08, "rim_intensity": 0.35 }
    }"#;

    // Seed one scene under `assets/scenes/<scene>/` (its manifest + one chunk, no heightmap), one
    // shared prefab, and the `default` light state the manifest points at.
    fn seed_project(layout: &ContentLayout, scene: &str) {
        std::fs::create_dir_all(layout.scene_dir(scene)).unwrap();
        std::fs::write(layout.scene_json(scene), SCENE_JSON).unwrap();
        std::fs::write(layout.chunk(scene, ChunkCoord::new(0, 0)), CHUNK_JSON).unwrap();

        std::fs::create_dir_all(layout.prefabs_dir()).unwrap();
        std::fs::write(layout.prefab("crate"), PREFAB_JSON).unwrap();

        std::fs::create_dir_all(layout.lighting_dir()).unwrap();
        std::fs::write(layout.lighting("default"), LIGHT_JSON).unwrap();
    }

    #[test]
    fn load_all_reads_the_scene_with_its_chunks_prefabs_and_light() {
        let root = unique_temp_dir();
        let _ = std::fs::remove_dir_all(&root);
        let layout = ContentLayout::new(&root);
        seed_project(&layout, "sample");

        let loaded = load_all(&layout).unwrap();
        assert_eq!(loaded.scene.name, "sample");
        assert_eq!(loaded.chunks.len(), 1);
        assert_eq!(loaded.chunks[0].0.coord, ChunkCoord::new(0, 0));
        assert!(loaded.chunks[0].1.is_none(), "no heightmap seeded, so the chunk has no terrain");
        assert_eq!(loaded.prefabs.len(), 1);
        assert!(loaded.prefabs.contains_key(&PrefabRef::new("crate")));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn load_all_plays_the_first_scene_name_sorted() {
        // Two scenes on disk; a demo plays one, and the choice is the sorted-first name so it is
        // deterministic regardless of directory order. "alpha" sorts before the seeded "sample".
        let root = unique_temp_dir();
        let _ = std::fs::remove_dir_all(&root);
        let layout = ContentLayout::new(&root);
        seed_project(&layout, "sample");
        std::fs::create_dir_all(layout.scene_dir("alpha")).unwrap();
        std::fs::write(layout.scene_json("alpha"), SCENE_JSON.replace("sample", "alpha")).unwrap();

        let loaded = load_all(&layout).unwrap();
        assert_eq!(loaded.scene.name, "alpha", "the sorted-first scene is the one played");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn load_all_errors_clearly_when_there_are_no_scenes() {
        // The empty-project case: no assets/ at all. taste never generates, so the message advises
        // running the editor first rather than reporting a bare missing-file error.
        let root = unique_temp_dir();
        let _ = std::fs::remove_dir_all(&root);
        let layout = ContentLayout::new(&root);
        // `LoadedContent` is not `Debug` (it keeps its shape), so match rather than `unwrap_err`.
        let err = match load_all(&layout) {
            Ok(_) => panic!("expected an error for a project with no scenes"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("no scenes"), "message should explain the empty project: {err}");

        let _ = std::fs::remove_dir_all(&root);
    }
}
