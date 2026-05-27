//! Registry: identity table for engine assets. Two halves (per plan section 1.2):
//!
//! - **Identity half** - serial allocation, slug map, usage tracking. Mutable at runtime;
//!   round-trips through `registry.json` on disk.
//! - **Storage half** - loaded GPU buffers, CPU mesh data. Lives in `ContentSystem.meshes`
//!   and on `ResidentChunk.gpu`; not stored in this module.
//!
//! Every mutation flows through `mutate`, which rebuilds the `Arc<RegistryReadView>` so
//! in-flight workers see the pre-mutation state and subsequent workers see the new state.

use std::path::PathBuf;
use std::sync::Arc;

use wok_scene::{AnimationId, AudioCueId, LightStateRef, MeshId, ShapePrimitive, Slug, VoiceLineId};

pub mod alloc;
pub mod entry;
pub mod populate;
pub mod rename;
pub mod serde;
pub mod view;

pub use alloc::{
    AnimationSerial, AudioSerial, EntrySlot, KindTable, LightSerial, MeshSerial, VoiceSerial,
};
pub use entry::{
    AnimationEntry, AssetStatus, AudioEntry, LightEntry, MeshEntry, UsageSite, VoiceEntry,
};
pub use view::{
    AnimationReadEntry, AudioReadEntry, LightReadEntry, MeshReadEntry, RegistryReadView,
    VoiceReadEntry,
};

pub use crate::error::AssetKind;

use crate::error::{LoadError, RegistryError};

/// The registry. Five typed `KindTable`s plus an `Arc<RegistryReadView>` rebuilt on every
/// mutation. The view-rebuild discipline lives entirely inside `mutate`; public mutators
/// route through it so the read view never drifts from live state.
#[derive(Debug)]
pub struct Registry {
    meshes: KindTable<MeshSerial, MeshEntry>,
    audio: KindTable<AudioSerial, AudioEntry>,
    animations: KindTable<AnimationSerial, AnimationEntry>,
    voice: KindTable<VoiceSerial, VoiceEntry>,
    light_states: KindTable<LightSerial, LightEntry>,
    view: Arc<RegistryReadView>,
}

impl Default for Registry {
    fn default() -> Self {
        Self::empty()
    }
}

impl Registry {
    pub fn empty() -> Self {
        let mut reg = Registry {
            meshes: KindTable::new(),
            audio: KindTable::new(),
            animations: KindTable::new(),
            voice: KindTable::new(),
            light_states: KindTable::new(),
            view: Arc::new(RegistryReadView {
                meshes: Vec::new(),
                audio: Vec::new(),
                animations: Vec::new(),
                voice: Vec::new(),
                light_states: Vec::new(),
            }),
        };
        reg.rebuild_view();
        reg
    }

    // ---- Lookup ----

    pub fn mesh(&self, id: &MeshId) -> Option<&MeshEntry> {
        self.meshes.get(id.serial())
    }

    pub fn audio(&self, id: &AudioCueId) -> Option<&AudioEntry> {
        self.audio.get(id.serial())
    }

    pub fn animation(&self, id: &AnimationId) -> Option<&AnimationEntry> {
        self.animations.get(id.serial())
    }

    pub fn voice(&self, id: &VoiceLineId) -> Option<&VoiceEntry> {
        self.voice.get(id.serial())
    }

    pub fn light_state(&self, id: &LightStateRef) -> Option<&LightEntry> {
        self.light_states.get(id.serial())
    }

    pub fn mesh_by_slug(&self, slug: &Slug) -> Option<MeshId> {
        self.meshes
            .by_slug(slug)
            .map(|serial| MeshId::new(slug.clone(), serial))
    }

    pub fn audio_by_slug(&self, slug: &Slug) -> Option<AudioCueId> {
        self.audio
            .by_slug(slug)
            .map(|serial| AudioCueId::new(slug.clone(), serial))
    }

    pub fn animation_by_slug(&self, slug: &Slug) -> Option<AnimationId> {
        self.animations
            .by_slug(slug)
            .map(|serial| AnimationId::new(slug.clone(), serial))
    }

    pub fn voice_by_slug(&self, slug: &Slug) -> Option<VoiceLineId> {
        self.voice
            .by_slug(slug)
            .map(|serial| VoiceLineId::new(slug.clone(), serial))
    }

    pub fn light_state_by_slug(&self, slug: &Slug) -> Option<LightStateRef> {
        self.light_states
            .by_slug(slug)
            .map(|serial| LightStateRef::new(slug.clone(), serial))
    }

    // ---- Registration ----

    /// Allocate a new mesh entry. `primitive` is `Some` for procedural placeholders and
    /// `None` for shipped meshes; the latter needs a `source_path` set via
    /// `set_mesh_source` after registration. The status follows from `primitive`: `Some`
    /// implies `Placeholder`, `None` implies `Shipped` (caller's responsibility to upgrade
    /// the entry once the source exists).
    pub fn register_mesh(
        &mut self,
        slug: Slug,
        primitive: Option<ShapePrimitive>,
    ) -> Result<MeshId, RegistryError> {
        let status = if primitive.is_some() {
            AssetStatus::Placeholder
        } else {
            AssetStatus::Shipped
        };
        let entry = MeshEntry {
            slug: slug.clone(),
            status,
            primitive,
            source_path: None,
            usage: Vec::new(),
        };
        let serial = self.mutate(|r| r.meshes.alloc(AssetKind::Mesh, entry))?;
        Ok(MeshId::new(slug, serial))
    }

    /// Allocate a new audio cue entry pointing at a shipped source path.
    pub fn register_audio(
        &mut self,
        slug: Slug,
        source: PathBuf,
    ) -> Result<AudioCueId, RegistryError> {
        let entry = AudioEntry {
            slug: slug.clone(),
            status: AssetStatus::Shipped,
            source_path: Some(source),
            usage: Vec::new(),
        };
        let serial = self.mutate(|r| r.audio.alloc(AssetKind::Audio, entry))?;
        Ok(AudioCueId::new(slug, serial))
    }

    /// Allocate a new animation entry pointing at a shipped source path.
    pub fn register_animation(
        &mut self,
        slug: Slug,
        source: PathBuf,
    ) -> Result<AnimationId, RegistryError> {
        let entry = AnimationEntry {
            slug: slug.clone(),
            status: AssetStatus::Shipped,
            source_path: Some(source),
            usage: Vec::new(),
        };
        let serial = self.mutate(|r| r.animations.alloc(AssetKind::Animation, entry))?;
        Ok(AnimationId::new(slug, serial))
    }

    pub fn register_voice(
        &mut self,
        slug: Slug,
        source: PathBuf,
    ) -> Result<VoiceLineId, RegistryError> {
        let entry = VoiceEntry {
            slug: slug.clone(),
            status: AssetStatus::Shipped,
            source_path: Some(source),
            usage: Vec::new(),
        };
        let serial = self.mutate(|r| r.voice.alloc(AssetKind::Voice, entry))?;
        Ok(VoiceLineId::new(slug, serial))
    }

    pub fn register_light_state(
        &mut self,
        slug: Slug,
        source: PathBuf,
    ) -> Result<LightStateRef, RegistryError> {
        let entry = LightEntry {
            slug: slug.clone(),
            status: AssetStatus::Shipped,
            source_path: Some(source),
            usage: Vec::new(),
        };
        let serial = self.mutate(|r| r.light_states.alloc(AssetKind::LightState, entry))?;
        Ok(LightStateRef::new(slug, serial))
    }

    // ---- Placeholder registration (used by populate for unknown slugs) ----

    /// Register a placeholder audio entry with no source path. Used by population when a
    /// scene references an unknown audio slug. The entry is upgraded to `Shipped` later by
    /// `set_audio_source` (or its equivalent).
    pub fn register_audio_placeholder(&mut self, slug: Slug) -> Result<AudioCueId, RegistryError> {
        let entry = AudioEntry {
            slug: slug.clone(),
            status: AssetStatus::Placeholder,
            source_path: None,
            usage: Vec::new(),
        };
        let serial = self.mutate(|r| r.audio.alloc(AssetKind::Audio, entry))?;
        Ok(AudioCueId::new(slug, serial))
    }

    pub fn register_animation_placeholder(
        &mut self,
        slug: Slug,
    ) -> Result<AnimationId, RegistryError> {
        let entry = AnimationEntry {
            slug: slug.clone(),
            status: AssetStatus::Placeholder,
            source_path: None,
            usage: Vec::new(),
        };
        let serial = self.mutate(|r| r.animations.alloc(AssetKind::Animation, entry))?;
        Ok(AnimationId::new(slug, serial))
    }

    pub fn register_voice_placeholder(&mut self, slug: Slug) -> Result<VoiceLineId, RegistryError> {
        let entry = VoiceEntry {
            slug: slug.clone(),
            status: AssetStatus::Placeholder,
            source_path: None,
            usage: Vec::new(),
        };
        let serial = self.mutate(|r| r.voice.alloc(AssetKind::Voice, entry))?;
        Ok(VoiceLineId::new(slug, serial))
    }

    pub fn register_light_state_placeholder(
        &mut self,
        slug: Slug,
    ) -> Result<LightStateRef, RegistryError> {
        let entry = LightEntry {
            slug: slug.clone(),
            status: AssetStatus::Placeholder,
            source_path: None,
            usage: Vec::new(),
        };
        let serial = self.mutate(|r| r.light_states.alloc(AssetKind::LightState, entry))?;
        Ok(LightStateRef::new(slug, serial))
    }

    // ---- Mutation: status upgrade ----

    /// Upgrade a placeholder mesh entry to shipped. Phase 4 does not exercise this (the
    /// only meshes are procedural placeholders); included for the on-disk format's stability.
    pub fn set_mesh_source(&mut self, id: &MeshId, source: PathBuf) -> Result<(), RegistryError> {
        self.mutate(|r| {
            let entry = r
                .meshes
                .get_mut(id.serial())
                .ok_or(RegistryError::UnknownSerial {
                    kind: AssetKind::Mesh,
                    serial: id.serial(),
                })?;
            entry.status = AssetStatus::Shipped;
            entry.source_path = Some(source);
            Ok(())
        })
    }

    // ---- Read view ----

    pub fn read_view(&self) -> Arc<RegistryReadView> {
        Arc::clone(&self.view)
    }

    // ---- Mutation discipline ----

    /// All public mutators route here. Invokes the closure, then rebuilds the read view
    /// from the post-mutation state. Plan section 9.3: "All public mutators go through
    /// `mutate`. The read view stays in lockstep."
    pub(crate) fn mutate<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        let r = f(self);
        self.rebuild_view();
        r
    }

    pub(crate) fn rebuild_view(&mut self) {
        // SAFETY note: `view::build` takes slices from the inner tables. We pull them
        // ahead of time so we don't borrow `self` mutably and immutably at once.
        let meshes = self.meshes_entries();
        let audio = self.audio_entries();
        let animations = self.animations_entries();
        let voice = self.voice_entries();
        let light_states = self.light_entries();
        self.view = view::build(&meshes, &audio, &animations, &voice, &light_states);
    }

    // ---- Internal accessors used by the serde module ----

    pub(crate) fn iter_mesh_slots(&self) -> impl Iterator<Item = (u32, &EntrySlot<MeshEntry>)> {
        self.meshes.iter_slots()
    }
    pub(crate) fn iter_audio_slots(&self) -> impl Iterator<Item = (u32, &EntrySlot<AudioEntry>)> {
        self.audio.iter_slots()
    }
    pub(crate) fn iter_animation_slots(
        &self,
    ) -> impl Iterator<Item = (u32, &EntrySlot<AnimationEntry>)> {
        self.animations.iter_slots()
    }
    pub(crate) fn iter_voice_slots(&self) -> impl Iterator<Item = (u32, &EntrySlot<VoiceEntry>)> {
        self.voice.iter_slots()
    }
    pub(crate) fn iter_light_slots(&self) -> impl Iterator<Item = (u32, &EntrySlot<LightEntry>)> {
        self.light_states.iter_slots()
    }

    pub(crate) fn mesh_next_serial(&self) -> u32 {
        self.meshes.next_serial()
    }
    pub(crate) fn audio_next_serial(&self) -> u32 {
        self.audio.next_serial()
    }
    pub(crate) fn animation_next_serial(&self) -> u32 {
        self.animations.next_serial()
    }
    pub(crate) fn voice_next_serial(&self) -> u32 {
        self.voice.next_serial()
    }
    pub(crate) fn light_next_serial(&self) -> u32 {
        self.light_states.next_serial()
    }

    pub(crate) fn insert_mesh_at(&mut self, serial: u32, entry: MeshEntry) -> Result<(), LoadError> {
        self.meshes
            .insert_at(AssetKind::Mesh, serial, entry)
            .map_err(LoadError::Registry)
    }
    pub(crate) fn insert_mesh_tombstone(&mut self, serial: u32) {
        self.meshes.insert_tombstone(serial);
    }
    pub(crate) fn set_mesh_next_serial(&mut self, next: u32) {
        self.meshes.set_next_serial(next);
    }

    pub(crate) fn insert_audio_at(
        &mut self,
        serial: u32,
        entry: AudioEntry,
    ) -> Result<(), LoadError> {
        self.audio
            .insert_at(AssetKind::Audio, serial, entry)
            .map_err(LoadError::Registry)
    }
    pub(crate) fn insert_audio_tombstone(&mut self, serial: u32) {
        self.audio.insert_tombstone(serial);
    }
    pub(crate) fn set_audio_next_serial(&mut self, next: u32) {
        self.audio.set_next_serial(next);
    }

    pub(crate) fn insert_animation_at(
        &mut self,
        serial: u32,
        entry: AnimationEntry,
    ) -> Result<(), LoadError> {
        self.animations
            .insert_at(AssetKind::Animation, serial, entry)
            .map_err(LoadError::Registry)
    }
    pub(crate) fn insert_animation_tombstone(&mut self, serial: u32) {
        self.animations.insert_tombstone(serial);
    }
    pub(crate) fn set_animation_next_serial(&mut self, next: u32) {
        self.animations.set_next_serial(next);
    }

    pub(crate) fn insert_voice_at(
        &mut self,
        serial: u32,
        entry: VoiceEntry,
    ) -> Result<(), LoadError> {
        self.voice
            .insert_at(AssetKind::Voice, serial, entry)
            .map_err(LoadError::Registry)
    }
    pub(crate) fn insert_voice_tombstone(&mut self, serial: u32) {
        self.voice.insert_tombstone(serial);
    }
    pub(crate) fn set_voice_next_serial(&mut self, next: u32) {
        self.voice.set_next_serial(next);
    }

    pub(crate) fn insert_light_at(
        &mut self,
        serial: u32,
        entry: LightEntry,
    ) -> Result<(), LoadError> {
        self.light_states
            .insert_at(AssetKind::LightState, serial, entry)
            .map_err(LoadError::Registry)
    }
    pub(crate) fn insert_light_tombstone(&mut self, serial: u32) {
        self.light_states.insert_tombstone(serial);
    }
    pub(crate) fn set_light_next_serial(&mut self, next: u32) {
        self.light_states.set_next_serial(next);
    }

    fn meshes_entries(&self) -> Vec<EntrySlot<MeshEntry>> {
        self.meshes
            .iter_slots()
            .map(|(_, slot)| slot.clone())
            .collect()
    }
    fn audio_entries(&self) -> Vec<EntrySlot<AudioEntry>> {
        self.audio
            .iter_slots()
            .map(|(_, slot)| slot.clone())
            .collect()
    }
    fn animations_entries(&self) -> Vec<EntrySlot<AnimationEntry>> {
        self.animations
            .iter_slots()
            .map(|(_, slot)| slot.clone())
            .collect()
    }
    fn voice_entries(&self) -> Vec<EntrySlot<VoiceEntry>> {
        self.voice
            .iter_slots()
            .map(|(_, slot)| slot.clone())
            .collect()
    }
    fn light_entries(&self) -> Vec<EntrySlot<LightEntry>> {
        self.light_states
            .iter_slots()
            .map(|(_, slot)| slot.clone())
            .collect()
    }

    // ---- Populate access points (used by populate.rs in step 5) ----

    #[allow(dead_code)]
    pub(crate) fn meshes_table_mut(&mut self) -> &mut KindTable<MeshSerial, MeshEntry> {
        &mut self.meshes
    }
    #[allow(dead_code)]
    pub(crate) fn audio_table_mut(&mut self) -> &mut KindTable<AudioSerial, AudioEntry> {
        &mut self.audio
    }
    #[allow(dead_code)]
    pub(crate) fn animations_table_mut(
        &mut self,
    ) -> &mut KindTable<AnimationSerial, AnimationEntry> {
        &mut self.animations
    }
    #[allow(dead_code)]
    pub(crate) fn voice_table_mut(&mut self) -> &mut KindTable<VoiceSerial, VoiceEntry> {
        &mut self.voice
    }
    #[allow(dead_code)]
    pub(crate) fn light_states_table_mut(
        &mut self,
    ) -> &mut KindTable<LightSerial, LightEntry> {
        &mut self.light_states
    }
}
