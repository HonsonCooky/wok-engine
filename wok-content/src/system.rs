//! `ContentSystem` - the public entry point. Plan section 3.1.
//!
//! Phase A step 6 stands up the chunk-lifecycle subset: `new`, `load_scene`, `unload_scene`,
//! `request_load`, `request_unload`, `poll`, `slot`. Step 7 adds `transition_chunk`. Step 8
//! wires terrain into the load pipeline. Step 9 adds the read accessors
//! (`slots`, `active_slots`, `vista_slots`, `mesh`, `registry`, `authored_eagerness`,
//! `streaming_eagerness`).
//!
//! Phase A's worker is the synchronous `LoopbackWorker`; the threaded worker lands in
//! Phase B.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use pantry::wgpu;
use wok_scene::{Chunk, ChunkCoord, Prefab, PrefabId, Scene};

use wok_scene::ChunkEagerness;

use crate::chunk::{
    ChunkSlot, ContentEvent, LoadHandle, SlotState,
    load::{integrate_result, should_dispatch},
    transition::apply_transition,
    unload::{apply_unload, finalize_unloads},
};
use crate::error::TransitionError;
use crate::config::ContentConfig;
use crate::error::LoadError;
use crate::registry::Registry;
use crate::worker::{LoopbackWorker, WorkerRequest};

/// The engine's content system. Owns the registry, the chunk lifecycle map, the worker,
/// and (when a scene is loaded) the resident scene data.
pub struct ContentSystem {
    content_root: PathBuf,
    config: Arc<ContentConfig>,
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    registry: Registry,
    slots: HashMap<ChunkCoord, ChunkSlot>,
    worker: LoopbackWorker,
    scene: Option<LoadedScene>,
    tick: u64,
    /// Queued events waiting for the next `poll()` to emit them. Plan section 3.1 has
    /// `poll` as the only event channel, so synchronous operations (load_scene,
    /// unload_scene, transition_chunk) stash here rather than returning events directly.
    pending_events: Vec<ContentEvent>,
}

/// Resident scene state. Plan section 3.1. Phase A loads every chunk's authored data and
/// the entire prefab set eagerly at scene boot (plan section 5.2).
pub struct LoadedScene {
    pub manifest: Scene,
    pub scene_dir: PathBuf,
    pub chunks_authored: HashMap<ChunkCoord, Arc<Chunk>>,
    pub prefabs: Arc<HashMap<PrefabId, Prefab>>,
}

impl ContentSystem {
    /// Construct a new content system. `content_root` is the directory containing
    /// `registry.json`, `prefabs/`, `scenes/`, etc. The registry is loaded from
    /// `<content_root>/registry.json` if the file exists; otherwise an empty registry is
    /// used (the scene boot path may populate it from authored data).
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        content_root: PathBuf,
        config: ContentConfig,
    ) -> Result<Self, LoadError> {
        let registry_path = content_root.join("registry.json");
        let registry = if registry_path.is_file() {
            Registry::load(&registry_path)?
        } else {
            Registry::empty()
        };
        Ok(ContentSystem {
            content_root,
            config: Arc::new(config),
            device,
            queue,
            registry,
            slots: HashMap::new(),
            worker: LoopbackWorker::new(),
            scene: None,
            tick: 0,
            pending_events: Vec::new(),
        })
    }

    /// Cooperative shutdown. Phase A's loopback worker has nothing to drain; Phase B's
    /// threaded worker uses this to signal and join. `Drop` cannot do it safely (the join
    /// would race against `Arc<wgpu::Device>` drop). Plan section 6.5.
    pub fn shutdown(self) {
        drop(self);
    }

    /// Load a scene from disk. Reads `scene.json`, every chunk file, and every prefab in
    /// `<content_root>/prefabs/`. Eagerly stores authored data per plan section 5.2.
    pub fn load_scene(&mut self, scene_dir: &Path) -> Result<(), LoadError> {
        // Drop any previously-loaded scene first (plan section 9.13 single-scene rule).
        if self.scene.is_some() {
            self.unload_scene();
        }
        let absolute_dir = if scene_dir.is_absolute() {
            scene_dir.to_owned()
        } else {
            self.content_root.join(scene_dir)
        };
        let manifest = wok_scene::load_scene_manifest(&absolute_dir)?;
        let manifest_id = manifest.id.clone();
        let mut chunks_authored: HashMap<ChunkCoord, Arc<Chunk>> =
            HashMap::with_capacity(manifest.chunks.len());
        for coord in &manifest.chunks {
            let chunk = wok_scene::load_chunk(&absolute_dir, *coord)?;
            chunks_authored.insert(*coord, Arc::new(chunk));
        }
        let prefabs_dir = self.content_root.join("prefabs");
        let prefabs_map = if prefabs_dir.is_dir() {
            wok_scene::load_prefab_dir(&prefabs_dir)?
        } else {
            HashMap::new()
        };
        let prefabs = Arc::new(prefabs_map);

        // Populate registry usage from the freshly-loaded data. Borrow-only walk via
        // chunks_authored (HashMap<ChunkCoord, Arc<Chunk>>); flatten the Arcs locally.
        let chunks_for_populate: HashMap<ChunkCoord, Chunk> = chunks_authored
            .iter()
            .map(|(c, arc)| (*c, (**arc).clone()))
            .collect();
        self.registry
            .populate_from_scene(&manifest, &prefabs, &chunks_for_populate)?;

        self.scene = Some(LoadedScene {
            manifest,
            scene_dir: absolute_dir,
            chunks_authored,
            prefabs,
        });
        self.pending_events
            .push(ContentEvent::SceneLoaded(manifest_id));
        Ok(())
    }

    /// Unload the current scene. Removes the LoadedScene, drops every resident slot's GPU
    /// handles, and queues the `SceneUnloaded` event for the next poll().
    pub fn unload_scene(&mut self) {
        let Some(scene) = self.scene.take() else { return };
        // Drop every slot (cancels in-flight loads via the missing-slot path; releases GPU
        // resources for resident slots).
        self.slots.clear();
        self.pending_events
            .push(ContentEvent::SceneUnloaded(scene.manifest.id));
        // The LoadedScene drops here; its Arc<Chunk> and Arc<HashMap<PrefabId, Prefab>>
        // release with it.
    }

    /// Flip a Resident slot's runtime eagerness. Plan section 3.1 + 5; mechanics in
    /// `chunk/transition.rs`. The `ChunkTransitioned` event is queued for the next
    /// `poll()`. Returns `TransitionError::UnknownSlot` if the coord has no slot;
    /// `TransitionError::NotResident` if the slot is Pending / Loading / Failed.
    pub fn transition_chunk(
        &mut self,
        coord: ChunkCoord,
        new: ChunkEagerness,
    ) -> Result<(), TransitionError> {
        if let Some(event) = apply_transition(&mut self.slots, coord, new)? {
            self.pending_events.push(event);
        }
        Ok(())
    }

    /// Idempotent. If the coord already has a Pending / Loading / Resident slot, returns
    /// the existing handle and does not re-dispatch. If the slot is Failed / Unloading or
    /// missing, dispatches.
    pub fn request_load(&mut self, coord: ChunkCoord) -> LoadHandle {
        // No scene loaded - return a handle for the missing slot. The game will observe the
        // missing slot via `system.slot(coord)`. We do not synthesize a Failed slot because
        // there is no scene context to fail against; load_scene must come first.
        let Some(scene) = &self.scene else {
            return LoadHandle::new(coord);
        };
        if !should_dispatch(self.slots.get(&coord)) {
            return LoadHandle::new(coord);
        }
        let Some(chunk) = scene.chunks_authored.get(&coord) else {
            // Coord not in the loaded scene's manifest. Mark Failed via a synthetic worker
            // result-style entry. Phase A does this inline; Phase B's threaded worker can
            // produce the same outcome through the channel.
            self.slots.insert(
                coord,
                ChunkSlot {
                    coord,
                    state: SlotState::Failed(Arc::new(LoadError::AssetMissing {
                        kind: crate::error::AssetKind::Mesh,
                        slug: None,
                        serial: u32::MAX, // sentinel: "no such chunk in the scene"
                    })),
                    touched_tick: self.tick,
                },
            );
            return LoadHandle::new(coord);
        };
        // Insert Pending slot and submit a worker request.
        self.slots.insert(
            coord,
            ChunkSlot {
                coord,
                state: SlotState::Pending,
                touched_tick: self.tick,
            },
        );
        self.worker.submit(WorkerRequest::LoadChunk {
            coord,
            chunk: Arc::clone(chunk),
            prefabs: Arc::clone(&scene.prefabs),
            registry: self.registry.read_view(),
            device: Arc::clone(&self.device),
            queue: Arc::clone(&self.queue),
            config: Arc::clone(&self.config),
        });
        LoadHandle::new(coord)
    }

    /// Request a chunk be unloaded. Plan section 3.2 details the state transitions; see
    /// `chunk::unload::apply_unload`.
    pub fn request_unload(&mut self, coord: ChunkCoord) {
        apply_unload(&mut self.slots, coord);
    }

    /// Drain worker results, finalize unloads, and emit the resulting events. Plan section
    /// 3.1 `poll`. Phase A's loopback worker runs every queued request synchronously
    /// inside this method.
    pub fn poll(&mut self) -> Vec<ContentEvent> {
        self.tick = self.tick.saturating_add(1);
        let mut events: Vec<ContentEvent> = std::mem::take(&mut self.pending_events);
        // Drain worker queue. We collect results first so the closure does not borrow self;
        // then integrate.
        let mut results = Vec::new();
        self.worker.drain(|r| results.push(r));
        for r in results {
            if let Some(e) = integrate_result(&mut self.slots, r) {
                events.push(e);
            }
        }
        finalize_unloads(&mut self.slots, &mut events);
        events
    }

    /// Inspect a slot by coord. Returns `None` if the coord has no slot. Lifecycle-level
    /// inspection only; consumers should iterate via the accessors landing in step 9.
    pub fn slot(&self, coord: ChunkCoord) -> Option<&ChunkSlot> {
        self.slots.get(&coord)
    }

    /// Borrow the current resident scene state. `None` if no scene is loaded.
    pub fn scene(&self) -> Option<&LoadedScene> {
        self.scene.as_ref()
    }
}

