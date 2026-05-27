//! Orchestrator crate between `wok-scene` and the rest of the engine. Owns the asset registry,
//! the chunk lifecycle (load / unload / transition), procedural primitives, terrain mesh
//! generation, GPU upload coordination, and the public `ContentSystem` API.
//!
//! Phase A scope: registry, primitives, storage, chunk slot state machine, terrain mesh, and
//! the synchronous `LoopbackWorker`. Streaming (Phase C), the real background worker thread
//! (Phase B), snapshots (Phase D), and hot-reload integration (Phase E) land in later phases.
//! See `wok-engine-designs/wok-content-plan.md` for the canonical plan.

pub use pantry;
pub use wok_scene;

pub mod config;
pub mod error;
pub mod primitives;
pub mod registry;
pub mod storage;

pub use config::{ContentConfig, SurfaceTagPalette};
pub use error::{AssetKind, LoadError, RegistryError, SaveError, SnapshotError, TransitionError};
pub use registry::{
    AnimationEntry, AnimationReadEntry, AnimationSerial, AssetStatus, AudioEntry, AudioReadEntry,
    AudioSerial, EntrySlot, KindTable, LightEntry, LightReadEntry, LightSerial, MeshEntry,
    MeshReadEntry, MeshSerial, Registry, RegistryReadView, UsageSite, VoiceEntry, VoiceReadEntry,
    VoiceSerial,
};
pub use storage::{MeshCpu, MeshGpu, MeshVertex};
