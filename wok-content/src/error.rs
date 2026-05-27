//! Public error enums. Narrow, hand-rolled `Display + std::error::Error` impls matching the
//! pattern wok-scene established. Each enum's variants name a single distinguishable failure
//! mode; the variant carries the context a caller needs to handle or report the failure.

use std::path::PathBuf;

use wok_scene::{ChunkCoord, PrefabId, SceneId, Slug};

/// What kind of asset a registry lookup or population failure refers to. Used for diagnostics
/// only; never the load-time discriminator (the load-time discriminator is the specific
/// `MeshId` / `AudioCueId` / etc. ID type).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetKind {
    Mesh,
    Audio,
    Animation,
    Voice,
    LightState,
}

impl std::fmt::Display for AssetKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            AssetKind::Mesh => "mesh",
            AssetKind::Audio => "audio",
            AssetKind::Animation => "animation",
            AssetKind::Voice => "voice",
            AssetKind::LightState => "light_state",
        };
        f.write_str(name)
    }
}

/// Errors that surface from chunk-loading, scene-loading, and worker pipelines. Broad on
/// purpose: a chunk load passes through file I/O, parse, slice, registry lookup, and GPU
/// upload, so the variant tells the caller which phase failed. The pipeline propagates with
/// `?` and the variant identifies the failure.
#[derive(Debug)]
pub enum LoadError {
    /// Underlying wok-scene load failure (file I/O, parse, format version, terrain sibling).
    Scene(wok_scene::LoadError),
    /// Underlying wok-scene slice failure (unknown prefab/state, invalid shape flags, terrain
    /// table overflow).
    Slice(wok_scene::SliceError),
    /// Registry mutation or lookup failure during populate or load.
    Registry(RegistryError),
    /// A placement referenced a prefab that the scene's resident prefab set did not contain.
    PrefabMissing(PrefabId),
    /// A runtime array referenced an asset serial that the registry could not resolve. The
    /// slug is best-effort: the registry has it when the serial is known but only partially
    /// populated; the serial alone is authoritative.
    AssetMissing {
        kind: AssetKind,
        slug: Option<Slug>,
        serial: u32,
    },
    /// GPU buffer creation or queue submission failed. The string is the underlying wgpu
    /// error message; rich wgpu error variants are not exposed here because wgpu's error
    /// types are not `'static + Send + Sync` in a uniform way across versions.
    Gpu(String),
    /// The background worker channel is closed (shutdown in progress, or the worker thread
    /// panicked and was not respawned). Phase A never produces this because the LoopbackWorker
    /// is synchronous; included so the variant exists when Phase B brings the real worker.
    WorkerGone,
    /// Raw I/O failure outside the wok-scene path (e.g. registry.json read).
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Scene(e) => write!(f, "scene load failed: {e}"),
            LoadError::Slice(e) => write!(f, "slice failed: {e}"),
            LoadError::Registry(e) => write!(f, "registry error: {e}"),
            LoadError::PrefabMissing(id) => write!(f, "prefab {} missing from loaded set", id.0),
            LoadError::AssetMissing {
                kind,
                slug,
                serial,
            } => match slug {
                Some(s) => write!(
                    f,
                    "asset missing: {kind} {s}-{serial} not present in registry"
                ),
                None => write!(f, "asset missing: {kind} serial {serial} not present in registry"),
            },
            LoadError::Gpu(msg) => write!(f, "GPU error: {msg}"),
            LoadError::WorkerGone => f.write_str("background worker is gone"),
            LoadError::Io { path, source } => {
                write!(f, "I/O error at {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for LoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            LoadError::Scene(e) => Some(e),
            LoadError::Slice(e) => Some(e),
            LoadError::Registry(e) => Some(e),
            LoadError::Io { source, .. } => Some(source),
            LoadError::PrefabMissing(_)
            | LoadError::AssetMissing { .. }
            | LoadError::Gpu(_)
            | LoadError::WorkerGone => None,
        }
    }
}

impl From<wok_scene::LoadError> for LoadError {
    fn from(e: wok_scene::LoadError) -> Self {
        LoadError::Scene(e)
    }
}

impl From<wok_scene::SliceError> for LoadError {
    fn from(e: wok_scene::SliceError) -> Self {
        LoadError::Slice(e)
    }
}

impl From<RegistryError> for LoadError {
    fn from(e: RegistryError) -> Self {
        LoadError::Registry(e)
    }
}

/// Errors produced by registry mutation. `SlugCollision` and `UnknownSerial` are the two
/// structural failure modes; `InvalidRename` covers self-rename attempts, slug validation
/// failures, and anything else `rename_*` chooses to reject in the future.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    /// A `register_*` or `rename_*` operation asked for a slug already in use by a different
    /// serial within the same kind. The existing serial is returned so the caller can decide
    /// whether to reuse it.
    SlugCollision {
        kind: AssetKind,
        slug: Slug,
        existing: u32,
    },
    /// A lookup or mutation referenced a serial that has no entry (deleted or never allocated).
    UnknownSerial { kind: AssetKind, serial: u32 },
    /// `rename_*` rejected the requested slug for reasons other than collision (e.g., empty).
    /// The string is human-readable explanation; the structured causes are upstream
    /// (`InvalidSlug` from wok-scene) but flattened here because the rename API takes a
    /// pre-validated `Slug` and the remaining rejection cases are semantic ("renaming to the
    /// same slug" is the obvious example).
    InvalidRename(String),
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegistryError::SlugCollision {
                kind,
                slug,
                existing,
            } => write!(
                f,
                "{kind} slug {slug:?} already used by serial {existing}"
            ),
            RegistryError::UnknownSerial { kind, serial } => {
                write!(f, "{kind} serial {serial} not present in registry")
            }
            RegistryError::InvalidRename(reason) => {
                write!(f, "invalid rename: {reason}")
            }
        }
    }
}

impl std::error::Error for RegistryError {}

/// Errors produced by `Registry::save` and serializer paths.
#[derive(Debug)]
pub enum SaveError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    Encode {
        source: pantry::serde_json::Error,
    },
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveError::Io { path, source } => {
                write!(f, "I/O error writing {}: {source}", path.display())
            }
            SaveError::Encode { source } => write!(f, "encode error: {source}"),
        }
    }
}

impl std::error::Error for SaveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SaveError::Io { source, .. } => Some(source),
            SaveError::Encode { source } => Some(source),
        }
    }
}

/// Errors produced by snapshot capture and restore. Phase A does not implement snapshots; the
/// variants exist so the public surface is stable when Phase D lands. `ChunkLoadFailed` wraps
/// the underlying `LoadError` in a `Box` to keep `SnapshotError` from growing past a small
/// fixed footprint.
#[derive(Debug)]
pub enum SnapshotError {
    UnsupportedVersion(u32),
    SchemaMismatch { snapshot: u64, expected: u64 },
    SceneMismatch {
        snapshot: SceneId,
        loaded: Option<SceneId>,
    },
    ChunkLoadFailed {
        coord: ChunkCoord,
        error: Box<LoadError>,
    },
    Parse(String),
}

impl std::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SnapshotError::UnsupportedVersion(v) => {
                write!(f, "unsupported snapshot _format {v}")
            }
            SnapshotError::SchemaMismatch { snapshot, expected } => {
                write!(
                    f,
                    "snapshot schema_hash {snapshot:#x} does not match engine build {expected:#x}"
                )
            }
            SnapshotError::SceneMismatch { snapshot, loaded } => match loaded {
                Some(l) => write!(
                    f,
                    "snapshot scene_id {} does not match loaded scene {}",
                    snapshot.0, l.0
                ),
                None => write!(
                    f,
                    "snapshot scene_id {} cannot be restored: no scene loaded",
                    snapshot.0
                ),
            },
            SnapshotError::ChunkLoadFailed { coord, error } => write!(
                f,
                "chunk ({}, {}) failed to load during restore: {error}",
                coord.x, coord.z
            ),
            SnapshotError::Parse(s) => write!(f, "snapshot parse error: {s}"),
        }
    }
}

impl std::error::Error for SnapshotError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SnapshotError::ChunkLoadFailed { error, .. } => Some(error.as_ref()),
            _ => None,
        }
    }
}

/// Errors produced by `ContentSystem::transition_chunk`. `NotResident` exists because runtime
/// eagerness only exists once a slot reaches Resident; the game must wait for
/// `ContentEvent::ChunkResident` before calling `transition_chunk`. The slot's current state
/// is returned as a string label so the variant stays `Clone`-and-`PartialEq`-friendly without
/// dragging the full `SlotState` (which carries non-clonable GPU handles) into the error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionError {
    UnknownSlot(ChunkCoord),
    NotResident {
        coord: ChunkCoord,
        state_label: &'static str,
    },
}

impl std::fmt::Display for TransitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransitionError::UnknownSlot(coord) => {
                write!(f, "no slot for chunk ({}, {})", coord.x, coord.z)
            }
            TransitionError::NotResident { coord, state_label } => write!(
                f,
                "chunk ({}, {}) is in state {state_label}, transition requires Resident",
                coord.x, coord.z
            ),
        }
    }
}

impl std::error::Error for TransitionError {}
