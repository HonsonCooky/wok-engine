//! Chunk lifecycle: slot state machine, request_load, request_unload, poll integration. The
//! transition operation (`transition_chunk`) lands in step 7; terrain mesh generation in
//! step 8. The synchronous `LoopbackWorker` (see `crate::worker`) backs every load.

use std::sync::Arc;

use wok_scene::{ChunkCoord, ChunkEagerness, SceneId};

use crate::error::LoadError;

pub mod load;
pub mod slot;
pub mod transition;
pub mod unload;

pub use slot::{
    ChunkGpuHandles, ChunkSlot, LoadHandle, LoadStatus, ResidentChunk, SlotState, TerrainGpu,
    VisibleMesh,
};

/// Phase A's `MeshGpuRef` alias - the slot-owned visible mesh. Re-exported under both names
/// so external code that adopts the plan-document term (`MeshGpuRef`) finds the type while
/// Phase A retains the slot-owned shape. Phase B will reintroduce a real `MeshGpuRef` that
/// references a registry-tracked `MeshId`; the alias serves as a marker for the migration.
pub type MeshGpuRef = VisibleMesh;

/// Events emitted by `ContentSystem::poll()`. Game polls each tick and drains these. Plan
/// section 3.6.
#[derive(Debug)]
pub enum ContentEvent {
    ChunkResident(ChunkCoord),
    ChunkUnloaded(ChunkCoord),
    ChunkFailed {
        coord: ChunkCoord,
        error: Arc<LoadError>,
    },
    ChunkTransitioned {
        coord: ChunkCoord,
        from: ChunkEagerness,
        to: ChunkEagerness,
    },
    SceneLoaded(SceneId),
    SceneUnloaded(SceneId),
    HotReload(HotReloadKind),
}

/// Hot-reload event variants. Phase E populates this; Phase A defines the enum so
/// `ContentEvent::HotReload` is constructible by the future watcher integration without a
/// type change.
#[derive(Debug, Clone)]
pub enum HotReloadKind {
    SceneManifest,
    PrefabChanged(wok_scene::PrefabId),
    ChunkChanged(ChunkCoord),
}
