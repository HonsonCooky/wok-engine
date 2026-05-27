//! Rename operations on the registry. The mechanics live in `KindTable::rename`; this module
//! is just the public entry-point glue that dispatches from `&MeshId` / `&AudioCueId` / etc.
//! to the typed table and rebuilds the read view.
//!
//! See plan section 5.4 - rename is atomic on the main thread, three statements (drop old
//! slug, insert new slug, write entry slug). The read view is rebuilt afterwards so
//! subsequent worker requests see the new slug; in-flight workers still hold the previous
//! `Arc<RegistryReadView>` and continue with the old data.
//!
//! Deviation from plan section 3.3: signatures take `&MeshId` etc. by reference rather than
//! by value. The IDs are `Clone` (cheap `Arc`-bump on the slug) but the rename body only
//! reads `id.serial()`; consuming the caller's `MeshId` adds nothing. Pass-by-reference also
//! matches the lookup signatures on `Registry::mesh(&MeshId)` so the rename and lookup APIs
//! line up.

use wok_scene::{AnimationId, AudioCueId, LightStateRef, MeshId, Slug, VoiceLineId};

use crate::error::{AssetKind, RegistryError};
use crate::registry::Registry;

impl Registry {
    pub fn rename_mesh(&mut self, id: &MeshId, new_slug: Slug) -> Result<(), RegistryError> {
        self.mutate(|r| r.meshes_table_mut().rename(AssetKind::Mesh, id.serial(), new_slug))
    }

    pub fn rename_audio(
        &mut self,
        id: &AudioCueId,
        new_slug: Slug,
    ) -> Result<(), RegistryError> {
        self.mutate(|r| r.audio_table_mut().rename(AssetKind::Audio, id.serial(), new_slug))
    }

    pub fn rename_animation(
        &mut self,
        id: &AnimationId,
        new_slug: Slug,
    ) -> Result<(), RegistryError> {
        self.mutate(|r| {
            r.animations_table_mut()
                .rename(AssetKind::Animation, id.serial(), new_slug)
        })
    }

    pub fn rename_voice(
        &mut self,
        id: &VoiceLineId,
        new_slug: Slug,
    ) -> Result<(), RegistryError> {
        self.mutate(|r| r.voice_table_mut().rename(AssetKind::Voice, id.serial(), new_slug))
    }

    pub fn rename_light_state(
        &mut self,
        id: &LightStateRef,
        new_slug: Slug,
    ) -> Result<(), RegistryError> {
        self.mutate(|r| {
            r.light_states_table_mut()
                .rename(AssetKind::LightState, id.serial(), new_slug)
        })
    }
}
