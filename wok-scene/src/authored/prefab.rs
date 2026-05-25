use std::collections::BTreeMap;

use pantry::serde::{Deserialize, Serialize};

use crate::ids::{AudioCueId, MeshId, PrefabId};

use super::shape::Shape;

/// One named state of a prefab. The asset registry replaces placeholders at the state level:
/// a real mesh covers all of this state's visible shapes at once. `audio_cues` maps a
/// game-meaningful local name (e.g. `"impact"`, `"open"`) to a registry-resolved cue.
///
/// Deviation from plan section 3: `audio_cues` is `BTreeMap<String, AudioCueId>` rather than
/// `Vec<(String, AudioCueId)>`. The on-disk shape `{"impact": "wood-impact-12"}` reads better
/// than `[["impact", "wood-impact-12"]]`, `BTreeMap` sorts deterministically on save (matching
/// the plan section 4 determinism rule), and duplicate cue names are a bug rather than valid
/// intent. Order does not matter; cues are looked up by name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(crate = "pantry::serde", deny_unknown_fields)]
pub struct PrefabState {
    pub name: String,
    pub shapes: Vec<Shape>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mesh_override: Option<MeshId>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub audio_cues: BTreeMap<String, AudioCueId>,
}

/// Authored prefab. `default_state` names which state to use when a placement omits one.
/// Linear lookup in `states`; indices are not stable across edits.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(crate = "pantry::serde", deny_unknown_fields)]
pub struct Prefab {
    pub id: PrefabId,
    pub default_state: String,
    pub states: Vec<PrefabState>,
}
