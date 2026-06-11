//! Application of watcher-reported file changes to the running editor.
//!
//! The watcher reports raw paths; `crate::content::classify` maps each to a reload action, and
//! this module applies them. Chunk and scene files go through `crate::sync`'s content-compared
//! path, so the editor's own saves (which the watcher reports like any other write) are no-ops;
//! prefab changes re-transform the chunks from the in-memory authored forms, preserving unsaved
//! edits. Reload failures are printed and skipped rather than crashing: a watcher event can
//! arrive mid-write with a half-written file, and the completing write delivers a second event
//! that retries.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use wok_light::LightState;
use wok_mesh::MeshGpu;
use wok_platform::wgpu;
use wok_scene::ChunkCoord;

use crate::content::{self, ContentPaths, Reload};
use crate::model::EditorModel;
use crate::sync::{self, ChunkOutcome};

/// Apply one frame's worth of watcher-reported changes.
pub fn apply(
    paths: &ContentPaths,
    model: &mut EditorModel,
    light: &mut LightState,
    terrain_gpu: &mut BTreeMap<ChunkCoord, MeshGpu>,
    device: &wgpu::Device,
    changed: Vec<PathBuf>,
) {
    let mut scene_changed = false;
    let mut prefabs_changed = false;
    let mut light_changed = false;
    let mut chunks: BTreeSet<ChunkCoord> = BTreeSet::new();
    for path in changed {
        match content::classify(&paths.root, &path) {
            Some(Reload::Scene) => scene_changed = true,
            Some(Reload::Prefabs) => prefabs_changed = true,
            Some(Reload::Chunk(coord)) => {
                chunks.insert(coord);
            }
            Some(Reload::Light(name)) if name == model.scene.default_lighting.as_str() => {
                light_changed = true;
            }
            _ => {}
        }
    }

    if scene_changed {
        match wok_scene::load_scene(paths.scene()) {
            Ok(scene) => {
                let lighting_changed = scene.default_lighting != model.scene.default_lighting;
                if sync::apply_external_scene(model, scene) {
                    light_changed |= lighting_changed;
                    println!("wok: reloaded scene manifest");
                }
            }
            Err(err) => eprintln!("wok: scene reload failed, keeping previous: {err}"),
        }
    }

    if prefabs_changed {
        match content::load_prefab_library(paths) {
            Ok(prefabs) if prefabs != model.prefabs => {
                model.prefabs = prefabs;
                // The runtime arrays do not retain which prefabs a chunk placed, so a prefab
                // change re-transforms every chunk - from memory, keeping unsaved edits.
                for coord in model.chunks.keys().copied().collect::<Vec<_>>() {
                    if let Err(err) = model.retransform(coord) {
                        eprintln!("wok: chunk {}_{} re-transform failed, now unloaded: {err}", coord.x, coord.z);
                    }
                }
                model.validate_selection();
                println!("wok: reloaded prefab library");
            }
            Ok(_) => {}
            Err(err) => eprintln!("wok: prefab reload failed, keeping previous: {err}"),
        }
    }

    for coord in chunks {
        reload_chunk(paths, model, terrain_gpu, device, coord);
    }

    if light_changed {
        let name = model.scene.default_lighting.as_str();
        match wok_light::load_light_state(paths.light(name)) {
            Ok((_, state)) => {
                *light = state;
                println!("wok: reloaded light state {name:?}");
            }
            Err(err) => eprintln!("wok: light reload failed, keeping previous: {err}"),
        }
    }
}

/// Re-read one chunk from disk and apply it through the content-compared path, refreshing the
/// GPU terrain mesh only when something actually changed.
fn reload_chunk(
    paths: &ContentPaths,
    model: &mut EditorModel,
    terrain_gpu: &mut BTreeMap<ChunkCoord, MeshGpu>,
    device: &wgpu::Device,
    coord: ChunkCoord,
) {
    let on_disk = if paths.chunk(coord).exists() {
        match content::load_chunk_with_heightmap(paths, coord) {
            Ok(pair) => Some(pair),
            Err(err) => {
                eprintln!("wok: chunk {}_{} reload failed, keeping previous: {err}", coord.x, coord.z);
                return;
            }
        }
    } else {
        None
    };
    match sync::apply_external_chunk(model, coord, on_disk) {
        Ok(ChunkOutcome::Unchanged) => {}
        Ok(ChunkOutcome::Updated) => {
            refresh_terrain_gpu(model, terrain_gpu, device, coord);
            println!("wok: reloaded chunk {}_{}", coord.x, coord.z);
        }
        Ok(ChunkOutcome::Removed) => {
            refresh_terrain_gpu(model, terrain_gpu, device, coord);
            println!("wok: chunk {}_{} removed", coord.x, coord.z);
        }
        Err(err) => eprintln!("wok: chunk {}_{} reload failed, now unloaded: {err}", coord.x, coord.z),
    }
}

/// Re-upload (or drop) one chunk's terrain mesh after its runtime changed.
fn refresh_terrain_gpu(
    model: &EditorModel,
    terrain_gpu: &mut BTreeMap<ChunkCoord, MeshGpu>,
    device: &wgpu::Device,
    coord: ChunkCoord,
) {
    terrain_gpu.remove(&coord);
    if let Some(mesh) = model.store.get(coord).and_then(|r| r.terrain_mesh.as_ref()) {
        terrain_gpu.insert(coord, MeshGpu::upload(device, mesh));
    }
}
