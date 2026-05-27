//! Worker request and result protocol. Crosses the channel boundary between the main
//! thread and the worker. Plan section 1.4 - the request carries its own self-contained
//! data; the result carries whatever the main thread needs to integrate.

use std::collections::HashMap;
use std::sync::Arc;

use pantry::wgpu;
use wok_scene::{Chunk, ChunkCoord, ChunkRuntime, Prefab, PrefabId};

use crate::chunk::VisibleMesh;
use crate::config::ContentConfig;
use crate::error::LoadError;
use crate::registry::RegistryReadView;
use crate::storage::MeshGpu;

/// Request enum. Phase A has one variant; Phase B and later phases add more (asset
/// pre-bake, snapshot capture, etc.).
pub enum WorkerRequest {
    /// Load a chunk: slice authored data, generate placeholder meshes, upload to GPU,
    /// generate terrain mesh. All inputs are `Arc`-wrapped so the request is cheaply
    /// cloned and the borrow checker stays out of the worker's way.
    LoadChunk {
        coord: ChunkCoord,
        chunk: Arc<Chunk>,
        prefabs: Arc<HashMap<PrefabId, Prefab>>,
        registry: Arc<RegistryReadView>,
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        config: Arc<ContentConfig>,
    },
}

impl std::fmt::Debug for WorkerRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorkerRequest::LoadChunk { coord, .. } => write!(
                f,
                "WorkerRequest::LoadChunk {{ coord: ({}, {}), .. }}",
                coord.x, coord.z
            ),
        }
    }
}

/// Result enum. Each variant matches a request kind in Phase A; later phases may produce
/// progress events alongside the final result. The `ChunkLoaded` payload is boxed because
/// `ChunkRuntime` carries multi-`Vec` arrays (visible, hitboxes, triggers, regions) that
/// blow the enum's stack footprint past a few hundred bytes if held inline. Boxing keeps
/// the enum cheap to move across the channel boundary in Phase B.
pub enum WorkerResult {
    ChunkLoaded(Box<ChunkLoadedPayload>),
    ChunkFailed {
        coord: ChunkCoord,
        error: Arc<LoadError>,
    },
}

/// Payload for `WorkerResult::ChunkLoaded`. Boxed inside the result variant; held inline
/// here for ergonomic field access at the integration site.
#[derive(Debug)]
pub struct ChunkLoadedPayload {
    pub coord: ChunkCoord,
    pub runtime: ChunkRuntime,
    pub visible_meshes: Vec<VisibleMesh>,
    pub terrain_gpu: Option<MeshGpu>,
}

impl std::fmt::Debug for WorkerResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorkerResult::ChunkLoaded(p) => write!(
                f,
                "WorkerResult::ChunkLoaded {{ coord: ({}, {}), .. }}",
                p.coord.x, p.coord.z
            ),
            WorkerResult::ChunkFailed { coord, error } => write!(
                f,
                "WorkerResult::ChunkFailed {{ coord: ({}, {}), error: {error} }}",
                coord.x, coord.z
            ),
        }
    }
}
