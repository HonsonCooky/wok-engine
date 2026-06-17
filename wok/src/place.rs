//! Terrain rest policy and the conservative bounds it rests on.
//!
//! How a placed prefab is vertically corrected to sit on the heightmap is application-side policy
//! by design (the engine provides `height_at`; what to do with it is authoring intent), so it lives
//! in the wok application rather than an engine crate. The sample generator assigns a tuned [`Rest`]
//! per prefab; place mode (with picking, a later brief) will derive one per prefab's shapes.
//!
//! [`world_aabb`] is the conservative world-space box of a transformed unit primitive, computed from
//! wok-scene's canonical [`Primitive::unit_aabb`] exactly as wok-physics does (transform the eight
//! corners, take the component-wise min/max). The frame uses it for rest footprints and for the
//! shadow region (`crate::scene`); wok-physics is not a dependency yet, and reading the same unit
//! convention keeps these boxes from drifting from the colliders picking will later classify.

use glam::Vec3;
use wok_scene::{Aabb, Heightmap, Mat4, Prefab, Primitive, Transform};

/// How a prefab is rested on the terrain at placement time. Which rest fits is authoring intent
/// about the shape's underside, not derivable from geometry alone; the sample generator records it
/// per prefab.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Rest {
    /// Corner rest: the bounds' bottom sits on the highest of its footprint samples (the four
    /// bottom corners and the centre), so no corner hovers or sinks. Right for flat-bottomed shapes,
    /// whose silhouette really is their bounds.
    Corner,
    /// The bounds' bottom sits on the terrain sample under the placement centre (`height_at`), sunk
    /// `sink_m` so a curved underside meets the ground instead of tangentially perching on it. Right
    /// for round or organic shapes, which corner rest floats on any slope.
    Center { sink_m: f32 },
}

/// Conservative world-space (or chunk-local: the matrix decides the frame) AABB of a transformed
/// primitive. Transforms the eight corners of the primitive's canonical [`Primitive::unit_aabb`] and
/// takes their component-wise min/max - correct under rotation and non-uniform scale, never an
/// under-estimate. The same reduction wok-physics makes; reading wok-scene's unit convention keeps
/// it from drifting from the collider bounds picking will classify later.
pub fn world_aabb(primitive: Primitive, transform: Mat4) -> Aabb {
    let local = primitive.unit_aabb();
    let corners = [
        Vec3::new(local.min.x, local.min.y, local.min.z),
        Vec3::new(local.max.x, local.min.y, local.min.z),
        Vec3::new(local.min.x, local.max.y, local.min.z),
        Vec3::new(local.max.x, local.max.y, local.min.z),
        Vec3::new(local.min.x, local.min.y, local.max.z),
        Vec3::new(local.max.x, local.min.y, local.max.z),
        Vec3::new(local.min.x, local.max.y, local.max.z),
        Vec3::new(local.max.x, local.max.y, local.max.z),
    ];
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for corner in corners {
        let p = transform.transform_point3(corner);
        min = min.min(p);
        max = max.max(p);
    }
    Aabb::new(min, max)
}

/// Vertically correct a placement so the prefab rests on the terrain surface per its `Rest` policy.
///
/// Corner rest lifts the bounds' bottom to the highest of its five footprint samples (four bottom
/// corners and the centre), so a flat-bottomed shape sits on the highest ground it covers and no
/// corner sinks. Center rest drops the bottom to the terrain sample under the placement centre,
/// minus the policy's sink. Either way the difference from the original bottom is the correction. A
/// prefab with no shapes (mesh-only) has no bounds to rest; its origin goes to the terrain sample
/// directly.
pub fn rest_on_terrain(prefab: &Prefab, rest: Rest, transform: Transform, terrain: &Heightmap) -> Transform {
    if default_state_shapes(prefab).is_empty() {
        let ground = terrain.height_at(transform.translation.x, transform.translation.z);
        let translation = Vec3::new(transform.translation.x, ground, transform.translation.z);
        return Transform { translation, ..transform };
    }
    let bounds = prefab_bounds(prefab, &transform);
    let bottom = match rest {
        Rest::Corner => {
            let center = (bounds.min + bounds.max) * 0.5;
            let samples = [
                (bounds.min.x, bounds.min.z),
                (bounds.max.x, bounds.min.z),
                (bounds.min.x, bounds.max.z),
                (bounds.max.x, bounds.max.z),
                (center.x, center.z),
            ];
            samples
                .iter()
                .map(|&(x, z)| terrain.height_at(x, z))
                .fold(f32::NEG_INFINITY, f32::max)
        }
        Rest::Center { sink_m } => {
            terrain.height_at(transform.translation.x, transform.translation.z) - sink_m
        }
    };
    Transform { translation: transform.translation + Vec3::Y * (bottom - bounds.min.y), ..transform }
}

/// Conservative AABB of a placed prefab: the union of [`world_aabb`] over its default state's
/// shapes, composed `placement * shape` exactly as the slicer composes them.
pub fn prefab_bounds(prefab: &Prefab, transform: &Transform) -> Aabb {
    let placement = transform.to_mat4();
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for shape in default_state_shapes(prefab) {
        let b = world_aabb(shape.primitive, placement * shape.transform.to_mat4());
        min = min.min(b.min);
        max = max.max(b.max);
    }
    Aabb::new(min, max)
}

/// The shapes of the prefab's default state. Prefab validation (`wok_scene::load_prefab`)
/// guarantees the default state exists for loaded prefabs; a hand-built prefab that violates it
/// rests as if empty rather than panicking mid-frame.
fn default_state_shapes(prefab: &Prefab) -> &[wok_scene::Shape] {
    prefab
        .states
        .iter()
        .find(|s| s.name == prefab.default_state)
        .map_or(&[], |s| s.shapes.as_slice())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wok_scene::{CHUNK_GRID_LEN, PrefabState, Shape, SurfaceTag};

    fn flat_heightmap(height_m: f32) -> Heightmap {
        let raw = Heightmap::meters_to_raw(height_m);
        Heightmap::new(vec![raw; CHUNK_GRID_LEN], vec![SurfaceTag::new("grass")], vec![0; CHUNK_GRID_LEN])
            .expect("flat grid has the right length")
    }

    fn one_shape_prefab(primitive: Primitive, scale: Vec3) -> Prefab {
        Prefab {
            states: vec![PrefabState {
                name: "default".to_string(),
                shapes: vec![Shape {
                    primitive,
                    transform: Transform { scale, ..Transform::IDENTITY },
                    surface: None,
                    is_hitbox: true,
                    is_visible: true,
                }],
                mesh: None,
            }],
            default_state: "default".to_string(),
        }
    }

    fn at(x: f32, z: f32) -> Transform {
        Transform { translation: Vec3::new(x, 0.0, z), ..Transform::IDENTITY }
    }

    #[test]
    fn world_aabb_of_a_scaled_translated_cube_matches_expected_bounds() {
        // Scale 2x (half-extent 0.5 -> 1.0) then translate +10 in x: the conservative box reads as
        // metres around the placement, the unit-primitive convention wok-scene owns.
        let t = Transform { translation: Vec3::new(10.0, 0.0, 0.0), scale: Vec3::splat(2.0), ..Transform::IDENTITY };
        let a = world_aabb(Primitive::Cube, t.to_mat4());
        assert_eq!(a.min, Vec3::new(9.0, -1.0, -1.0));
        assert_eq!(a.max, Vec3::new(11.0, 1.0, 1.0));
    }

    #[test]
    fn corner_rest_puts_the_bounds_bottom_on_flat_ground() {
        let prefab = one_shape_prefab(Primitive::Cube, Vec3::new(2.0, 2.0, 2.0));
        let rested = rest_on_terrain(&prefab, Rest::Corner, at(60.0, 60.0), &flat_heightmap(5.0));
        let bounds = prefab_bounds(&prefab, &rested);
        assert!((bounds.min.y - 5.0).abs() < 1e-3, "bottom at {}", bounds.min.y);
    }

    #[test]
    fn center_rest_sinks_by_the_policy_amount() {
        let prefab = one_shape_prefab(Primitive::Ellipsoid, Vec3::splat(2.0));
        let rested = rest_on_terrain(&prefab, Rest::Center { sink_m: 0.08 }, at(60.0, 60.0), &flat_heightmap(5.0));
        let bounds = prefab_bounds(&prefab, &rested);
        assert!((bounds.min.y - (5.0 - 0.08)).abs() < 1e-3, "bottom at {}", bounds.min.y);
    }

    #[test]
    fn a_shapeless_prefab_rests_its_origin_on_the_sample() {
        let prefab = Prefab {
            states: vec![PrefabState { name: "default".to_string(), shapes: vec![], mesh: None }],
            default_state: "default".to_string(),
        };
        let rested = rest_on_terrain(&prefab, Rest::Corner, at(10.0, 10.0), &flat_heightmap(3.0));
        assert!((rested.translation.y - 3.0).abs() < 1e-3);
    }
}
