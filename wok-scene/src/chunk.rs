//! Chunk-level data: placements and per-chunk streaming metadata.
//!
//! A scene is a grid of 128m x 128m chunks (HLD engine constant; not encoded here). Each chunk
//! file holds the placements within that cell plus optional streaming overrides. The chunk's
//! file name on disk (`{x}_{z}.json`) is the source of truth for which chunks exist; the scene
//! manifest does not list them. wok-content owns the streaming algorithm that turns the chunk
//! metadata into a desired loaded set.

use serde::{Deserialize, Serialize};

use crate::math::Transform;
use crate::refs::{InstanceId, PrefabRef};

/// How aggressively a chunk should be loaded.
///
/// - `Eager` - loaded when within the scene's load radius around the player.
/// - `Lazy` - loaded only on explicit request (e.g. crossing a door trigger).
/// - `Vista` - loaded for rendering but excluded from simulation, collision, and trigger
///   evaluation. Used for distant skyboxes and visible-but-unwalkable terrain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Eagerness {
    Eager,
    Lazy,
    Vista,
}

/// Integer cell coordinate. Multiplied by the engine's chunk size at runtime to get world
/// space; the data does not bake the chunk size in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChunkCoord {
    pub x: i32,
    pub z: i32,
}

impl ChunkCoord {
    pub const fn new(x: i32, z: i32) -> Self {
        ChunkCoord { x, z }
    }
}

/// One instance of a prefab placed in a chunk.
///
/// `instance_id` is allocated from the scene's monotonic counter at placement creation time
/// and stays stable for the lifetime of the placement; see `Scene::allocate_instance_id`.
/// `state` of `None` resolves to the prefab's `default_state` at runtime; reference resolution
/// is wok-content's job, not this crate's. `name` is an optional author-facing display name -
/// pure annotation, never used for reference resolution (references stay by prefab name and
/// instance id) - shown by tools where present and omitted from the file when `None`, so every
/// pre-name file loads unchanged.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Placement {
    pub prefab: PrefabRef,
    pub instance_id: InstanceId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub transform: Transform,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

/// Per-chunk streaming overrides.
///
/// `eagerness` of `None` means "use the scene's `default_eagerness`". `neighbors` and
/// `always_load_with` describe topology the streaming algorithm reads: neighbors are eligible
/// to load when this chunk loads; `always_load_with` chunks are forced to load whenever this
/// chunk does (e.g. interior shells around an exterior).
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChunkStreaming {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eagerness: Option<Eagerness>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub neighbors: Vec<ChunkCoord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub always_load_with: Vec<ChunkCoord>,
}

/// One chunk: its coordinate, placements, and streaming metadata.
///
/// Terrain heightmap data lives in a sibling binary file (`{x}_{z}.heightmap.bin`), loaded and
/// saved separately via `crate::load_heightmap` / `crate::save_heightmap`; see
/// `crate::heightmap`. The chunk JSON does not embed or reference it - the file name pairing is
/// the link.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Chunk {
    pub coord: ChunkCoord,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub placements: Vec<Placement>,
    #[serde(default)]
    pub streaming: ChunkStreaming,
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    fn sample_placement(id: u32) -> Placement {
        Placement {
            prefab: PrefabRef::new("oak_tree"),
            instance_id: InstanceId(id),
            name: None,
            transform: Transform {
                translation: Vec3::new(10.0, 0.0, -5.0),
                ..Transform::IDENTITY
            },
            state: None,
        }
    }

    // ---- Eagerness ----

    #[test]
    fn eagerness_round_trips_for_every_variant() {
        for e in [Eagerness::Eager, Eagerness::Lazy, Eagerness::Vista] {
            let json = serde_json::to_string(&e).unwrap();
            let back: Eagerness = serde_json::from_str(&json).unwrap();
            assert_eq!(back, e);
        }
    }

    // ---- ChunkCoord ----

    #[test]
    fn chunk_coord_round_trips() {
        let c = ChunkCoord::new(-3, 7);
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(json, r#"{"x":-3,"z":7}"#);
        let back: ChunkCoord = serde_json::from_str(&json).unwrap();
        assert_eq!(back, c);
    }

    // ---- Placement ----

    #[test]
    fn placement_round_trips_without_state() {
        let p = sample_placement(7);
        let json = serde_json::to_string(&p).unwrap();
        assert!(!json.contains("\"state\""));
        let back: Placement = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn placement_round_trips_with_state() {
        let mut p = sample_placement(8);
        p.state = Some("destroyed".into());
        let json = serde_json::to_string(&p).unwrap();
        let back: Placement = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn placement_round_trips_with_a_display_name() {
        let mut p = sample_placement(9);
        p.name = Some("the old oak".into());
        let json = serde_json::to_string(&p).unwrap();
        let back: Placement = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn an_unnamed_placement_omits_the_name_key() {
        // The additive-schema promise, write side: None never appears in the file, so saving an
        // untouched scene produces byte-identical placements to the pre-name format.
        let json = serde_json::to_string(&sample_placement(7)).unwrap();
        assert!(!json.contains("\"name\""), "None must be omitted: {json}");
    }

    #[test]
    fn a_pre_name_placement_file_loads_unchanged() {
        // The additive-schema promise, read side: JSON written before the field existed (no
        // `name` key anywhere) deserializes with `name: None`.
        let legacy = r#"{
            "prefab": "oak_tree",
            "instance_id": 7,
            "transform": { "pos": [10.0, 0.0, -5.0] }
        }"#;
        let back: Placement = serde_json::from_str(legacy).unwrap();
        assert_eq!(back, sample_placement(7));
    }

    // ---- ChunkStreaming ----

    #[test]
    fn chunk_streaming_default_serializes_empty() {
        let s = ChunkStreaming::default();
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, "{}");
        let back: ChunkStreaming = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn chunk_streaming_round_trips_populated() {
        let s = ChunkStreaming {
            eagerness: Some(Eagerness::Lazy),
            neighbors: vec![ChunkCoord::new(1, 0), ChunkCoord::new(0, 1)],
            always_load_with: vec![ChunkCoord::new(-1, -1)],
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: ChunkStreaming = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    // ---- Chunk ----

    #[test]
    fn chunk_round_trips_empty() {
        let c = Chunk {
            coord: ChunkCoord::new(0, 0),
            placements: vec![],
            streaming: ChunkStreaming::default(),
        };
        let json = serde_json::to_string(&c).unwrap();
        let back: Chunk = serde_json::from_str(&json).unwrap();
        assert_eq!(back, c);
    }

    #[test]
    fn chunk_round_trips_populated() {
        let c = Chunk {
            coord: ChunkCoord::new(2, -1),
            placements: vec![sample_placement(1), sample_placement(2)],
            streaming: ChunkStreaming {
                eagerness: Some(Eagerness::Eager),
                neighbors: vec![ChunkCoord::new(3, -1)],
                always_load_with: vec![],
            },
        };
        let json = serde_json::to_string(&c).unwrap();
        let back: Chunk = serde_json::from_str(&json).unwrap();
        assert_eq!(back, c);
    }
}
