//! The "load a chunk" pipeline. Runs the work the worker thread would do; for Phase A's
//! LoopbackWorker the worker thread is the main thread. Plan section 5.1.
//!
//! Sequence:
//! 1. Slice authored chunk into `ChunkRuntime` via `wok_scene::slice_chunk`. Maps
//!    `SliceError::UnknownPrefab` to `LoadError::PrefabMissing` so callers can distinguish
//!    "prefab not in scene's resident set" from generic slice failures.
//! 2. Verify every prefab-state `mesh_override` referenced by a placement is present in the
//!    registry read view. Missing references surface as `LoadError::AssetMissing` (§7.3
//!    test 7). Phase A does not consume the override (no shipped meshes); the lookup
//!    enforces the registry's coverage of authored asset references.
//! 3. For each `VisibleShape` in the sliced runtime, generate a procedural placeholder mesh
//!    from its inline `ShapePrimitive` and upload it to the GPU. Slot-owned per the Phase A
//!    deviation noted in `chunk/slot.rs`.
//! 4. Terrain mesh generation lands in step 8; Phase A step 6 always yields `None`.

use std::sync::Arc;

use wok_scene::{slice_chunk, SliceError};

use crate::chunk::VisibleMesh;
use crate::error::{AssetKind, LoadError};
use crate::primitives;
use crate::storage;
use crate::terrain;
use crate::worker::protocol::{ChunkLoadedPayload, WorkerRequest, WorkerResult};

pub fn run_load_chunk(request: WorkerRequest) -> WorkerResult {
    match request {
        WorkerRequest::LoadChunk {
            coord,
            chunk,
            prefabs,
            registry,
            device,
            queue,
            config,
        } => {
            // Slice.
            let runtime = match slice_chunk(&chunk, &*prefabs) {
                Ok(r) => r,
                Err(e) => return WorkerResult::ChunkFailed {
                    coord,
                    error: Arc::new(slice_error_to_load_error(e)),
                },
            };

            // Verify every prefab-state mesh_override referenced by a placement resolves in
            // the registry view. The slicer would have already failed on unknown
            // prefab/state, so unwrap is safe under the slicer's contract.
            for placement in &chunk.placements {
                let prefab = prefabs
                    .get(&placement.prefab)
                    .expect("slice succeeded; prefab must be resident");
                let state = prefab
                    .states
                    .iter()
                    .find(|s| s.name == placement.state)
                    .expect("slice succeeded; state must exist");
                if let Some(mesh_id) = &state.mesh_override
                    && registry.mesh(mesh_id.serial()).is_none()
                {
                    return WorkerResult::ChunkFailed {
                        coord,
                        error: Arc::new(LoadError::AssetMissing {
                            kind: AssetKind::Mesh,
                            slug: Some(mesh_id.slug().clone()),
                            serial: mesh_id.serial(),
                        }),
                    };
                }
            }

            // Generate visible meshes.
            let mut visible_meshes = Vec::with_capacity(runtime.visible.len());
            for (idx, vs) in runtime.visible.iter().enumerate() {
                let cpu = primitives::generate(&vs.primitive, &config);
                let label =
                    format!("chunk({},{}).visible[{idx}]", coord.x, coord.z);
                let gpu = match storage::upload(&device, &queue, &cpu, &label) {
                    Ok(g) => g,
                    Err(e) => {
                        return WorkerResult::ChunkFailed {
                            coord,
                            error: Arc::new(e),
                        };
                    }
                };
                visible_meshes.push(VisibleMesh {
                    gpu,
                    source_visible_index: idx as u32,
                    local_transform: vs.local_transform,
                });
            }

            // Terrain: slot-owned, generated only when the runtime carries a terrain field.
            // Plan section 9.17 (slot ownership) and 9.18 (NW-SE triangulation locked).
            let terrain_gpu = if runtime.terrain.is_some() {
                let cpu = terrain::generate_mesh(&runtime, &config.terrain_palette);
                let label = format!("chunk({},{}).terrain", coord.x, coord.z);
                match storage::upload(&device, &queue, &cpu, &label) {
                    Ok(g) => Some(g),
                    Err(e) => {
                        return WorkerResult::ChunkFailed {
                            coord,
                            error: Arc::new(e),
                        };
                    }
                }
            } else {
                None
            };

            WorkerResult::ChunkLoaded(Box::new(ChunkLoadedPayload {
                coord,
                runtime,
                visible_meshes,
                terrain_gpu,
            }))
        }
    }
}

/// Map wok-scene's `SliceError` into our `LoadError`. The unknown-prefab variant becomes
/// `LoadError::PrefabMissing` (plan section 3.5 surface): the prefab id is what the caller
/// needs to act on, and we already know the chunk coord from the request context.
fn slice_error_to_load_error(e: SliceError) -> LoadError {
    match e {
        SliceError::UnknownPrefab(id) => LoadError::PrefabMissing(id),
        other => LoadError::Slice(other),
    }
}
