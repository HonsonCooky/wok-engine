//! Per-asset-kind registry entries. Each entry pairs an authored slug, a status (placeholder
//! vs shipped), the source of the data (a procedural primitive for placeholder meshes, or a
//! source-file path for shipped assets), and a list of usage sites discovered by registry
//! population. Tombstones are not entries; they sit in the kind table as `None` slots with a
//! companion deletion record (see `KindTable::tombstones` in `mod.rs`).

use std::path::PathBuf;

use wok_scene::{ChunkCoord, PrefabId, SceneId, ShapePrimitive, Slug};

/// Asset status: where the data behind a serial comes from. Placeholder assets are
/// procedural (primitives baked at registry-build time, default audio cues, etc.); shipped
/// assets reference a source file on disk. Phase 4 only exercises placeholder meshes; the
/// shipped variant exists so the registry's on-disk format does not need to be bumped when
/// shipped assets arrive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetStatus {
    Placeholder,
    Shipped,
}

/// A site in authored data where an asset is referenced. Populated by
/// `Registry::populate_from_scene`. Variants name the kind of reference so the editor can
/// jump straight to the field that holds it; the `ChunkRegion` variant carries the scene id
/// so a multi-scene editor can resolve which scene's chunk the usage points at.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum UsageSite {
    /// Prefab state-level override (currently only meshes, via `PrefabState::mesh_override`).
    PrefabState {
        prefab: PrefabId,
        state: String,
    },
    /// Per-shape reference inside a prefab state. The shape index is the index inside
    /// `PrefabState::shapes`. Unused in Phase A (shape primitives are inline, not referenced
    /// by id) but kept on the type so future shape-via-id references don't need a variant
    /// extension.
    PrefabShape {
        prefab: PrefabId,
        state: String,
        shape_index: u32,
    },
    /// Per-prefab-state audio cue reference (the `BTreeMap<String, AudioCueId>` slot). The
    /// cue name is the map key.
    PrefabAudioCue {
        prefab: PrefabId,
        state: String,
        cue_name: String,
    },
    /// A chunk-level region marker referenced this asset (currently only LightStateRef via
    /// `RegionPurpose::Lighting`). The region name identifies which marker.
    ChunkRegion {
        scene: SceneId,
        coord: ChunkCoord,
        region_name: String,
    },
    /// A chunk's `light_state` field references this LightStateRef.
    ChunkLightState {
        scene: SceneId,
        coord: ChunkCoord,
    },
}

/// Mesh registry entry. `primitive` is `Some` for procedurally-generated placeholders and
/// `None` for shipped meshes (which will carry a `source_path` instead). Phase 4 exercises
/// only placeholder meshes; shipped meshes round-trip through the on-disk format but the
/// `populate_from_scene` walk does not yet upgrade entries from placeholder to shipped.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshEntry {
    pub slug: Slug,
    pub status: AssetStatus,
    pub primitive: Option<ShapePrimitive>,
    pub source_path: Option<PathBuf>,
    pub usage: Vec<UsageSite>,
}

/// Audio cue registry entry. The registry tracks identity only; the actual audio buffer
/// lives in the future wok-audio crate (plan section 1.1, decisions index).
#[derive(Debug, Clone, PartialEq)]
pub struct AudioEntry {
    pub slug: Slug,
    pub status: AssetStatus,
    pub source_path: Option<PathBuf>,
    pub usage: Vec<UsageSite>,
}

/// Animation registry entry. wok-anim owns the pose data; we track identity only.
#[derive(Debug, Clone, PartialEq)]
pub struct AnimationEntry {
    pub slug: Slug,
    pub status: AssetStatus,
    pub source_path: Option<PathBuf>,
    pub usage: Vec<UsageSite>,
}

/// Voice line registry entry. Same pattern as audio: identity here, data in wok-audio.
#[derive(Debug, Clone, PartialEq)]
pub struct VoiceEntry {
    pub slug: Slug,
    pub status: AssetStatus,
    pub source_path: Option<PathBuf>,
    pub usage: Vec<UsageSite>,
}

/// Light state registry entry. wok-light owns the animation curves; we track identity only.
#[derive(Debug, Clone, PartialEq)]
pub struct LightEntry {
    pub slug: Slug,
    pub status: AssetStatus,
    pub source_path: Option<PathBuf>,
    pub usage: Vec<UsageSite>,
}

/// Trait that abstracts the per-kind entry types so generic `KindTable<E>` code can reach a
/// few common fields without enum dispatch. Implemented manually rather than derived so the
/// implementations stay narrow and the field names don't have to match across kinds.
pub trait RegistryEntry: Sized + Clone + std::fmt::Debug + PartialEq {
    fn slug(&self) -> &Slug;
    fn slug_mut(&mut self) -> &mut Slug;
    fn usage(&self) -> &[UsageSite];
    fn usage_mut(&mut self) -> &mut Vec<UsageSite>;
    fn clear_usage(&mut self) {
        self.usage_mut().clear();
    }
}

macro_rules! impl_registry_entry {
    ($ty:ty) => {
        impl RegistryEntry for $ty {
            fn slug(&self) -> &Slug {
                &self.slug
            }
            fn slug_mut(&mut self) -> &mut Slug {
                &mut self.slug
            }
            fn usage(&self) -> &[UsageSite] {
                &self.usage
            }
            fn usage_mut(&mut self) -> &mut Vec<UsageSite> {
                &mut self.usage
            }
        }
    };
}

impl_registry_entry!(MeshEntry);
impl_registry_entry!(AudioEntry);
impl_registry_entry!(AnimationEntry);
impl_registry_entry!(VoiceEntry);
impl_registry_entry!(LightEntry);
