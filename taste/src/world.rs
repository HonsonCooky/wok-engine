//! The simulation's view of the loaded content: static collision geometry and terrain.
//!
//! [`World`] is reduced once from the chunk store after loading (taste loads everything up front;
//! streaming is out of scope), and the fixed-step loop reads it every step. Two reductions happen
//! here, both game policy the engine deliberately leaves to the caller:
//!
//! - **Chunk-origin composition.** Runtime arrays are chunk-local; the simulation tracks the player
//!   in world space. Hitboxes are classified into colliders (`wok_physics::classify_collider`:
//!   exact spheres and vertical cylinders where the shape allows, conservative AABBs otherwise) in
//!   chunk-local space and lifted by their chunk's origin once, here, so the per-step slide never
//!   re-derives them. The lift is a pure translation, exact in float, so the local collider and
//!   the lifted one describe the same shape - and classification reads only the transform's basis,
//!   so classifying locally then lifting is the position-independent order.
//! - **Terrain locality.** Heightmaps sample in chunk-local metres, so terrain queries go through
//!   [`World::terrain_under`], which finds the chunk under a world x/z; the caller maps into that
//!   chunk's frame and back. The correction is purely vertical, which is what keeps the canon's
//!   position-independence intact.
//!
//! The fields are public plain data: the replay test builds small worlds by hand through the same
//! type the app fills from disk content.

use glam::{Mat4, Vec3};
use wok_content::ChunkStore;
use wok_physics::{Collider, classify_collider, world_aabb};
use wok_scene::{Aabb, CHUNK_GRID_DIM, ChunkCoord, Heightmap, VisibleItem};

/// Chunk side in metres, derived from the heightmap grid (128 one-metre cells; the 129th sample is
/// the shared edge). wok-scene deliberately does not bake the chunk size into ChunkCoord, so this
/// composition is application policy, the same constant the editor derives.
pub const CHUNK_SIZE_M: f32 = (CHUNK_GRID_DIM - 1) as f32;

/// World-space origin of a chunk: its grid coordinate times the chunk size.
pub fn chunk_origin(coord: ChunkCoord) -> Vec3 {
    Vec3::new(coord.x as f32 * CHUNK_SIZE_M, 0.0, coord.z as f32 * CHUNK_SIZE_M)
}

/// One chunk's terrain, paired with the world-space origin that maps into its local frame.
pub struct ChunkTerrain {
    pub origin: Vec3,
    pub heightmap: Heightmap,
}

/// Everything the fixed-step loop collides against, reduced once after content load.
pub struct World {
    /// Solid hitboxes from every loaded chunk, classified into world-space colliders.
    pub statics: Vec<Collider>,
    /// Terrain per chunk that has it, in the store's deterministic coordinate order.
    pub terrains: Vec<ChunkTerrain>,
}

impl World {
    /// Reduce every loaded chunk in `store` to the simulation's arrays. Iteration order is the
    /// store's coordinate order, so identical content produces identical arrays (the determinism
    /// contract carried through the reduction).
    pub fn from_store(store: &ChunkStore) -> World {
        let mut statics = Vec::new();
        let mut terrains = Vec::new();
        for (coord, runtime) in store.iter_loaded() {
            let origin = chunk_origin(coord);
            for hitbox in &runtime.hitboxes {
                statics.push(classify_collider(hitbox.primitive, hitbox.transform).translated(origin));
            }
            if let Some(heightmap) = runtime.heightmap.clone() {
                terrains.push(ChunkTerrain { origin, heightmap });
            }
        }
        World { statics, terrains }
    }

    /// World-space bounds of everything the loaded chunks hold - terrain plus placed visible and
    /// hitbox extents - the other reduction over the store: the base of the shadow region (caller
    /// policy per the render contract; the same reduction the editor makes). Conservative AABBs
    /// are the right tool here even for round shapes - a region wants cover, not fit. Falls back
    /// to a small box around the origin when nothing is loaded, so the shadow fit stays
    /// well-formed.
    pub fn scene_bounds(store: &ChunkStore) -> Aabb {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        let mut grow = |b: Aabb| {
            min = min.min(b.min);
            max = max.max(b.max);
        };
        for (coord, runtime) in store.iter_loaded() {
            let origin = chunk_origin(coord);
            let origin_mat = Mat4::from_translation(origin);
            if let Some(mesh) = runtime.terrain_mesh.as_ref() {
                let b = mesh.bounds();
                grow(Aabb::new(b.min + origin, b.max + origin));
            }
            for item in &runtime.visible {
                if let VisibleItem::Primitive { primitive, transform, .. } = item {
                    grow(world_aabb(*primitive, origin_mat * *transform));
                }
            }
            for hitbox in &runtime.hitboxes {
                grow(world_aabb(hitbox.primitive, origin_mat * hitbox.transform));
            }
        }
        if min.x > max.x {
            return Aabb::new(Vec3::splat(-1.0), Vec3::splat(1.0));
        }
        Aabb::new(min, max)
    }

    /// The terrain chunk under world-space `(x, z)`, if any. Edges are inclusive on both sides; a
    /// point on a shared edge resolves to the first chunk in coordinate order, and both chunks
    /// sample the same height there (the 129th row is the neighbour's first), so the choice does
    /// not change the answer.
    pub fn terrain_under(&self, x: f32, z: f32) -> Option<&ChunkTerrain> {
        self.terrains.iter().find(|t| {
            x >= t.origin.x
                && x <= t.origin.x + CHUNK_SIZE_M
                && z >= t.origin.z
                && z <= t.origin.z + CHUNK_SIZE_M
        })
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use wok_scene::{CHUNK_GRID_LEN, SurfaceTag};

    fn flat(height_m: f32) -> Heightmap {
        let raw = Heightmap::meters_to_raw(height_m);
        Heightmap::new(vec![raw; CHUNK_GRID_LEN], vec![SurfaceTag::new("g")], vec![0; CHUNK_GRID_LEN]).unwrap()
    }

    #[test]
    fn chunk_origin_scales_by_the_chunk_size() {
        assert_eq!(chunk_origin(ChunkCoord::new(0, 0)), Vec3::ZERO);
        assert_eq!(chunk_origin(ChunkCoord::new(2, -1)), Vec3::new(256.0, 0.0, -128.0));
    }

    #[test]
    fn terrain_under_mapping_inverts_the_mesh_origin_composition() {
        // The terrain mesh draws chunk-local vertices under Mat4::from_translation(chunk_origin)
        // (taste and wok compose it identically); terrain_under hands back the origin that the
        // sampler's caller subtracts. The two must be exact inverses, or the collided surface and
        // the drawn surface would shear apart by the difference - the diagnosis item A3.
        let coord = ChunkCoord::new(2, -1);
        let world = World {
            statics: vec![],
            terrains: vec![ChunkTerrain { origin: chunk_origin(coord), heightmap: flat(3.0) }],
        };
        let local = glam::Vec3::new(40.25, 3.0, 100.5);
        let world_point = glam::Mat4::from_translation(chunk_origin(coord)).transform_point3(local);
        let t = world.terrain_under(world_point.x, world_point.z).expect("the lifted point is inside the chunk");
        assert_eq!(world_point - t.origin, local, "world-to-local must undo the mesh's origin lift exactly");
    }

    #[test]
    fn terrain_under_resolves_by_chunk_extent() {
        let world = World {
            statics: vec![],
            terrains: vec![
                ChunkTerrain { origin: chunk_origin(ChunkCoord::new(0, 0)), heightmap: flat(1.0) },
                ChunkTerrain { origin: chunk_origin(ChunkCoord::new(1, 0)), heightmap: flat(2.0) },
            ],
        };
        assert_eq!(world.terrain_under(64.0, 64.0).unwrap().origin.x, 0.0);
        assert_eq!(world.terrain_under(200.0, 64.0).unwrap().origin.x, 128.0);
        assert!(world.terrain_under(64.0, -10.0).is_none(), "off the south edge is no chunk");
        assert!(world.terrain_under(300.0, 64.0).is_none(), "past the last chunk is no chunk");
    }
}
