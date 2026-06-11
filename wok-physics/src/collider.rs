//! The static collider vocabulary the narrow phase tests against.
//!
//! Part 1 reduced every hitbox to a conservative [`Aabb`] ([`crate::bounds::world_aabb`]): correct,
//! but a round prefab collided as its box, so the felt surface was wider than the drawn one. This
//! module names the shapes the narrow phase can test exactly; the decision of which a transformed
//! primitive becomes lives in [`crate::classify`] ([`crate::classify_collider`]), shared with the
//! editor so both consumers agree on what a placement collides as.
//!
//! Part 4 adds [`Collider::Obb`], the oriented box: a rotated solid cube no longer collides as the
//! conservative axis-aligned box around it, which reached past the drawn faces and gave the player
//! an invisible standable margin (the phantom-shelf finding). The representation is a centre, the
//! half-extents along the box's own axes, and a unit quaternion `rotation` mapping the box frame
//! into the world. A quaternion rather than a stored basis because the queries need both directions
//! of the map and a unit quaternion's inverse is its conjugate - exact, no matrix inversion - while
//! the per-axis scale lives in `half_extents`, keeping the frame map rigid (which is what lets a
//! contact normal transform back by the same rotation; see [`crate::sweep_obb`]).
//!
//! Determinism (canon contract): every operation here is pure arithmetic of relative coordinates,
//! no RNG, no parallelism, position-independent under [`Collider::translated`].
//!
//! [`Aabb`]: wok_scene::Aabb

use glam::{Quat, Vec3};
use wok_scene::Aabb;

/// A static collision shape in world (or chunk-local: the producer decides the frame) space.
///
/// The narrow phase ([`crate::sweep`], [`crate::slide`]) takes slices of these. `Aabb` is part 1's
/// box, unchanged; the round shapes and the oriented box are exact where that box was conservative.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Collider {
    /// An axis-aligned box: the conservative reduction every shape can fall back to.
    Aabb(Aabb),
    /// A sphere: an ellipsoid that classified as exactly round.
    Sphere { center: Vec3, radius: f32 },
    /// A vertical (y-axis) solid cylinder: `radius` in the horizontal plane, extending
    /// `half_height` above and below `center`. Axis-aligned vertical only, by scope.
    VertCylinder { center: Vec3, radius: f32, half_height: f32 },
    /// An oriented box: `half_extents` along the box's own axes, `rotation` (a unit quaternion)
    /// mapping the box frame into the world. A cube under any rigid rotation and per-axis scale.
    Obb { center: Vec3, half_extents: Vec3, rotation: Quat },
}

impl Collider {
    /// Is `point` inside this collider? Exact per shape (no tolerance), and closed: a point on the
    /// surface is contained, so the answer flips exactly where the solid ends. Pure comparisons of
    /// relative coordinates - deterministic and position-independent like the rest of the crate.
    /// First consumer is the game camera's eye-inside-geometry test; the editor's picking is the
    /// expected second.
    pub fn contains(&self, point: Vec3) -> bool {
        match *self {
            Collider::Aabb(aabb) => {
                point.x >= aabb.min.x && point.x <= aabb.max.x
                    && point.y >= aabb.min.y && point.y <= aabb.max.y
                    && point.z >= aabb.min.z && point.z <= aabb.max.z
            }
            Collider::Sphere { center, radius } => {
                (point - center).length_squared() <= radius * radius
            }
            Collider::VertCylinder { center, radius, half_height } => {
                let d = point - center;
                d.y.abs() <= half_height && d.x * d.x + d.z * d.z <= radius * radius
            }
            Collider::Obb { center, half_extents, rotation } => {
                // The point in the box frame (the conjugate is the unit quaternion's exact
                // inverse), then the axis-aligned test against the half-extents.
                let local = rotation.conjugate() * (point - center);
                local.x.abs() <= half_extents.x
                    && local.y.abs() <= half_extents.y
                    && local.z.abs() <= half_extents.z
            }
        }
    }

    /// The same collider translated by `by`: the lift from chunk-local to world space. A pure
    /// translation, so it commutes exactly with classification (see [`crate::classify`]).
    pub fn translated(&self, by: Vec3) -> Collider {
        match *self {
            Collider::Aabb(aabb) => Collider::Aabb(Aabb::new(aabb.min + by, aabb.max + by)),
            Collider::Sphere { center, radius } => Collider::Sphere { center: center + by, radius },
            Collider::VertCylinder { center, radius, half_height } => {
                Collider::VertCylinder { center: center + by, radius, half_height }
            }
            Collider::Obb { center, half_extents, rotation } => {
                Collider::Obb { center: center + by, half_extents, rotation }
            }
        }
    }
}

/// Part 1's boxes are colliders as they stand: existing AABB producers wrap, nothing re-derives.
impl From<Aabb> for Collider {
    fn from(aabb: Aabb) -> Collider {
        Collider::Aabb(aabb)
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_4;

    // ---- point containment ----

    #[test]
    fn an_aabb_contains_its_interior_faces_and_corners_but_not_beyond() {
        let c = Collider::Aabb(Aabb::new(Vec3::new(-1.0, 0.0, 2.0), Vec3::new(1.0, 4.0, 5.0)));
        assert!(c.contains(Vec3::new(0.0, 2.0, 3.0)), "interior");
        assert!(c.contains(Vec3::new(1.0, 2.0, 3.0)), "a face is the closed boundary");
        assert!(c.contains(Vec3::new(-1.0, 0.0, 2.0)), "the min corner is contained");
        assert!(c.contains(Vec3::new(1.0, 4.0, 5.0)), "the max corner is contained");
        assert!(!c.contains(Vec3::new(1.0 + 1e-5, 2.0, 3.0)), "just past a face in x");
        assert!(!c.contains(Vec3::new(0.0, -1e-5, 3.0)), "just below the floor in y");
        assert!(!c.contains(Vec3::new(0.0, 2.0, 5.0 + 1e-5)), "just past the far face in z");
    }

    #[test]
    fn a_sphere_contains_its_interior_and_surface_but_not_beyond() {
        let c = Collider::Sphere { center: Vec3::new(3.0, 1.0, -2.0), radius: 2.0 };
        assert!(c.contains(Vec3::new(3.0, 1.0, -2.0)), "the centre");
        assert!(c.contains(Vec3::new(3.0, 3.0, -2.0)), "a surface point is contained (closed)");
        // Just inside along a diagonal (an exact diagonal surface point does not exist in float;
        // the axis-aligned one above is the exact boundary case).
        let diag = Vec3::new(3.0, 1.0, -2.0) + Vec3::ONE.normalize() * 1.999;
        assert!(c.contains(diag), "just inside along a diagonal");
        assert!(!c.contains(Vec3::new(3.0, 3.0 + 1e-4, -2.0)), "just past the surface");
        assert!(!c.contains(Vec3::new(3.0 + 1.5, 1.0 + 1.5, -2.0)), "outside along a diagonal");
    }

    #[test]
    fn a_cylinder_contains_its_interior_wall_caps_and_rim_but_not_beyond() {
        let c = Collider::VertCylinder { center: Vec3::new(5.0, 2.0, 5.0), radius: 1.5, half_height: 2.0 };
        assert!(c.contains(Vec3::new(5.0, 2.0, 5.0)), "the centre");
        assert!(c.contains(Vec3::new(6.5, 2.0, 5.0)), "the wall is the closed boundary");
        assert!(c.contains(Vec3::new(5.0, 4.0, 5.0)), "the top cap");
        assert!(c.contains(Vec3::new(5.0, 0.0, 5.0)), "the bottom cap");
        assert!(c.contains(Vec3::new(6.5, 4.0, 5.0)), "the rim: wall and cap at once");
        assert!(!c.contains(Vec3::new(6.5 + 1e-4, 2.0, 5.0)), "just past the wall");
        assert!(!c.contains(Vec3::new(5.0, 4.0 + 1e-5, 5.0)), "just above the top cap");
        assert!(
            !c.contains(Vec3::new(6.0, 4.5, 5.0)),
            "inside the radius but above the cap: a cylinder is not a box"
        );
        assert!(
            !c.contains(Vec3::new(6.2, 2.0, 6.2)),
            "inside the bounding box's corner but outside the round wall"
        );
    }

    #[test]
    fn an_obb_contains_its_rotated_interior_and_boundary_but_not_its_world_box_corner() {
        // A unit-half-extent box yawed 45 degrees: the rotated faces cut the world AABB's corners
        // off. The cut corner region is exactly the phantom shelf the Obb retires, so containment
        // must say no there.
        let c = Collider::Obb {
            center: Vec3::new(2.0, 1.0, 3.0),
            half_extents: Vec3::ONE,
            rotation: Quat::from_rotation_y(FRAC_PI_4),
        };
        assert!(c.contains(Vec3::new(2.0, 1.0, 3.0)), "the centre");
        // Along the rotated +x axis the face is exactly 1 away.
        let face = Vec3::new(2.0, 1.0, 3.0) + Quat::from_rotation_y(FRAC_PI_4) * Vec3::X;
        assert!(c.contains(face), "a rotated face point is the closed boundary");
        assert!(
            !c.contains(face + Quat::from_rotation_y(FRAC_PI_4) * Vec3::X * 1e-3),
            "just past the rotated face"
        );
        // The world AABB of this box spans +/- sqrt(2) in x/z; its corner region is outside the
        // rotated solid.
        assert!(
            !c.contains(Vec3::new(2.0 + 1.3, 1.0, 3.0 + 1.3)),
            "inside the conservative world box's corner but outside the rotated solid"
        );
        assert!(!c.contains(Vec3::new(2.0, 2.0 + 1e-4, 3.0)), "just above the (unrotated) top");
    }

    #[test]
    fn contains_commutes_with_translation() {
        // The chunk lift, for containment: testing the lifted point against the lifted collider
        // answers as the local pair does. Relative coordinates, so exact for every shape.
        let by = Vec3::new(1280.0, -64.0, 2560.0);
        let cases = [
            (Collider::Aabb(Aabb::new(Vec3::ZERO, Vec3::ONE)), Vec3::new(0.5, 0.5, 0.5)),
            (Collider::Sphere { center: Vec3::ONE, radius: 2.0 }, Vec3::new(2.0, 1.0, 1.0)),
            (Collider::VertCylinder { center: Vec3::ZERO, radius: 1.0, half_height: 2.0 }, Vec3::new(0.5, 1.5, 0.0)),
            (
                Collider::Obb { center: Vec3::ZERO, half_extents: Vec3::ONE, rotation: Quat::from_rotation_y(0.7) },
                Vec3::new(0.9, 0.5, 0.3),
            ),
        ];
        for (collider, point) in cases {
            assert_eq!(
                collider.contains(point),
                collider.translated(by).contains(point + by),
                "{collider:?}: containment must survive the lift"
            );
        }
    }

    #[test]
    fn translated_moves_every_variant_and_changes_nothing_else() {
        let by = Vec3::new(2.0, -1.0, 3.0);
        let aabb = Collider::Aabb(Aabb::new(Vec3::ZERO, Vec3::ONE)).translated(by);
        assert_eq!(aabb, Collider::Aabb(Aabb::new(by, Vec3::ONE + by)));
        let sphere = Collider::Sphere { center: Vec3::ONE, radius: 2.0 }.translated(by);
        assert_eq!(sphere, Collider::Sphere { center: Vec3::ONE + by, radius: 2.0 });
        let cyl = Collider::VertCylinder { center: Vec3::ZERO, radius: 1.0, half_height: 2.0 }.translated(by);
        assert_eq!(cyl, Collider::VertCylinder { center: by, radius: 1.0, half_height: 2.0 });
        let rot = Quat::from_rotation_y(0.7);
        let obb = Collider::Obb { center: Vec3::ZERO, half_extents: Vec3::new(1.0, 2.0, 3.0), rotation: rot }
            .translated(by);
        assert_eq!(obb, Collider::Obb { center: by, half_extents: Vec3::new(1.0, 2.0, 3.0), rotation: rot });
    }

    #[test]
    fn from_aabb_wraps_part_1_boxes_unchanged() {
        let aabb = Aabb::new(Vec3::ZERO, Vec3::ONE);
        assert_eq!(Collider::from(aabb), Collider::Aabb(aabb));
    }
}
