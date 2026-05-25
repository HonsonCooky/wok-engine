use pantry::serde::{Deserialize, Serialize};

use crate::ids::{ChunkCoord, LightStateRef, SceneId};

use super::streaming::ChunkEagerness;

/// Scene manifest. Lists which chunks exist for this scene and the default streaming and
/// lighting state. Per-chunk overrides live in each chunk file's metadata, not here.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(crate = "pantry::serde", deny_unknown_fields)]
pub struct Scene {
    pub id: SceneId,
    pub default_load_radius_meters: f32,
    pub default_eagerness: ChunkEagerness,
    pub default_light_state: LightStateRef,
    pub chunks: Vec<ChunkCoord>,
}
