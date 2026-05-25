use pantry::math::{Aabb, Transform};
use pantry::serde::{Deserialize, Serialize};

use crate::ids::{ChunkCoord, LightStateRef, PrefabId};

use super::streaming::ChunkMetadata;

/// One placed prefab instance inside a chunk. Transform is chunk-local; world coordinates
/// are obtained by composing with `chunk.coord.to_world_offset()` at consumer sites.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(crate = "pantry::serde", deny_unknown_fields)]
pub struct PrefabPlacement {
    pub prefab: PrefabId,
    pub transform: Transform,
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_tag: Option<String>,
}

/// Environmental zone purpose. Not a gameplay trigger - those are prefab trigger volumes.
/// Region markers describe environmental properties (fog, lighting, ambient color) that
/// apply within a bounded volume.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    crate = "pantry::serde",
    tag = "kind",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum RegionPurpose {
    Fog {
        color: [f32; 3],
        density: f32,
        distance: f32,
    },
    Lighting {
        state: LightStateRef,
    },
    Ambient {
        color: [f32; 3],
    },
}

/// Region marker. Bounds are chunk-local; cross-chunk regions are authored as one marker
/// per chunk with shared parameters, each clipped to its own chunk's extents.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(crate = "pantry::serde", deny_unknown_fields)]
pub struct RegionMarker {
    pub name: String,
    pub bounds: Aabb,
    pub purpose: RegionPurpose,
}

/// One authored chunk. The on-disk `_format` integer is a file-level concern handled by
/// `load`/`save`; the struct does not carry it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(crate = "pantry::serde", deny_unknown_fields)]
pub struct Chunk {
    pub coord: ChunkCoord,
    pub metadata: ChunkMetadata,
    pub light_state: LightStateRef,
    pub placements: Vec<PrefabPlacement>,
    pub regions: Vec<RegionMarker>,
}
