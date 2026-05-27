//! On-disk JSON format for `Registry`. See plan section 4.1 for the schema. Load is parse +
//! validate `_format` + insert entries by serial; save sorts entries by serial and emits one
//! object per slot (live or tombstone). Slots that were never allocated (`EntrySlot::Empty`)
//! are not emitted; gaps in the on-disk array implicitly recreate them on next load.
//!
//! The `_format` header lives outside the typed body (which uses `deny_unknown_fields` on
//! every inner object), so the parse-validate-strip pattern from wok-scene's `serde_format`
//! applies. Save uses an inline `_format` field at the top level.

use std::path::{Path, PathBuf};

use pantry::serde::{Deserialize, Serialize};
use pantry::serde_json;

use wok_scene::{ShapePrimitive, Slug};

use crate::error::{LoadError, SaveError};
use crate::registry::Registry;
use crate::registry::alloc::EntrySlot;
use crate::registry::entry::{
    AnimationEntry, AssetStatus, AudioEntry, LightEntry, MeshEntry, VoiceEntry,
};

/// The registry file's `_format` value. Bumped on any deliberate breaking change to the
/// on-disk format. Independent of wok-scene's `_format` (plan section 9.10).
pub const CURRENT_FORMAT: u32 = 1;

impl Registry {
    /// Load a registry from `registry.json`.
    pub fn load(path: &Path) -> Result<Self, LoadError> {
        let contents = std::fs::read_to_string(path).map_err(|source| LoadError::Io {
            path: path.to_owned(),
            source,
        })?;
        Self::from_json_str(&contents, path)
    }

    /// Save the registry to `registry.json`. Output is pretty-printed for human readability,
    /// matching wok-scene's policy (plan history "Saved JSON is pretty-printed").
    pub fn save(&self, path: &Path) -> Result<(), SaveError> {
        let body = self.to_file();
        let mut top = serde_json::Map::new();
        top.insert(
            "_format".to_string(),
            serde_json::Value::from(CURRENT_FORMAT),
        );
        let mut body_value = serde_json::to_value(&body).map_err(|source| SaveError::Encode {
            source,
        })?;
        if let serde_json::Value::Object(map) = &mut body_value {
            // Append body fields after `_format` so the header is first.
            for (k, v) in std::mem::take(map) {
                top.insert(k, v);
            }
        }
        let value = serde_json::Value::Object(top);
        let pretty = serde_json::to_string_pretty(&value).map_err(|source| SaveError::Encode {
            source,
        })?;
        std::fs::write(path, pretty).map_err(|source| SaveError::Io {
            path: path.to_owned(),
            source,
        })
    }

    /// Parse JSON text into a `Registry`. Exposed for tests; the `path` argument carries
    /// through to diagnostics in error variants.
    pub(crate) fn from_json_str(contents: &str, path: &Path) -> Result<Self, LoadError> {
        let mut value: serde_json::Value =
            serde_json::from_str(contents).map_err(|source| LoadError::Scene(
                wok_scene::LoadError::Parse {
                    path: path.to_owned(),
                    source,
                },
            ))?;
        let format = value
            .get("_format")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| {
                LoadError::Scene(wok_scene::LoadError::MissingFormat {
                    path: path.to_owned(),
                })
            })?;
        if format != u64::from(CURRENT_FORMAT) {
            return Err(LoadError::Scene(wok_scene::LoadError::UnsupportedVersion {
                path: path.to_owned(),
                found: format as u32,
            }));
        }
        if let serde_json::Value::Object(map) = &mut value {
            map.remove("_format");
        }
        let file: RegistryFile = serde_json::from_value(value).map_err(|source| {
            LoadError::Scene(wok_scene::LoadError::Parse {
                path: path.to_owned(),
                source,
            })
        })?;
        Self::from_file(file)
    }

    fn to_file(&self) -> RegistryFile {
        RegistryFile {
            meshes: mesh_section(self),
            audio: audio_section(self),
            animations: animation_section(self),
            voice: voice_section(self),
            light_states: light_section(self),
        }
    }

    fn from_file(file: RegistryFile) -> Result<Self, LoadError> {
        let mut reg = Registry::empty();
        // Mesh
        for od in file.meshes.entries {
            match od.status {
                OnDiskStatus::Deleted => reg.insert_mesh_tombstone(od.serial),
                OnDiskStatus::Placeholder | OnDiskStatus::Shipped => {
                    let entry = MeshEntry {
                        slug: od.slug,
                        status: if matches!(od.status, OnDiskStatus::Placeholder) {
                            AssetStatus::Placeholder
                        } else {
                            AssetStatus::Shipped
                        },
                        primitive: od.primitive,
                        source_path: od.source,
                        usage: Vec::new(),
                    };
                    reg.insert_mesh_at(od.serial, entry)?;
                }
            }
        }
        reg.set_mesh_next_serial(file.meshes.next_serial);
        // Audio
        for od in file.audio.entries {
            match od.status {
                OnDiskStatus::Deleted => reg.insert_audio_tombstone(od.serial),
                OnDiskStatus::Placeholder | OnDiskStatus::Shipped => {
                    let entry = AudioEntry {
                        slug: od.slug,
                        status: status_from_disk(od.status),
                        source_path: od.source,
                        usage: Vec::new(),
                    };
                    reg.insert_audio_at(od.serial, entry)?;
                }
            }
        }
        reg.set_audio_next_serial(file.audio.next_serial);
        // Animations
        for od in file.animations.entries {
            match od.status {
                OnDiskStatus::Deleted => reg.insert_animation_tombstone(od.serial),
                OnDiskStatus::Placeholder | OnDiskStatus::Shipped => {
                    let entry = AnimationEntry {
                        slug: od.slug,
                        status: status_from_disk(od.status),
                        source_path: od.source,
                        usage: Vec::new(),
                    };
                    reg.insert_animation_at(od.serial, entry)?;
                }
            }
        }
        reg.set_animation_next_serial(file.animations.next_serial);
        // Voice
        for od in file.voice.entries {
            match od.status {
                OnDiskStatus::Deleted => reg.insert_voice_tombstone(od.serial),
                OnDiskStatus::Placeholder | OnDiskStatus::Shipped => {
                    let entry = VoiceEntry {
                        slug: od.slug,
                        status: status_from_disk(od.status),
                        source_path: od.source,
                        usage: Vec::new(),
                    };
                    reg.insert_voice_at(od.serial, entry)?;
                }
            }
        }
        reg.set_voice_next_serial(file.voice.next_serial);
        // Light states
        for od in file.light_states.entries {
            match od.status {
                OnDiskStatus::Deleted => reg.insert_light_tombstone(od.serial),
                OnDiskStatus::Placeholder | OnDiskStatus::Shipped => {
                    let entry = LightEntry {
                        slug: od.slug,
                        status: status_from_disk(od.status),
                        source_path: od.source,
                        usage: Vec::new(),
                    };
                    reg.insert_light_at(od.serial, entry)?;
                }
            }
        }
        reg.set_light_next_serial(file.light_states.next_serial);
        reg.rebuild_view();
        Ok(reg)
    }
}

fn status_from_disk(s: OnDiskStatus) -> AssetStatus {
    match s {
        OnDiskStatus::Placeholder => AssetStatus::Placeholder,
        OnDiskStatus::Shipped => AssetStatus::Shipped,
        OnDiskStatus::Deleted => unreachable!("status_from_disk called on Deleted"),
    }
}

fn status_to_disk(s: &AssetStatus) -> OnDiskStatus {
    match s {
        AssetStatus::Placeholder => OnDiskStatus::Placeholder,
        AssetStatus::Shipped => OnDiskStatus::Shipped,
    }
}

fn mesh_section(reg: &Registry) -> Section<MeshOnDisk> {
    let entries = reg
        .iter_mesh_slots()
        .filter_map(|(serial, slot)| match slot {
            EntrySlot::Live(e) => Some(MeshOnDisk {
                serial,
                slug: e.slug.clone(),
                status: status_to_disk(&e.status),
                primitive: e.primitive,
                source: e.source_path.clone(),
            }),
            EntrySlot::Tombstone => Some(MeshOnDisk {
                serial,
                slug: Slug::new(&format!("_deleted_{serial}"))
                    .expect("synthetic deleted slug is valid"),
                status: OnDiskStatus::Deleted,
                primitive: None,
                source: None,
            }),
            EntrySlot::Empty => None,
        })
        .collect();
    Section {
        next_serial: reg.mesh_next_serial(),
        entries,
    }
}

fn audio_section(reg: &Registry) -> Section<AudioOnDisk> {
    let entries = reg
        .iter_audio_slots()
        .filter_map(|(serial, slot)| match slot {
            EntrySlot::Live(e) => Some(AudioOnDisk {
                serial,
                slug: e.slug.clone(),
                status: status_to_disk(&e.status),
                source: e.source_path.clone(),
            }),
            EntrySlot::Tombstone => Some(AudioOnDisk {
                serial,
                slug: Slug::new(&format!("_deleted_{serial}")).expect("synthetic deleted slug is valid"),
                status: OnDiskStatus::Deleted,
                source: None,
            }),
            EntrySlot::Empty => None,
        })
        .collect();
    Section {
        next_serial: reg.audio_next_serial(),
        entries,
    }
}

fn animation_section(reg: &Registry) -> Section<AnimationOnDisk> {
    let entries = reg
        .iter_animation_slots()
        .filter_map(|(serial, slot)| match slot {
            EntrySlot::Live(e) => Some(AnimationOnDisk {
                serial,
                slug: e.slug.clone(),
                status: status_to_disk(&e.status),
                source: e.source_path.clone(),
            }),
            EntrySlot::Tombstone => Some(AnimationOnDisk {
                serial,
                slug: Slug::new(&format!("_deleted_{serial}")).expect("synthetic deleted slug is valid"),
                status: OnDiskStatus::Deleted,
                source: None,
            }),
            EntrySlot::Empty => None,
        })
        .collect();
    Section {
        next_serial: reg.animation_next_serial(),
        entries,
    }
}

fn voice_section(reg: &Registry) -> Section<VoiceOnDisk> {
    let entries = reg
        .iter_voice_slots()
        .filter_map(|(serial, slot)| match slot {
            EntrySlot::Live(e) => Some(VoiceOnDisk {
                serial,
                slug: e.slug.clone(),
                status: status_to_disk(&e.status),
                source: e.source_path.clone(),
            }),
            EntrySlot::Tombstone => Some(VoiceOnDisk {
                serial,
                slug: Slug::new(&format!("_deleted_{serial}")).expect("synthetic deleted slug is valid"),
                status: OnDiskStatus::Deleted,
                source: None,
            }),
            EntrySlot::Empty => None,
        })
        .collect();
    Section {
        next_serial: reg.voice_next_serial(),
        entries,
    }
}

fn light_section(reg: &Registry) -> Section<LightOnDisk> {
    let entries = reg
        .iter_light_slots()
        .filter_map(|(serial, slot)| match slot {
            EntrySlot::Live(e) => Some(LightOnDisk {
                serial,
                slug: e.slug.clone(),
                status: status_to_disk(&e.status),
                source: e.source_path.clone(),
            }),
            EntrySlot::Tombstone => Some(LightOnDisk {
                serial,
                slug: Slug::new(&format!("_deleted_{serial}")).expect("synthetic deleted slug is valid"),
                status: OnDiskStatus::Deleted,
                source: None,
            }),
            EntrySlot::Empty => None,
        })
        .collect();
    Section {
        next_serial: reg.light_next_serial(),
        entries,
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(crate = "pantry::serde", rename_all = "snake_case")]
enum OnDiskStatus {
    Placeholder,
    Shipped,
    Deleted,
}

#[derive(Serialize, Deserialize)]
#[serde(crate = "pantry::serde", deny_unknown_fields)]
struct RegistryFile {
    #[serde(default = "default_section")]
    meshes: Section<MeshOnDisk>,
    #[serde(default = "default_section")]
    audio: Section<AudioOnDisk>,
    #[serde(default = "default_section")]
    animations: Section<AnimationOnDisk>,
    #[serde(default = "default_section")]
    voice: Section<VoiceOnDisk>,
    #[serde(default = "default_section")]
    light_states: Section<LightOnDisk>,
}

fn default_section<E>() -> Section<E> {
    Section {
        next_serial: 0,
        entries: Vec::new(),
    }
}

#[derive(Serialize, Deserialize)]
#[serde(crate = "pantry::serde", deny_unknown_fields)]
struct Section<E> {
    next_serial: u32,
    #[serde(default = "Vec::new")]
    entries: Vec<E>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(crate = "pantry::serde", deny_unknown_fields)]
struct MeshOnDisk {
    serial: u32,
    slug: Slug,
    status: OnDiskStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    primitive: Option<ShapePrimitive>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source: Option<PathBuf>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(crate = "pantry::serde", deny_unknown_fields)]
struct AudioOnDisk {
    serial: u32,
    slug: Slug,
    status: OnDiskStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source: Option<PathBuf>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(crate = "pantry::serde", deny_unknown_fields)]
struct AnimationOnDisk {
    serial: u32,
    slug: Slug,
    status: OnDiskStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source: Option<PathBuf>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(crate = "pantry::serde", deny_unknown_fields)]
struct VoiceOnDisk {
    serial: u32,
    slug: Slug,
    status: OnDiskStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source: Option<PathBuf>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(crate = "pantry::serde", deny_unknown_fields)]
struct LightOnDisk {
    serial: u32,
    slug: Slug,
    status: OnDiskStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source: Option<PathBuf>,
}
