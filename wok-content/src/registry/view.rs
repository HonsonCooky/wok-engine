//! `RegistryReadView` - an immutable, cheap-to-`Arc`-clone snapshot of the registry tables
//! that workers consume. See plan section 9.3 for the lock-free reader pattern. The view
//! holds enough data for the worker pipeline:
//!
//! - mesh primitive (procedural placeholders) and source path (shipped),
//! - audio / animation / voice source path,
//! - light state source path,
//!
//! plus the slug for diagnostics. `UsageSite` lists are deliberately omitted: workers do
//! not need usage information. Phase A's `LoopbackWorker` runs synchronously and could
//! borrow the live registry instead, but the `Arc<RegistryReadView>` shape is wired in
//! from step 2 so Phase B does not need to retrofit the worker contract.

use std::path::PathBuf;
use std::sync::Arc;

use wok_scene::{ShapePrimitive, Slug};

use crate::registry::alloc::EntrySlot;
use crate::registry::entry::{AnimationEntry, AssetStatus, AudioEntry, LightEntry, MeshEntry, VoiceEntry};

/// Slim mesh entry carried in the read view. Omits the `UsageSite` list (worker does not
/// need it); keeps the primitive and source path (worker needs at least one).
#[derive(Debug, Clone, PartialEq)]
pub struct MeshReadEntry {
    pub slug: Slug,
    pub status: AssetStatus,
    pub primitive: Option<ShapePrimitive>,
    pub source_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AudioReadEntry {
    pub slug: Slug,
    pub status: AssetStatus,
    pub source_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AnimationReadEntry {
    pub slug: Slug,
    pub status: AssetStatus,
    pub source_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VoiceReadEntry {
    pub slug: Slug,
    pub status: AssetStatus,
    pub source_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LightReadEntry {
    pub slug: Slug,
    pub status: AssetStatus,
    pub source_path: Option<PathBuf>,
}

/// Read-only registry view. `Arc`-cloned by workers; rebuilt by the registry on every
/// mutation so existing workers see the pre-mutation state.
#[derive(Debug, Clone, PartialEq)]
pub struct RegistryReadView {
    pub meshes: Vec<Option<MeshReadEntry>>,
    pub audio: Vec<Option<AudioReadEntry>>,
    pub animations: Vec<Option<AnimationReadEntry>>,
    pub voice: Vec<Option<VoiceReadEntry>>,
    pub light_states: Vec<Option<LightReadEntry>>,
}

impl RegistryReadView {
    /// Look up a mesh by serial. Returns `None` for empty slots, tombstones, and
    /// out-of-range serials. The view never lies: if it answers `Some`, the entry is live.
    pub fn mesh(&self, serial: u32) -> Option<&MeshReadEntry> {
        self.meshes.get(serial as usize).and_then(Option::as_ref)
    }

    pub fn audio(&self, serial: u32) -> Option<&AudioReadEntry> {
        self.audio.get(serial as usize).and_then(Option::as_ref)
    }

    pub fn animation(&self, serial: u32) -> Option<&AnimationReadEntry> {
        self.animations
            .get(serial as usize)
            .and_then(Option::as_ref)
    }

    pub fn voice(&self, serial: u32) -> Option<&VoiceReadEntry> {
        self.voice.get(serial as usize).and_then(Option::as_ref)
    }

    pub fn light_state(&self, serial: u32) -> Option<&LightReadEntry> {
        self.light_states
            .get(serial as usize)
            .and_then(Option::as_ref)
    }
}

/// Build the read view from the live tables. Internal helper called by `Registry::mutate`.
pub(crate) fn build(
    meshes: &[EntrySlot<MeshEntry>],
    audio: &[EntrySlot<AudioEntry>],
    animations: &[EntrySlot<AnimationEntry>],
    voice: &[EntrySlot<VoiceEntry>],
    light_states: &[EntrySlot<LightEntry>],
) -> Arc<RegistryReadView> {
    Arc::new(RegistryReadView {
        meshes: meshes.iter().map(slot_to_mesh).collect(),
        audio: audio.iter().map(slot_to_audio).collect(),
        animations: animations.iter().map(slot_to_animation).collect(),
        voice: voice.iter().map(slot_to_voice).collect(),
        light_states: light_states.iter().map(slot_to_light).collect(),
    })
}

fn slot_to_mesh(slot: &EntrySlot<MeshEntry>) -> Option<MeshReadEntry> {
    match slot {
        EntrySlot::Live(e) => Some(MeshReadEntry {
            slug: e.slug.clone(),
            status: e.status.clone(),
            primitive: e.primitive,
            source_path: e.source_path.clone(),
        }),
        _ => None,
    }
}

fn slot_to_audio(slot: &EntrySlot<AudioEntry>) -> Option<AudioReadEntry> {
    match slot {
        EntrySlot::Live(e) => Some(AudioReadEntry {
            slug: e.slug.clone(),
            status: e.status.clone(),
            source_path: e.source_path.clone(),
        }),
        _ => None,
    }
}

fn slot_to_animation(slot: &EntrySlot<AnimationEntry>) -> Option<AnimationReadEntry> {
    match slot {
        EntrySlot::Live(e) => Some(AnimationReadEntry {
            slug: e.slug.clone(),
            status: e.status.clone(),
            source_path: e.source_path.clone(),
        }),
        _ => None,
    }
}

fn slot_to_voice(slot: &EntrySlot<VoiceEntry>) -> Option<VoiceReadEntry> {
    match slot {
        EntrySlot::Live(e) => Some(VoiceReadEntry {
            slug: e.slug.clone(),
            status: e.status.clone(),
            source_path: e.source_path.clone(),
        }),
        _ => None,
    }
}

fn slot_to_light(slot: &EntrySlot<LightEntry>) -> Option<LightReadEntry> {
    match slot {
        EntrySlot::Live(e) => Some(LightReadEntry {
            slug: e.slug.clone(),
            status: e.status.clone(),
            source_path: e.source_path.clone(),
        }),
        _ => None,
    }
}
