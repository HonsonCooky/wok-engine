//! Auto-population from a loaded scene. Walks each prefab's states, each chunk's
//! `light_state` and region markers, and the scene manifest's `default_light_state`,
//! recording one `UsageSite` per reference at the matching registry entry.
//!
//! See plan section 3.3 ("Auto-population (called after scene/prefab load)") and test
//! section 7.1 #6-#8 for the contract. The walk is idempotent: every entry's `usage`
//! vector is cleared before walking, so re-populating produces the same lists (test #7).
//!
//! **Unknown-slug policy**: when an authored reference names a slug that isn't in the
//! registry, populate creates a placeholder entry with that slug (test #8). The serial
//! the authored token quoted is **not** reused - populate allocates a fresh next-serial
//! via `register_*_placeholder`. The slug-keyed lookup means subsequent re-populates find
//! the same entry and record at it. The authored token's old serial is debug surface
//! only (per wok-scene §9.1 and plan §5.4 stale-slug rule); a future save through the
//! editor would re-emit the reference with the new serial.

use std::collections::HashMap;

use wok_scene::{
    AudioCueId, Chunk, ChunkCoord, LightStateRef, MeshId, Prefab, PrefabId, RegionPurpose,
    Scene, Slug,
};

use crate::error::{AssetKind, RegistryError};
use crate::registry::Registry;
use crate::registry::entry::UsageSite;

impl Registry {
    /// Walk a scene + its prefabs + its chunks, recording asset usage at each registry
    /// entry that is referenced. Unknown slugs are added as placeholders. Existing usage
    /// records are cleared before the walk so the result is the canonical usage map for
    /// this snapshot of authored data.
    ///
    /// Returns `RegistryError` only if a placeholder registration collides at the allocator
    /// level. Phase A's walk never produces that condition (placeholders are registered
    /// only for slugs not already in the table), so callers treat the error as a structural
    /// bug rather than a recoverable failure.
    pub fn populate_from_scene(
        &mut self,
        scene: &Scene,
        prefabs: &HashMap<PrefabId, Prefab>,
        chunks: &HashMap<ChunkCoord, Chunk>,
    ) -> Result<(), RegistryError> {
        self.mutate(|r| {
            r.meshes_table_mut().clear_all_usage();
            r.audio_table_mut().clear_all_usage();
            r.animations_table_mut().clear_all_usage();
            r.voice_table_mut().clear_all_usage();
            r.light_states_table_mut().clear_all_usage();

            // Scene-level default_light_state.
            let site = UsageSite::SceneDefaultLightState {
                scene: scene.id.clone(),
            };
            ensure_light_state_usage(r, &scene.default_light_state, site)?;

            // Prefabs: walk each state's mesh_override and audio_cues.
            for prefab in prefabs.values() {
                for state in &prefab.states {
                    if let Some(mesh) = &state.mesh_override {
                        let site = UsageSite::PrefabState {
                            prefab: prefab.id.clone(),
                            state: state.name.clone(),
                        };
                        ensure_mesh_usage(r, mesh, site)?;
                    }
                    for (cue_name, cue_id) in &state.audio_cues {
                        let site = UsageSite::PrefabAudioCue {
                            prefab: prefab.id.clone(),
                            state: state.name.clone(),
                            cue_name: cue_name.clone(),
                        };
                        ensure_audio_usage(r, cue_id, site)?;
                    }
                }
            }

            // Chunks: light_state and any region with Lighting purpose.
            for chunk in chunks.values() {
                let site = UsageSite::ChunkLightState {
                    scene: scene.id.clone(),
                    coord: chunk.coord,
                };
                ensure_light_state_usage(r, &chunk.light_state, site)?;
                for region in &chunk.regions {
                    if let RegionPurpose::Lighting { state } = &region.purpose {
                        let site = UsageSite::ChunkRegion {
                            scene: scene.id.clone(),
                            coord: chunk.coord,
                            region_name: region.name.clone(),
                        };
                        ensure_light_state_usage(r, state, site)?;
                    }
                }
            }
            Ok(())
        })
    }
}

/// Look up the mesh entry for `id`; if not present by slug, register a placeholder with
/// that slug. Push the usage site onto the resolved entry.
fn ensure_mesh_usage(
    reg: &mut Registry,
    id: &MeshId,
    site: UsageSite,
) -> Result<(), RegistryError> {
    // Lookup-or-create by slug. The authored file's serial is debug surface; the registry
    // assigns a fresh next-serial when populating an unknown slug.
    let serial = if let Some(existing) = reg.mesh_by_slug(id.slug()) {
        existing.serial()
    } else {
        reg.register_placeholder_mesh_internal(id.slug().clone())?
            .serial()
    };
    let entry = reg
        .meshes_table_mut()
        .get_mut(serial)
        .expect("mesh entry just registered or found by slug must be live");
    entry.usage.push(site);
    Ok(())
}

fn ensure_audio_usage(
    reg: &mut Registry,
    id: &AudioCueId,
    site: UsageSite,
) -> Result<(), RegistryError> {
    let serial = if let Some(existing) = reg.audio_by_slug(id.slug()) {
        existing.serial()
    } else {
        reg.register_placeholder_audio_internal(id.slug().clone())?
            .serial()
    };
    let entry = reg
        .audio_table_mut()
        .get_mut(serial)
        .expect("audio entry just registered or found by slug must be live");
    entry.usage.push(site);
    Ok(())
}

fn ensure_light_state_usage(
    reg: &mut Registry,
    id: &LightStateRef,
    site: UsageSite,
) -> Result<(), RegistryError> {
    let serial = if let Some(existing) = reg.light_state_by_slug(id.slug()) {
        existing.serial()
    } else {
        reg.register_placeholder_light_internal(id.slug().clone())?
            .serial()
    };
    let entry = reg
        .light_states_table_mut()
        .get_mut(serial)
        .expect("light entry just registered or found by slug must be live");
    entry.usage.push(site);
    Ok(())
}

// ---- Internal placeholder registration -------------------------------------------------
//
// The public `register_*_placeholder` methods route through `mutate`, which clears and
// rebuilds the read view. populate is already inside `mutate`; nested `mutate` would clear
// the view twice per call. The internal variants skip the view rebuild and produce raw
// IDs.

impl Registry {
    fn register_placeholder_mesh_internal(
        &mut self,
        slug: Slug,
    ) -> Result<MeshId, RegistryError> {
        let entry = crate::registry::entry::MeshEntry {
            slug: slug.clone(),
            status: crate::registry::entry::AssetStatus::Placeholder,
            primitive: None,
            source_path: None,
            usage: Vec::new(),
        };
        let serial = self.meshes_table_mut().alloc(AssetKind::Mesh, entry)?;
        Ok(MeshId::new(slug, serial))
    }

    fn register_placeholder_audio_internal(
        &mut self,
        slug: Slug,
    ) -> Result<AudioCueId, RegistryError> {
        let entry = crate::registry::entry::AudioEntry {
            slug: slug.clone(),
            status: crate::registry::entry::AssetStatus::Placeholder,
            source_path: None,
            usage: Vec::new(),
        };
        let serial = self.audio_table_mut().alloc(AssetKind::Audio, entry)?;
        Ok(AudioCueId::new(slug, serial))
    }

    fn register_placeholder_light_internal(
        &mut self,
        slug: Slug,
    ) -> Result<LightStateRef, RegistryError> {
        let entry = crate::registry::entry::LightEntry {
            slug: slug.clone(),
            status: crate::registry::entry::AssetStatus::Placeholder,
            source_path: None,
            usage: Vec::new(),
        };
        let serial = self
            .light_states_table_mut()
            .alloc(AssetKind::LightState, entry)?;
        Ok(LightStateRef::new(slug, serial))
    }
}
