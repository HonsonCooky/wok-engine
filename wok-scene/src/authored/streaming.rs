use pantry::serde::{Deserialize, Serialize};

use crate::ids::ChunkCoord;

/// Streaming class for a chunk. Slicing treats all three values identically; downstream
/// crates (`wok-content`, `wok-physics`, `wok-render`, etc.) consult the tag to decide what
/// to skip - see plan section 8 for the runtime semantics.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(crate = "pantry::serde", rename_all = "snake_case")]
pub enum ChunkEagerness {
    Eager,
    Lazy,
    Vista,
}

/// Per-chunk topology that informs the streaming algorithm. `neighbors` are chunks expected
/// to be loaded alongside via radius; `interlocks` are chunks that must be loaded together
/// regardless of distance (portals, teleporters, scripted gates).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(crate = "pantry::serde", deny_unknown_fields)]
pub struct ChunkMetadata {
    pub eagerness: ChunkEagerness,
    pub neighbors: Vec<ChunkCoord>,
    pub interlocks: Vec<ChunkCoord>,
}
