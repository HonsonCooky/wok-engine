//! Terrain rest policy: how a placed prefab is vertically corrected to sit on the heightmap.
//!
//! Shared by the sample generator (which assigns a tuned [`Rest`] per prefab in its table) and the
//! editor's place mode (which derives a [`Rest`] from the prefab's shapes via [`rest_for_prefab`]).
//! Placement policy is application-side by design (the engine provides `height_at` and the
//! lift-only `resolve_heightmap`; what to do with them is authoring intent), so this lives in the
//! wok application rather than an engine crate.

use glam::Vec3;
use wok_physics::{resolve_heightmap, world_aabb};
use wok_scene::{Aabb, HEIGHT_MIN_M, Heightmap, Prefab, Primitive, Transform};

/// How a prefab is rested on the terrain at placement time. Which rest fits is authoring intent
/// about the shape's underside, not derivable from geometry alone; the sample generator records
/// it per prefab, and the editor approximates it per shape ([`rest_for_prefab`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Rest {
    /// AABB corner rest via wok-physics `resolve_heightmap`: the bounds' bottom sits on the
    /// highest footprint sample, so no corner hovers or sinks. Right for flat-bottomed shapes,
    /// whose silhouette really is their bounds.
    Corner,
    /// The bounds' bottom sits on the terrain sample under the placement centre (`height_at`),
    /// sunk `sink_m` so a curved underside meets the ground instead of tangentially perching on
    /// it. Right for round or organic shapes, which corner rest floats on any slope.
    Center { sink_m: f32 },
}

/// Sink for shape-derived centre rest: deep enough that a round underside reads grounded on the
/// sample terrain, shallow enough that small round prefabs do not bury. The sample generator's
/// per-prefab sinks (0.02-0.08) bracket it; a placed prefab's exact contact patch is the user's
/// to tune in the inspector afterwards.
const DERIVED_SINK_M: f32 = 0.05;

/// Derive a rest policy from a prefab's default-state shapes: round-bottomed primitives
/// (ellipsoid, capsule) take centre rest, anything with a flat-bottomed shape (cube, cylinder,
/// plane) takes corner rest. Mixed prefabs go corner: one flat bottom dominates how the whole
/// thing sits.
pub fn rest_for_prefab(prefab: &Prefab) -> Rest {
    let round_only = default_state_shapes(prefab).iter().all(|shape| {
        matches!(shape.primitive, Primitive::Ellipsoid | Primitive::Capsule)
    });
    if round_only { Rest::Center { sink_m: DERIVED_SINK_M } } else { Rest::Corner }
}

/// Vertically correct a placement so the prefab rests on the terrain surface per its `Rest`
/// policy.
///
/// Corner rest: the authored bounds are reduced to a conservative chunk-local AABB (`world_aabb`
/// over the default state's shapes), dropped below the lowest representable terrain, and lifted by
/// the lift-only `resolve_heightmap`; the lifted bottom is exactly the surface under the
/// footprint. Center rest: the bounds' bottom goes to the terrain sample under the placement
/// centre, minus the policy's sink. Either way the difference from the original bottom is the
/// correction. A prefab with no shapes (mesh-only) has no bounds to rest; its origin goes to the
/// terrain sample directly.
pub fn rest_on_terrain(prefab: &Prefab, rest: Rest, transform: Transform, terrain: &Heightmap) -> Transform {
    if default_state_shapes(prefab).is_empty() {
        let ground = terrain.height_at(transform.translation.x, transform.translation.z);
        let translation = Vec3::new(transform.translation.x, ground, transform.translation.z);
        return Transform { translation, ..transform };
    }
    let bounds = prefab_bounds(prefab, &transform);
    let bottom = match rest {
        Rest::Corner => {
            let drop = (HEIGHT_MIN_M - 1.0) - bounds.min.y;
            let dropped = Aabb::new(bounds.min + Vec3::Y * drop, bounds.max + Vec3::Y * drop);
            resolve_heightmap(dropped, terrain).min.y
        }
        Rest::Center { sink_m } => {
            terrain.height_at(transform.translation.x, transform.translation.z) - sink_m
        }
    };
    Transform { translation: transform.translation + Vec3::Y * (bottom - bounds.min.y), ..transform }
}

/// Conservative AABB of a placed prefab: the union of `world_aabb` over its default state's
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
    fn rest_derives_center_for_round_shapes_and_corner_otherwise() {
        let boulder = one_shape_prefab(Primitive::Ellipsoid, Vec3::splat(2.0));
        assert_eq!(rest_for_prefab(&boulder), Rest::Center { sink_m: DERIVED_SINK_M });
        let marker = one_shape_prefab(Primitive::Capsule, Vec3::new(0.8, 2.0, 0.8));
        assert_eq!(rest_for_prefab(&marker), Rest::Center { sink_m: DERIVED_SINK_M });
        for flat in [Primitive::Cube, Primitive::Cylinder, Primitive::Plane] {
            assert_eq!(rest_for_prefab(&one_shape_prefab(flat, Vec3::ONE)), Rest::Corner, "{flat:?}");
        }
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
