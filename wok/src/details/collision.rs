//! The inspector's conservative-collision warning: does a placement's rotation outgrow the
//! colliders the engine can carry exactly, so it still collides as a wider axis-aligned box?
//!
//! Cubes rotate honestly (Obb), spheres and vertical cylinders spin onto themselves, and an
//! axis-aligned shape's box is not widened by the reduction - none of those warrant the warning.
//! What remains is the genuinely conservative fall: a rotated shape (tilted round solids, yawed
//! capsules or planes) or a sheared matrix, where the felt surface really is wider than the drawn
//! one. `basis_is_axis_aligned` is wok-physics's own tolerance, so the warning cannot disagree with
//! classification.

use wok_physics::{Collider, basis_is_axis_aligned, classify_collider};
use wok_scene::{Placement, Prefab};

/// Does this placement have a solid shape whose rotation the colliders cannot carry exactly, so it
/// still collides as the conservative axis-aligned box? Surfaced in the inspector only where it is
/// true, next to the transform fields the user is editing.
pub(super) fn has_conservative_rotated_solid(prefab: &Prefab, placement: &Placement) -> bool {
    let state_name = placement.state.as_deref().unwrap_or(prefab.default_state.as_str());
    let Some(state) = prefab.states.iter().find(|s| s.name == state_name) else { return false };
    let placement_mat = placement.transform.to_mat4();
    state.shapes.iter().filter(|s| s.is_hitbox).any(|shape| {
        let world = placement_mat * shape.transform.to_mat4();
        matches!(classify_collider(shape.primitive, world), Collider::Aabb(_)) && !basis_is_axis_aligned(&world)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Quat, Vec3};
    use wok_scene::{InstanceId, PrefabRef, PrefabState, Primitive, Shape, Transform};

    fn solid(primitive: Primitive) -> Prefab {
        Prefab {
            states: vec![PrefabState {
                name: "default".to_string(),
                shapes: vec![Shape {
                    primitive,
                    transform: Transform::IDENTITY,
                    surface: None,
                    is_hitbox: true,
                    is_visible: true,
                }],
                mesh: None,
            }],
            default_state: "default".to_string(),
        }
    }

    fn placed(rotation: Quat, scale: Vec3) -> Placement {
        Placement {
            prefab: PrefabRef::new("p"),
            instance_id: InstanceId(0),
            name: None,
            transform: Transform { translation: Vec3::new(4.0, 1.0, 4.0), rotation, scale },
            state: None,
        }
    }

    #[test]
    fn a_clean_yawed_cube_no_longer_warns() {
        // The Obb carries the yaw exactly: no conservative box, no warning.
        let prefab = solid(Primitive::Cube);
        let yawed = placed(Quat::from_rotation_y(0.6), Vec3::new(2.0, 1.0, 1.5));
        assert!(!has_conservative_rotated_solid(&prefab, &yawed));
    }

    #[test]
    fn axis_aligned_solids_never_warn() {
        // Unrotated shapes collide as their own box (or better); the warning is about rotation
        // the colliders cannot carry, not about placeholder-grade boxes.
        for primitive in [Primitive::Cube, Primitive::Capsule, Primitive::Plane, Primitive::Cylinder] {
            let prefab = solid(primitive);
            let unrotated = placed(Quat::IDENTITY, Vec3::new(2.0, 1.0, 1.5));
            assert!(!has_conservative_rotated_solid(&prefab, &unrotated), "{primitive:?}");
        }
    }

    #[test]
    fn rotated_shapes_that_still_fall_to_the_box_warn() {
        // A yawed capsule and a tilted cylinder have no exact collider: the conservative box
        // genuinely outgrows them, which is exactly what the user should hear.
        let capsule = solid(Primitive::Capsule);
        assert!(has_conservative_rotated_solid(&capsule, &placed(Quat::from_rotation_y(0.6), Vec3::ONE)));
        let cylinder = solid(Primitive::Cylinder);
        assert!(has_conservative_rotated_solid(&cylinder, &placed(Quat::from_rotation_x(0.5), Vec3::ONE)));
    }

    #[test]
    fn a_rotated_round_shape_with_an_exact_collider_does_not_warn() {
        // A yawed upright cylinder classifies exactly (yaw spins it onto itself): no warning.
        let cylinder = solid(Primitive::Cylinder);
        assert!(!has_conservative_rotated_solid(&cylinder, &placed(Quat::from_rotation_y(0.8), Vec3::ONE)));
    }
}
