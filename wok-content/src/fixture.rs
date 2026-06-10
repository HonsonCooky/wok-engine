//! Shared test fixture: one authored chunk exercising every slice routing path, plus a heightmap.
//!
//! The prefab library covers the slicer's four routes: a solid placeholder (visible + hitbox), a
//! trigger volume (hitbox only), a mesh-replaced prefab that keeps a collision shape, and a second
//! mesh-replaced prefab so referenced-asset collection sees more than one name. The fixture chunk
//! places the tree twice so deduplication is observable. Expected slice output, for orientation:
//! 4 visible items (rock cube, oak mesh x2, sign mesh), 3 hitboxes (rock cube, tree cylinder x2),
//! 1 trigger (the zone, instance 2). Tests compare against `wok_scene::slice_chunk` run directly
//! rather than restating these counts.

use std::collections::HashMap;

use glam::Vec3;
use wok_scene::{
    CHUNK_GRID_LEN, Chunk, ChunkCoord, ChunkStreaming, Heightmap, InstanceId, MeshRef, Placement,
    Prefab, PrefabRef, PrefabState, Primitive, Shape, SurfaceTag, Transform,
};

fn shape(primitive: Primitive, is_visible: bool, is_hitbox: bool) -> Shape {
    Shape { primitive, transform: Transform::IDENTITY, surface: None, is_hitbox, is_visible }
}

/// A prefab with a single `default` state holding the given shapes and optional mesh.
fn prefab(shapes: Vec<Shape>, mesh: Option<MeshRef>) -> Prefab {
    Prefab {
        states: vec![PrefabState { name: "default".into(), shapes, mesh }],
        default_state: "default".into(),
    }
}

fn placement(prefab: &str, id: u32, translation: Vec3) -> Placement {
    Placement {
        prefab: PrefabRef::new(prefab),
        instance_id: InstanceId(id),
        transform: Transform { translation, ..Transform::IDENTITY },
        state: None,
    }
}

/// The prefab library the fixture chunk's placements resolve against.
pub(crate) fn library() -> HashMap<PrefabRef, Prefab> {
    [
        ("rock", prefab(vec![shape(Primitive::Cube, true, true)], None)),
        ("zone", prefab(vec![shape(Primitive::Cube, false, true)], None)),
        (
            "tree",
            prefab(vec![shape(Primitive::Cylinder, true, true)], Some(MeshRef::new("oak_tree"))),
        ),
        (
            "sign",
            prefab(vec![shape(Primitive::Plane, true, false)], Some(MeshRef::new("wooden_sign"))),
        ),
    ]
    .into_iter()
    .map(|(name, p)| (PrefabRef::new(name), p))
    .collect()
}

/// The fixture chunk at (0, 0): rock, zone, tree, tree (duplicate mesh reference), sign.
pub(crate) fn fixture_chunk() -> Chunk {
    Chunk {
        coord: ChunkCoord::new(0, 0),
        placements: vec![
            placement("rock", 1, Vec3::new(4.0, 0.0, 4.0)),
            placement("zone", 2, Vec3::new(10.0, 1.0, 10.0)),
            placement("tree", 3, Vec3::new(20.0, 0.0, 8.0)),
            placement("tree", 4, Vec3::new(24.0, 0.0, 8.0)),
            placement("sign", 5, Vec3::new(5.0, 0.0, 30.0)),
        ],
        streaming: ChunkStreaming::default(),
    }
}

/// A minimal chunk at the given coordinate: one rock placement. For store tests that need several
/// distinct chunks without caring about their content.
pub(crate) fn simple_chunk(x: i32, z: i32, id: u32) -> Chunk {
    Chunk {
        coord: ChunkCoord::new(x, z),
        placements: vec![placement("rock", id, Vec3::ZERO)],
        streaming: ChunkStreaming::default(),
    }
}

/// Flat terrain at a single raw height across every sample, one surface tag.
pub(crate) fn flat_heightmap(raw: u16) -> Heightmap {
    Heightmap::new(vec![raw; CHUNK_GRID_LEN], vec![SurfaceTag::new("grass")], vec![0; CHUNK_GRID_LEN])
        .unwrap()
}
