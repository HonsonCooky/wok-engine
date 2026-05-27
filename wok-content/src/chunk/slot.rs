//! Slot state machine and the per-slot types. Plan section 3.2 fixes the shape: `SlotState`
//! holds the variant payload (`Resident` and `Unloading` both carry `ResidentChunk`);
//! `ChunkGpuHandles` holds the GPU resources the slot owns; `VisibleMesh` is the per-visible
//! shape's slot-owned GPU mesh.
//!
//! **Phase A deviation from plan section 3.2**: `MeshGpuRef { mesh_id, ... }` is replaced by
//! `VisibleMesh { gpu, ... }`. Plan section 9.4 mesh-immortality applies to registry-tracked
//! meshes; Phase A has none (only procedural placeholders), so visible meshes are slot-owned
//! like terrain (parallel to plan section 9.17 terrain ownership). Phase B reintroduces
//! `HashMap<MeshId, MeshGpu>` plus dedup once registry-tracked shipped meshes arrive; the
//! `MeshGpuRef` flavor lands then. Documented in the step-6 plan-vs-reality memo.

use std::sync::Arc;

use pantry::math::Mat4;
use wok_scene::{ChunkCoord, ChunkRuntime};

use crate::error::LoadError;
use crate::storage::MeshGpu;

/// One chunk slot in the lifecycle map. `state` carries the per-state payload; `touched_tick`
/// is the LRU timestamp (Phase B uses it for eviction).
#[derive(Debug)]
pub struct ChunkSlot {
    pub coord: ChunkCoord,
    pub state: SlotState,
    pub touched_tick: u64,
}

/// Lifecycle state for a chunk slot. The variants encode the "if Resident, runtime is Some"
/// invariant in the type system: there is no way to read `ResidentChunk` data from a slot
/// that is not `Resident` or `Unloading`. Plan section 3.2.
#[derive(Debug)]
pub enum SlotState {
    /// Requested; queued for the worker (not yet started).
    Pending,
    /// On the worker thread (Phase B). Phase A's `LoopbackWorker` processes synchronously,
    /// so this state is transient and not externally observable; included so the type matches
    /// the plan and Phase B does not need a refactor.
    Loading,
    /// Ready for consumers (renderer, physics, etc.).
    Resident(ResidentChunk),
    /// Scheduled for release. The chunk data is still readable this frame (consumers can
    /// finish their iteration); next `poll()` removes the slot and emits `ChunkUnloaded`.
    Unloading(ResidentChunk),
    /// Load attempt failed. The error is `Arc`-wrapped because both the slot and the
    /// `ChunkFailed` event need to reference it; `LoadError` cannot be `Clone` directly
    /// (it wraps `std::io::Error`).
    Failed(Arc<LoadError>),
}

impl SlotState {
    /// Short textual label for diagnostics. Stable across versions; tests should not pattern
    /// on it.
    pub fn label(&self) -> &'static str {
        match self {
            SlotState::Pending => "Pending",
            SlotState::Loading => "Loading",
            SlotState::Resident(_) => "Resident",
            SlotState::Unloading(_) => "Unloading",
            SlotState::Failed(_) => "Failed",
        }
    }
}

/// The data behind a `Resident` (or `Unloading`) slot. Runtime arrays come from
/// `wok_scene::slice_chunk`; GPU handles are slot-owned per the Phase A deviation noted at
/// the module level.
#[derive(Debug)]
pub struct ResidentChunk {
    pub runtime: ChunkRuntime,
    pub gpu: ChunkGpuHandles,
}

/// GPU resources held by a `ResidentChunk`. Every field drops with the slot.
#[derive(Debug)]
pub struct ChunkGpuHandles {
    /// One `VisibleMesh` per `ChunkRuntime.visible` entry, in matching order.
    pub visible: Vec<VisibleMesh>,
    /// Per-chunk terrain mesh. `Some` when the source `Chunk` carried `TerrainData`. Lands
    /// in step 8; step 6 always emits `None`.
    pub terrain: Option<MeshGpu>,
}

/// One slot-owned visible mesh. Wraps the procedurally-generated `MeshGpu` for one
/// `VisibleShape` entry. Phase A: `gpu` is owned per-slot. Phase B will replace this with
/// `MeshGpuRef { mesh_id, ... }` once registry-tracked meshes return.
#[derive(Debug)]
pub struct VisibleMesh {
    pub gpu: MeshGpu,
    /// Index into the source `ChunkRuntime.visible` array. The renderer reads
    /// `ChunkRuntime.visible[source_visible_index]` for the color and source placement
    /// (debug data); the GPU buffer here is for vertex/index data.
    pub source_visible_index: u32,
    /// Chunk-local transform for this visible shape (already composed by the slicer from
    /// placement.transform * shape.transform).
    pub local_transform: Mat4,
}

/// Slot-owned terrain mesh alias. Same shape as `Option<MeshGpu>`; named separately so the
/// public type carries intent.
pub type TerrainGpu = Option<MeshGpu>;

/// Token returned by `request_load`. Phase A's only operation on a handle is to inspect the
/// coord; the game typically waits for `ContentEvent::ChunkResident` instead. The opaque
/// shape leaves room for Phase B to add a poll method without breaking the public API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LoadHandle {
    coord: ChunkCoord,
}

impl LoadHandle {
    pub(crate) fn new(coord: ChunkCoord) -> Self {
        LoadHandle { coord }
    }

    pub fn coord(self) -> ChunkCoord {
        self.coord
    }
}

/// Surface-level status of a coord, suitable for polling without holding a `&ChunkSlot`.
/// Returned by `ContentSystem::load_status(coord)`. (Method lands in step 9.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadStatus {
    Unknown,
    Pending,
    Loading,
    Resident,
    Unloading,
    Failed,
}
