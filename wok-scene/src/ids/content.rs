use pantry::serde::{Deserialize, Serialize};

use super::slug::Slug;

/// Stable identifier for a prefab definition. Serialized as the bare slug string.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(crate = "pantry::serde", transparent)]
pub struct PrefabId(pub Slug);

/// Stable identifier for a scene manifest. Serialized as the bare slug string.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(crate = "pantry::serde", transparent)]
pub struct SceneId(pub Slug);

/// Game-defined trigger identifier. Not slug-validated; the game chooses the spelling.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(crate = "pantry::serde", transparent)]
pub struct TriggerId(pub String);
