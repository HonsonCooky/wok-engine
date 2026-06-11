//! Chunk slicing: authored placements transformed into per-system runtime arrays.
//!
//! `slice_chunk` is the wok-scene half of the HLD "authored memory -> runtime arrays" step. It
//! walks a chunk's placements in order, resolves each one's prefab state, and routes every shape
//! by its `is_visible` / `is_hitbox` flags: solid placeholder (both) -> a visible item AND a
//! hitbox; trigger volume (hitbox only) -> a trigger tagged with the placement's `InstanceId`;
//! visual-only (visible only) -> a visible item; neither -> ignored (degenerate, wok-shell warns).
//!
//! Mesh replacement is at the state level: a resolved state with a `MeshRef` emits one mesh item
//! at the placement transform in place of all its visible primitives. Hitboxes and triggers still
//! come from the shapes, because a mesh supplies appearance, not collision.
//!
//! The caller (wok-content) supplies resolved prefabs via the `prefabs` map; wok-scene loads
//! nothing here, and reads the map by key only, so its iteration order never reaches the output.
//!
//! Determinism (canon contract): sequential in placement order then shape order, no parallelism,
//! no wall-clock. Transforms are chunk-local - `placement * shape` for shapes, `placement` for the
//! mesh item - never folding in the chunk's world offset, so output stays position-independent
//! (the consumer applies the offset downstream).

use std::collections::HashMap;
use std::hash::BuildHasher;

use crate::chunk::{Chunk, Placement};
use crate::math::Mat4;
use crate::prefab::{Prefab, PrefabState, Primitive};
use crate::refs::{InstanceId, MeshRef, PrefabRef, SurfaceTag};

/// Failure modes of `slice_chunk`. Narrow by design: slicing only fails when a placement points at
/// a prefab or state the caller did not supply. Kept separate from `crate::LoadError` because
/// loading a file and slicing already-resolved data are independent failure domains (the canon's
/// LoadError-vs-SliceError example).
#[derive(Debug, thiserror::Error)]
pub enum SliceError {
    /// A placement names a prefab absent from the `prefabs` lookup.
    #[error("placement references unknown prefab {0:?}")]
    UnknownPrefab(PrefabRef),

    /// A placement names a state the resolved prefab does not define.
    #[error("placement on prefab {prefab:?} names unknown state {state:?}")]
    UnknownState { prefab: PrefabRef, state: String },
}

/// One drawable produced by slicing: either a primitive placeholder or a replacement mesh.
///
/// Transforms are chunk-local 4x4 matrices; the consumer composes the chunk's world offset on top
/// downstream (see the determinism note in the module docs).
#[derive(Clone, Debug, PartialEq)]
pub enum VisibleItem {
    /// A primitive placeholder shape, carried with its optional surface tag.
    Primitive {
        primitive: Primitive,
        transform: Mat4,
        surface: Option<SurfaceTag>,
    },
    /// A named mesh standing in for a state's visible shapes.
    Mesh { mesh: MeshRef, transform: Mat4 },
}

/// One collision surface: a solid-placeholder shape lifted into chunk-local space.
#[derive(Clone, Debug, PartialEq)]
pub struct Hitbox {
    pub primitive: Primitive,
    pub transform: Mat4,
    pub surface: Option<SurfaceTag>,
}

/// One trigger volume, tagged with the placement instance that owns it so game logic can route
/// overlap events back to a specific instance.
#[derive(Clone, Debug, PartialEq)]
pub struct Trigger {
    pub primitive: Primitive,
    pub transform: Mat4,
    pub instance: InstanceId,
}

/// The runtime arrays a chunk slices into: drawables, collision surfaces, and trigger volumes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SlicedChunk {
    pub visible: Vec<VisibleItem>,
    pub hitboxes: Vec<Hitbox>,
    pub triggers: Vec<Trigger>,
}

/// Slice an authored chunk into per-system runtime arrays using caller-resolved prefabs.
///
/// See the module docs for the routing rules, mesh replacement, and the determinism contract.
/// Generic over the map's hasher so the caller can pass whatever `HashMap` it already holds.
pub fn slice_chunk<S: BuildHasher>(
    chunk: &Chunk,
    prefabs: &HashMap<PrefabRef, Prefab, S>,
) -> Result<SlicedChunk, SliceError> {
    let mut out = SlicedChunk::default();

    for placement in &chunk.placements {
        let prefab = prefabs
            .get(&placement.prefab)
            .ok_or_else(|| SliceError::UnknownPrefab(placement.prefab.clone()))?;
        let state = resolve_state(prefab, placement)?;
        let placement_mat = placement.transform.to_mat4();

        // A state with a mesh draws that one mesh in place of all its visible primitive shapes.
        // Hitboxes and triggers below are unaffected: a mesh is appearance, not collision.
        let has_mesh = state.mesh.is_some();
        if let Some(mesh) = &state.mesh {
            out.visible.push(VisibleItem::Mesh {
                mesh: mesh.clone(),
                transform: placement_mat,
            });
        }

        for shape in &state.shapes {
            let transform = placement_mat * shape.transform.to_mat4();

            // Visible primitives are emitted only when no mesh stands in for them.
            if shape.is_visible && !has_mesh {
                out.visible.push(VisibleItem::Primitive {
                    primitive: shape.primitive,
                    transform,
                    surface: shape.surface.clone(),
                });
            }

            // The hitbox flag splits by visibility: a visible hitbox is a solid placeholder's
            // collision surface, a non-visible hitbox is a trigger volume owned by this instance.
            if shape.is_hitbox {
                if shape.is_visible {
                    out.hitboxes.push(Hitbox {
                        primitive: shape.primitive,
                        transform,
                        surface: shape.surface.clone(),
                    });
                } else {
                    out.triggers.push(Trigger {
                        primitive: shape.primitive,
                        transform,
                        instance: placement.instance_id,
                    });
                }
            }
        }
    }

    Ok(out)
}

/// Resolve the state a placement uses: its explicit `state`, else the prefab's `default_state`.
fn resolve_state<'a>(
    prefab: &'a Prefab,
    placement: &Placement,
) -> Result<&'a PrefabState, SliceError> {
    let name = placement
        .state
        .as_deref()
        .unwrap_or(prefab.default_state.as_str());
    prefab
        .states
        .iter()
        .find(|s| s.name == name)
        .ok_or_else(|| SliceError::UnknownState {
            prefab: placement.prefab.clone(),
            state: name.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::{ChunkCoord, ChunkStreaming};
    use crate::math::Transform;
    use crate::prefab::Shape;
    use glam::Vec3;

    fn shape(primitive: Primitive, is_visible: bool, is_hitbox: bool) -> Shape {
        Shape {
            primitive,
            transform: Transform::IDENTITY,
            surface: None,
            is_hitbox,
            is_visible,
        }
    }

    fn translated(x: f32, y: f32, z: f32) -> Transform {
        Transform { translation: Vec3::new(x, y, z), ..Transform::IDENTITY }
    }

    /// A prefab with a single `default` state holding the given shapes and optional mesh.
    fn prefab(shapes: Vec<Shape>, mesh: Option<MeshRef>) -> Prefab {
        Prefab {
            states: vec![PrefabState { name: "default".into(), shapes, mesh }],
            default_state: "default".into(),
        }
    }

    fn placement(prefab: &str, id: u32, state: Option<&str>) -> Placement {
        Placement {
            prefab: PrefabRef::new(prefab),
            instance_id: InstanceId(id),
            name: None,
            transform: Transform::IDENTITY,
            state: state.map(String::from),
        }
    }

    fn chunk_of(placements: Vec<Placement>) -> Chunk {
        Chunk { coord: ChunkCoord::new(0, 0), placements, streaming: ChunkStreaming::default() }
    }

    fn library(entries: Vec<(&str, Prefab)>) -> HashMap<PrefabRef, Prefab> {
        entries.into_iter().map(|(name, p)| (PrefabRef::new(name), p)).collect()
    }

    // Each of the four flag combinations lands in the right array(s).

    #[test]
    fn solid_placeholder_emits_visible_and_hitbox() {
        let prefabs = library(vec![("box", prefab(vec![shape(Primitive::Cube, true, true)], None))]);
        let sliced = slice_chunk(&chunk_of(vec![placement("box", 1, None)]), &prefabs).unwrap();
        assert_eq!(sliced.visible.len(), 1);
        assert_eq!(sliced.hitboxes.len(), 1);
        assert!(sliced.triggers.is_empty());
        assert!(matches!(sliced.visible[0], VisibleItem::Primitive { primitive: Primitive::Cube, .. }));
        assert_eq!(sliced.hitboxes[0].primitive, Primitive::Cube);
    }

    #[test]
    fn trigger_volume_emits_trigger_with_instance_id() {
        let prefabs = library(vec![("zone", prefab(vec![shape(Primitive::Cube, false, true)], None))]);
        let sliced = slice_chunk(&chunk_of(vec![placement("zone", 7, None)]), &prefabs).unwrap();
        assert!(sliced.visible.is_empty());
        assert!(sliced.hitboxes.is_empty());
        assert_eq!(sliced.triggers.len(), 1);
        // The trigger carries the placement's instance id, the link game logic routes events on.
        assert_eq!(sliced.triggers[0].instance, InstanceId(7));
    }

    #[test]
    fn visual_only_emits_visible_only() {
        let prefabs = library(vec![("decal", prefab(vec![shape(Primitive::Plane, true, false)], None))]);
        let sliced = slice_chunk(&chunk_of(vec![placement("decal", 1, None)]), &prefabs).unwrap();
        assert_eq!(sliced.visible.len(), 1);
        assert!(sliced.hitboxes.is_empty());
        assert!(sliced.triggers.is_empty());
    }

    #[test]
    fn neither_flag_is_ignored() {
        let prefabs = library(vec![("ghost", prefab(vec![shape(Primitive::Cube, false, false)], None))]);
        let sliced = slice_chunk(&chunk_of(vec![placement("ghost", 1, None)]), &prefabs).unwrap();
        assert!(sliced.visible.is_empty());
        assert!(sliced.hitboxes.is_empty());
        assert!(sliced.triggers.is_empty());
    }

    #[test]
    fn mesh_state_emits_one_mesh_item_and_keeps_hitbox_and_trigger() {
        // A state with a mesh, a solid shape (hitbox + visible), and a trigger shape (hitbox only).
        let prefabs = library(vec![(
            "tree",
            prefab(
                vec![shape(Primitive::Cylinder, true, true), shape(Primitive::Cube, false, true)],
                Some(MeshRef::new("oak_tree_lod0")),
            ),
        )]);
        let sliced = slice_chunk(&chunk_of(vec![placement("tree", 3, None)]), &prefabs).unwrap();

        // The visible output is exactly the mesh, not the cylinder primitive it replaces.
        assert_eq!(sliced.visible.len(), 1);
        assert_eq!(
            sliced.visible[0],
            VisibleItem::Mesh { mesh: MeshRef::new("oak_tree_lod0"), transform: Mat4::IDENTITY }
        );
        // Hitbox and trigger still come from the shapes.
        assert_eq!(sliced.hitboxes.len(), 1);
        assert_eq!(sliced.hitboxes[0].primitive, Primitive::Cylinder);
        assert_eq!(sliced.triggers.len(), 1);
        assert_eq!(sliced.triggers[0].instance, InstanceId(3));
    }

    #[test]
    fn shape_transform_is_placement_times_shape() {
        let shape_tf = translated(2.0, 0.0, 0.0);
        let placement_tf = translated(10.0, 0.0, 0.0);
        let mut s = shape(Primitive::Cube, true, true);
        s.transform = shape_tf;
        let prefabs = library(vec![("box", prefab(vec![s], None))]);
        let mut p = placement("box", 1, None);
        p.transform = placement_tf;

        let sliced = slice_chunk(&chunk_of(vec![p]), &prefabs).unwrap();
        let expected = placement_tf.to_mat4() * shape_tf.to_mat4();
        match &sliced.visible[0] {
            VisibleItem::Primitive { transform, .. } => assert_eq!(*transform, expected),
            other => panic!("expected primitive, got {other:?}"),
        }
        assert_eq!(sliced.hitboxes[0].transform, expected);
    }

    #[test]
    fn mesh_item_transform_is_placement_only() {
        let placement_tf = translated(4.0, 5.0, 6.0);
        let prefabs = library(vec![("tree", prefab(vec![], Some(MeshRef::new("oak"))))]);
        let mut p = placement("tree", 1, None);
        p.transform = placement_tf;
        let sliced = slice_chunk(&chunk_of(vec![p]), &prefabs).unwrap();
        match &sliced.visible[0] {
            VisibleItem::Mesh { transform, .. } => assert_eq!(*transform, placement_tf.to_mat4()),
            other => panic!("expected mesh, got {other:?}"),
        }
    }

    #[test]
    fn explicit_state_overrides_default() {
        let pf = Prefab {
            states: vec![
                PrefabState { name: "default".into(), shapes: vec![shape(Primitive::Cube, true, false)], mesh: None },
                PrefabState { name: "destroyed".into(), shapes: vec![], mesh: None }, // empty: nothing emitted
            ],
            default_state: "default".into(),
        };
        let prefabs = library(vec![("crate", pf)]);
        let sliced = slice_chunk(&chunk_of(vec![placement("crate", 1, Some("destroyed"))]), &prefabs).unwrap();
        assert!(sliced.visible.is_empty(), "destroyed state has no shapes");
    }

    #[test]
    fn slicing_is_deterministic() {
        let prefabs = library(vec![(
            "box",
            prefab(vec![shape(Primitive::Cube, true, true), shape(Primitive::Cube, false, true)], None),
        )]);
        let chunk = chunk_of(vec![placement("box", 1, None), placement("box", 2, None)]);
        assert_eq!(slice_chunk(&chunk, &prefabs).unwrap(), slice_chunk(&chunk, &prefabs).unwrap());
    }

    #[test]
    fn output_follows_placement_order() {
        let prefabs = library(vec![("zone", prefab(vec![shape(Primitive::Cube, false, true)], None))]);
        let chunk = chunk_of(vec![placement("zone", 10, None), placement("zone", 20, None)]);
        let sliced = slice_chunk(&chunk, &prefabs).unwrap();
        let ids: Vec<u32> = sliced.triggers.iter().map(|t| t.instance.0).collect();
        assert_eq!(ids, vec![10, 20]);
    }

    #[test]
    fn unknown_prefab_is_slice_error() {
        let prefabs = library(vec![]);
        match slice_chunk(&chunk_of(vec![placement("missing", 1, None)]), &prefabs).unwrap_err() {
            SliceError::UnknownPrefab(r) => assert_eq!(r, PrefabRef::new("missing")),
            other => panic!("expected UnknownPrefab, got {other:?}"),
        }
    }

    #[test]
    fn unknown_state_is_slice_error() {
        let prefabs = library(vec![("box", prefab(vec![shape(Primitive::Cube, true, true)], None))]);
        match slice_chunk(&chunk_of(vec![placement("box", 1, Some("nonexistent"))]), &prefabs).unwrap_err() {
            SliceError::UnknownState { prefab, state } => {
                assert_eq!(prefab, PrefabRef::new("box"));
                assert_eq!(state, "nonexistent");
            }
            other => panic!("expected UnknownState, got {other:?}"),
        }
    }
}
