//! The runtime-arrays form of one chunk, and the transform that produces it.
//!
//! `transform_chunk` is the HLD's "authored memory -> runtime arrays" transition (data-flow state 2
//! to state 3): it composes wok-scene's `slice_chunk` with wok-mesh's `terrain_mesh` and collects the
//! asset names the result references. The authored chunk is consumed by value, so after the transform
//! it is no longer referenced - the HLD data-flow rule enforced by the type system rather than by
//! convention. The heightmap moves in whole: physics samples it at runtime, so unlike the placements
//! it stays alive in the runtime form rather than being transformed away.
//!
//! Asset names stay unresolved. `referenced_assets` is the per-chunk seed the future content scan
//! reads to build the missing-assets list; no loader exists yet and this crate does not invent one.
//! The names are collected from the runtime arrays themselves (the mesh items the resolved prefab
//! states actually emitted), deduplicated in first-appearance order: that is exactly the set this
//! chunk's runtime form needs, while meshes referenced only by unplaced prefab states are the content
//! scan's concern at the prefab level, not this chunk's.
//!
//! Determinism (canon contract): a fixed composition of deterministic steps - slicing is sequential
//! in placement order, terrain meshing is a fixed loop over the grid, and asset collection walks the
//! visible array in order. Identical inputs produce identical arrays, bit for bit. No threads, no
//! clocks, no RNG.

use std::collections::HashMap;
use std::hash::BuildHasher;

use wok_mesh::MeshCpu;
use wok_scene::{
    Chunk, ChunkCoord, Heightmap, Hitbox, MeshRef, Prefab, PrefabRef, SliceError, Trigger,
    VisibleItem, slice_chunk,
};

/// Failure modes of `transform_chunk`. One variant today: slicing is the only fallible step (terrain
/// meshing is total over a valid `Heightmap`, and asset collection cannot fail). New steps that can
/// fail (asset-name resolution against loaded data, in a later part) add variants here.
#[derive(Debug, thiserror::Error)]
pub enum TransformError {
    /// A placement referenced a prefab or state the caller's library does not supply.
    #[error("chunk slicing failed: {0}")]
    Slice(#[from] SliceError),
}

/// The runtime-arrays form of one chunk (HLD data-flow state 3).
///
/// Owns everything downstream systems read per frame: the slice arrays (chunk-local transforms, per
/// wok-scene's slicer contract), the terrain mesh when the chunk has terrain, the heightmap physics
/// samples, and the unresolved asset names the chunk references. Produced only by `transform_chunk`;
/// the fields are public because the struct is a plain data carrier with no invariant beyond
/// "terrain_mesh is present iff heightmap is", which the one producer upholds by construction.
#[derive(Clone, Debug, PartialEq)]
pub struct ChunkRuntime {
    /// The chunk's grid coordinate, carried from the authored form: the consumer derives the chunk's
    /// world offset from it when composing chunk-local transforms into world space.
    pub coord: ChunkCoord,
    /// Drawables: primitive placeholders and replacement meshes, chunk-local transforms.
    pub visible: Vec<VisibleItem>,
    /// Collision surfaces for solid placeholders.
    pub hitboxes: Vec<Hitbox>,
    /// Trigger volumes, each tagged with the owning placement's instance id.
    pub triggers: Vec<Trigger>,
    /// The triangulated terrain surface, present iff the chunk has a heightmap.
    pub terrain_mesh: Option<MeshCpu>,
    /// The terrain heightmap, moved out of the authored form: physics samples it at runtime.
    pub heightmap: Option<Heightmap>,
    /// Every asset name the runtime arrays reference, unresolved, deduplicated, in first-appearance
    /// order. Meshes are the only asset kind reachable from a chunk today; the list grows kinds as
    /// the data model does (audio, etc.). Seed data for the future content scan.
    pub referenced_assets: Vec<MeshRef>,
}

/// Transform one authored chunk into its runtime form.
///
/// Consumes the chunk and heightmap (see module docs); borrows the prefab library, which is shared
/// across chunks and resolved by the caller, exactly as `wok_scene::slice_chunk` takes it. Generic
/// over the map's hasher for the same reason wok-scene is: the caller passes whatever map it holds.
// Consuming `chunk` is the point (module docs): the authored form dies here, by move, even though
// the body only reads it. Clippy cannot see that intent through the signature.
#[allow(clippy::needless_pass_by_value)]
pub fn transform_chunk<S: BuildHasher>(
    chunk: Chunk,
    heightmap: Option<Heightmap>,
    prefabs: &HashMap<PrefabRef, Prefab, S>,
) -> Result<ChunkRuntime, TransformError> {
    let sliced = slice_chunk(&chunk, prefabs)?;
    let referenced_assets = collect_referenced_assets(&sliced.visible);
    let terrain_mesh = heightmap.as_ref().map(wok_mesh::terrain_mesh);
    Ok(ChunkRuntime {
        coord: chunk.coord,
        visible: sliced.visible,
        hitboxes: sliced.hitboxes,
        triggers: sliced.triggers,
        terrain_mesh,
        heightmap,
        referenced_assets,
    })
}

/// Collect the mesh names the visible array references, deduplicated in first-appearance order.
/// Linear scan for the dedup: a chunk holds at most a few hundred placements, so a set buys nothing.
fn collect_referenced_assets(visible: &[VisibleItem]) -> Vec<MeshRef> {
    let mut out: Vec<MeshRef> = Vec::new();
    for item in visible {
        if let VisibleItem::Mesh { mesh, .. } = item {
            if !out.contains(mesh) {
                out.push(mesh.clone());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::{fixture_chunk, flat_heightmap, library, simple_chunk};
    use wok_scene::ChunkStreaming;

    #[test]
    fn slice_arrays_match_the_slicer_run_directly() {
        let prefabs = library();
        let chunk = fixture_chunk();
        let direct = slice_chunk(&chunk, &prefabs).unwrap();
        let runtime = transform_chunk(chunk, None, &prefabs).unwrap();
        assert_eq!(runtime.visible, direct.visible);
        assert_eq!(runtime.hitboxes, direct.hitboxes);
        assert_eq!(runtime.triggers, direct.triggers);
        assert_eq!(runtime.coord, ChunkCoord::new(0, 0));
    }

    #[test]
    fn terrain_mesh_is_generated_when_a_heightmap_is_present() {
        let heightmap = flat_heightmap(1000);
        let expected = wok_mesh::terrain_mesh(&heightmap);
        let runtime = transform_chunk(fixture_chunk(), Some(heightmap), &library()).unwrap();
        assert_eq!(runtime.terrain_mesh, Some(expected));
    }

    #[test]
    fn no_terrain_mesh_without_a_heightmap() {
        let runtime = transform_chunk(fixture_chunk(), None, &library()).unwrap();
        assert!(runtime.terrain_mesh.is_none());
        assert!(runtime.heightmap.is_none());
    }

    #[test]
    fn heightmap_moves_into_the_runtime_unchanged() {
        let heightmap = flat_heightmap(2000);
        let runtime =
            transform_chunk(fixture_chunk(), Some(heightmap.clone()), &library()).unwrap();
        assert_eq!(runtime.heightmap, Some(heightmap));
    }

    #[test]
    fn referenced_assets_are_deduplicated_in_first_appearance_order() {
        // The fixture places "tree" (oak_tree) twice before "sign" (wooden_sign): the duplicate
        // collapses and the order is the placements' order, not alphabetical.
        let runtime = transform_chunk(fixture_chunk(), None, &library()).unwrap();
        assert_eq!(
            runtime.referenced_assets,
            vec![MeshRef::new("oak_tree"), MeshRef::new("wooden_sign")]
        );
    }

    #[test]
    fn referenced_assets_are_empty_for_a_chunk_of_pure_primitives() {
        let runtime = transform_chunk(simple_chunk(0, 0, 1), None, &library()).unwrap();
        assert!(runtime.referenced_assets.is_empty());
    }

    #[test]
    fn double_transform_is_identical() {
        // The canon determinism contract at this crate's boundary: identical inputs, identical
        // arrays. PartialEq on the runtime compares every f32 exactly, so equal here means equal
        // bit patterns for all finite values, which these are.
        let prefabs = library();
        let (chunk, heightmap) = (fixture_chunk(), flat_heightmap(1234));
        let first = transform_chunk(chunk.clone(), Some(heightmap.clone()), &prefabs).unwrap();
        let second = transform_chunk(chunk, Some(heightmap), &prefabs).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn unknown_prefab_surfaces_as_a_transform_error() {
        let chunk = simple_chunk(1, 1, 9);
        let empty = std::collections::HashMap::new();
        match transform_chunk(chunk, None, &empty).unwrap_err() {
            TransformError::Slice(SliceError::UnknownPrefab(r)) => {
                assert_eq!(r, PrefabRef::new("rock"));
            }
            other => panic!("expected UnknownPrefab, got {other:?}"),
        }
    }

    #[test]
    fn empty_chunk_transforms_to_empty_arrays() {
        let chunk = Chunk {
            coord: ChunkCoord::new(3, -2),
            placements: vec![],
            streaming: ChunkStreaming::default(),
        };
        let runtime = transform_chunk(chunk, None, &library()).unwrap();
        assert!(runtime.visible.is_empty());
        assert!(runtime.hitboxes.is_empty());
        assert!(runtime.triggers.is_empty());
        assert!(runtime.referenced_assets.is_empty());
        assert_eq!(runtime.coord, ChunkCoord::new(3, -2));
    }
}
