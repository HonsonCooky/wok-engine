//! The static collider vocabulary, and the classification from authored hitbox shapes into it.
//!
//! Part 1 reduced every hitbox to a conservative [`Aabb`] ([`crate::bounds::world_aabb`]): correct,
//! but a round prefab collided as its box, so the felt surface was wider than the drawn one. This
//! module names the shapes the narrow phase can now test exactly - [`Collider`] - and owns the
//! decision of which a transformed primitive becomes ([`classify_collider`]). Classification lives
//! here in wok-physics rather than in the game because the editor's picking wants the identical
//! reduction: both consumers must agree on what a placement collides as.
//!
//! ## Classification rules
//!
//! Per primitive, against the transform's three basis columns (for a TRS matrix these are the
//! rotated, scaled axes; their lengths are the scales):
//!
//! - `Ellipsoid` whose axes are uniform in length, untilted (local up still vertical), and
//!   unsheared is exactly a sphere: [`Collider::Sphere`].
//! - `Cylinder` standing upright (local up vertical, local x/z horizontal) with uniform x/z scale
//!   is exactly a vertical cylinder: [`Collider::VertCylinder`]. Yaw is allowed - it spins a round
//!   shape onto itself.
//! - Everything else - other primitives, tilted or non-uniformly scaled round shapes (out of scope
//!   by design), degenerate scales - falls back to the conservative box via `world_aabb`, exactly
//!   the part 1 behavior.
//!
//! Both round rules accept an upside-down placement (local up mapped to world down): sphere and
//! vertical cylinder are symmetric about their horizontal mid-plane, so the flipped solid is the
//! same solid.
//!
//! Determinism (canon contract): pure arithmetic of the transform, no RNG, no parallelism. The
//! checks read only the basis columns, never the translation, so classification is
//! position-independent exactly: lifting a chunk-local hitbox to world space by a translation
//! cannot change which shape it classifies as ([`Collider::translated`] carries the result).
//!
//! [`Aabb`]: wok_scene::Aabb

use glam::Vec3;
use wok_scene::{Aabb, Mat4, Primitive};

use crate::bounds::world_aabb;

/// Relative tolerance for the classification checks: axis lengths within this fraction of each
/// other count as uniform, and cross-axis components within this fraction of the axis length count
/// as zero. Authored transforms come through `Transform::to_mat4` (TRS), where the checked values
/// are exact up to a few float operations; 1e-4 is far above that noise and far below any
/// authored difference a designer would mean.
const CLASSIFY_TOL: f32 = 1e-4;

/// Below this axis length the shape is flat or empty; classification falls back to the
/// conservative box rather than building a degenerate round collider.
const MIN_AXIS_LEN: f32 = 1e-6;

/// A static collision shape in world (or chunk-local: the producer decides the frame) space.
///
/// The narrow phase ([`crate::sweep`], [`crate::slide`]) takes slices of these. `Aabb` is part 1's
/// box, unchanged; the round shapes are exact where the box was conservative.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Collider {
    /// An axis-aligned box: the conservative reduction every shape can fall back to.
    Aabb(Aabb),
    /// A sphere: an ellipsoid that classified as exactly round.
    Sphere { center: Vec3, radius: f32 },
    /// A vertical (y-axis) solid cylinder: `radius` in the horizontal plane, extending
    /// `half_height` above and below `center`. Axis-aligned vertical only, by scope.
    VertCylinder { center: Vec3, radius: f32, half_height: f32 },
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
        }
    }

    /// The same collider translated by `by`: the lift from chunk-local to world space. A pure
    /// translation, so it commutes exactly with classification (see the module docs).
    pub fn translated(&self, by: Vec3) -> Collider {
        match *self {
            Collider::Aabb(aabb) => Collider::Aabb(Aabb::new(aabb.min + by, aabb.max + by)),
            Collider::Sphere { center, radius } => Collider::Sphere { center: center + by, radius },
            Collider::VertCylinder { center, radius, half_height } => {
                Collider::VertCylinder { center: center + by, radius, half_height }
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

/// Classify a transformed primitive (a sliced hitbox's `primitive` and `transform`) into the
/// collider the narrow phase should test: exact rules first, the conservative box otherwise. See
/// the module docs for the rules. Total: every input classifies, nothing errors.
pub fn classify_collider(primitive: Primitive, transform: Mat4) -> Collider {
    let candidate = match primitive {
        Primitive::Ellipsoid => classify_sphere(&transform),
        Primitive::Cylinder => classify_vert_cylinder(&transform),
        _ => None,
    };
    candidate.unwrap_or_else(|| Collider::Aabb(world_aabb(primitive, transform)))
}

/// The transform's basis columns (the images of the local axes) and its translation.
fn basis(transform: &Mat4) -> (Vec3, Vec3, Vec3, Vec3) {
    (
        transform.x_axis.truncate(),
        transform.y_axis.truncate(),
        transform.z_axis.truncate(),
        transform.w_axis.truncate(),
    )
}

/// An ellipsoid that is exactly a sphere: uniform axis lengths, no tilt, no shear. The radius takes
/// the longest axis, so what little the tolerance admits errs outward (conservative), never inward.
fn classify_sphere(transform: &Mat4) -> Option<Collider> {
    let (x, y, z, translation) = basis(transform);
    let (lx, ly, lz) = (x.length(), y.length(), z.length());
    let longest = lx.max(ly).max(lz);
    if lx.min(ly).min(lz) <= MIN_AXIS_LEN {
        return None;
    }
    let uniform = (lx - ly).abs() <= CLASSIFY_TOL * longest && (lx - lz).abs() <= CLASSIFY_TOL * longest;
    if !uniform || !vertical(y, ly) || !orthogonal(x, y, z) {
        return None;
    }
    // Unit primitives are inscribed in the unit cube (half-extent 0.5), so the world radius is
    // half the axis length.
    Some(Collider::Sphere { center: translation, radius: 0.5 * longest })
}

/// A cylinder that is exactly a vertical cylinder: upright, horizontal x/z of uniform length,
/// unsheared. Yaw is admitted (it maps the round wall onto itself). The radius takes the longer of
/// the two horizontal axes - the same outward-erring tolerance slack as the sphere.
fn classify_vert_cylinder(transform: &Mat4) -> Option<Collider> {
    let (x, y, z, translation) = basis(transform);
    let (lx, ly, lz) = (x.length(), y.length(), z.length());
    if lx.min(ly).min(lz) <= MIN_AXIS_LEN {
        return None;
    }
    let upright = vertical(y, ly) && x.y.abs() <= CLASSIFY_TOL * lx && z.y.abs() <= CLASSIFY_TOL * lz;
    let round = (lx - lz).abs() <= CLASSIFY_TOL * lx.max(lz);
    if !upright || !round || !orthogonal(x, y, z) {
        return None;
    }
    Some(Collider::VertCylinder {
        center: translation,
        radius: 0.5 * lx.max(lz),
        half_height: 0.5 * ly,
    })
}

/// Is the transformed local-up axis (anti)parallel to world up? Sign-free: both round shapes are
/// symmetric about their mid-plane, so an upside-down placement is the same solid.
fn vertical(y: Vec3, ly: f32) -> bool {
    y.x.abs() <= CLASSIFY_TOL * ly && y.z.abs() <= CLASSIFY_TOL * ly
}

/// Are the basis columns pairwise orthogonal? Always true for a TRS matrix; this guards the rules
/// against a sheared matrix arriving from somewhere else, where the shape is not what the columns'
/// lengths alone suggest.
fn orthogonal(x: Vec3, y: Vec3, z: Vec3) -> bool {
    let tol = |a: Vec3, b: Vec3| CLASSIFY_TOL * a.length() * b.length();
    x.dot(y).abs() <= tol(x, y) && y.dot(z).abs() <= tol(y, z) && x.dot(z).abs() <= tol(x, z)
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use glam::Quat;
    use std::f32::consts::{FRAC_PI_3, FRAC_PI_4};
    use wok_scene::Transform;

    fn trs(translation: Vec3, rotation: Quat, scale: Vec3) -> Mat4 {
        Transform { translation, rotation, scale }.to_mat4()
    }

    // ---- sphere rule ----

    #[test]
    fn a_uniform_ellipsoid_classifies_as_a_sphere() {
        let m = trs(Vec3::new(3.0, 1.0, -2.0), Quat::IDENTITY, Vec3::splat(4.0));
        let c = classify_collider(Primitive::Ellipsoid, m);
        assert_eq!(c, Collider::Sphere { center: Vec3::new(3.0, 1.0, -2.0), radius: 2.0 });
    }

    #[test]
    fn a_yawed_uniform_ellipsoid_is_still_a_sphere() {
        // Yaw is not tilt: local up stays vertical, and a sphere spun about any axis is itself.
        let m = trs(Vec3::ZERO, Quat::from_rotation_y(FRAC_PI_3), Vec3::splat(2.0));
        match classify_collider(Primitive::Ellipsoid, m) {
            Collider::Sphere { center, radius } => {
                assert!(center.length() < 1e-6);
                assert!((radius - 1.0).abs() < 1e-5);
            }
            other => panic!("expected a sphere, got {other:?}"),
        }
    }

    #[test]
    fn a_non_uniform_ellipsoid_falls_back_to_the_conservative_box() {
        let m = trs(Vec3::new(1.0, 2.0, 3.0), Quat::IDENTITY, Vec3::new(2.0, 1.0, 2.0));
        let c = classify_collider(Primitive::Ellipsoid, m);
        assert_eq!(c, Collider::Aabb(world_aabb(Primitive::Ellipsoid, m)), "stretched: AABB-grade by scope");
    }

    #[test]
    fn a_tilted_uniform_ellipsoid_falls_back_by_scope() {
        // Geometrically still a sphere, but tilted round shapes are out of scope by the brief:
        // the rule asks for no tilt, so the fallback is the documented conservative box.
        let m = trs(Vec3::ZERO, Quat::from_rotation_x(FRAC_PI_4), Vec3::splat(2.0));
        let c = classify_collider(Primitive::Ellipsoid, m);
        assert_eq!(c, Collider::Aabb(world_aabb(Primitive::Ellipsoid, m)));
    }

    #[test]
    fn an_upside_down_uniform_ellipsoid_is_the_same_sphere() {
        // Local up mapped to world down: symmetric solid, same sphere.
        let m = trs(Vec3::Y, Quat::from_rotation_x(std::f32::consts::PI), Vec3::splat(2.0));
        match classify_collider(Primitive::Ellipsoid, m) {
            Collider::Sphere { center, radius } => {
                assert!((center - Vec3::Y).length() < 1e-5);
                assert!((radius - 1.0).abs() < 1e-5);
            }
            other => panic!("expected a sphere, got {other:?}"),
        }
    }

    // ---- vertical cylinder rule ----

    #[test]
    fn an_upright_cylinder_with_uniform_xz_classifies_as_a_vert_cylinder() {
        let m = trs(Vec3::new(5.0, 2.0, 5.0), Quat::IDENTITY, Vec3::new(3.0, 4.0, 3.0));
        let c = classify_collider(Primitive::Cylinder, m);
        assert_eq!(
            c,
            Collider::VertCylinder { center: Vec3::new(5.0, 2.0, 5.0), radius: 1.5, half_height: 2.0 }
        );
    }

    #[test]
    fn a_yawed_upright_cylinder_is_still_a_vert_cylinder() {
        // Yaw spins the round wall onto itself; the cylinder is exactly the same solid.
        let m = trs(Vec3::X, Quat::from_rotation_y(FRAC_PI_3), Vec3::new(2.0, 6.0, 2.0));
        match classify_collider(Primitive::Cylinder, m) {
            Collider::VertCylinder { center, radius, half_height } => {
                assert!((center - Vec3::X).length() < 1e-5);
                assert!((radius - 1.0).abs() < 1e-5);
                assert!((half_height - 3.0).abs() < 1e-5);
            }
            other => panic!("expected a vertical cylinder, got {other:?}"),
        }
    }

    #[test]
    fn an_elliptical_cylinder_falls_back_to_the_conservative_box() {
        let m = trs(Vec3::ZERO, Quat::IDENTITY, Vec3::new(2.0, 4.0, 3.0));
        let c = classify_collider(Primitive::Cylinder, m);
        assert_eq!(c, Collider::Aabb(world_aabb(Primitive::Cylinder, m)), "elliptical: AABB-grade by scope");
    }

    #[test]
    fn a_tilted_cylinder_falls_back_to_the_conservative_box() {
        let m = trs(Vec3::ZERO, Quat::from_rotation_z(FRAC_PI_4), Vec3::new(2.0, 4.0, 2.0));
        let c = classify_collider(Primitive::Cylinder, m);
        assert_eq!(c, Collider::Aabb(world_aabb(Primitive::Cylinder, m)), "tilted: AABB-grade by scope");
    }

    // ---- everything else, degenerate, and shear ----

    #[test]
    fn boxy_primitives_always_classify_as_their_box() {
        for primitive in [Primitive::Cube, Primitive::Plane, Primitive::Capsule] {
            let m = trs(Vec3::new(1.0, 2.0, 3.0), Quat::IDENTITY, Vec3::splat(2.0));
            let c = classify_collider(primitive, m);
            assert_eq!(c, Collider::Aabb(world_aabb(primitive, m)), "{primitive:?} must stay AABB-grade");
        }
    }

    #[test]
    fn a_degenerate_scale_falls_back_rather_than_building_a_flat_round_shape() {
        let m = trs(Vec3::ZERO, Quat::IDENTITY, Vec3::new(0.0, 0.0, 0.0));
        let c = classify_collider(Primitive::Ellipsoid, m);
        assert_eq!(c, Collider::Aabb(world_aabb(Primitive::Ellipsoid, m)));
    }

    #[test]
    fn a_sheared_matrix_falls_back_even_with_uniform_column_lengths() {
        // Hand-built shear (not reachable through TRS): equal-length but non-orthogonal columns.
        // The solid is not a sphere, so the orthogonality guard must reject it.
        let m = Mat4::from_cols_array(&[
            1.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.6, 0.0, 0.8, 0.0, // a z axis leaning into x, same unit length
            0.0, 0.0, 0.0, 1.0,
        ]);
        let c = classify_collider(Primitive::Ellipsoid, m);
        assert_eq!(c, Collider::Aabb(world_aabb(Primitive::Ellipsoid, m)));
    }

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
    fn contains_commutes_with_translation() {
        // The chunk lift again, for containment: testing the lifted point against the lifted
        // collider answers as the local pair does. Relative coordinates, so exact for the round
        // shapes and the box alike.
        let by = Vec3::new(1280.0, -64.0, 2560.0);
        let cases = [
            (Collider::Aabb(Aabb::new(Vec3::ZERO, Vec3::ONE)), Vec3::new(0.5, 0.5, 0.5)),
            (Collider::Sphere { center: Vec3::ONE, radius: 2.0 }, Vec3::new(2.0, 1.0, 1.0)),
            (Collider::VertCylinder { center: Vec3::ZERO, radius: 1.0, half_height: 2.0 }, Vec3::new(0.5, 1.5, 0.0)),
        ];
        for (collider, point) in cases {
            assert_eq!(
                collider.contains(point),
                collider.translated(by).contains(point + by),
                "{collider:?}: containment must survive the lift"
            );
        }
    }

    // ---- position-independence and the lift ----

    #[test]
    fn classification_reads_only_the_basis_so_translation_commutes_exactly() {
        // The chunk lift: classifying the local transform then translating must equal classifying
        // the lifted transform. For the round shapes that holds bit for bit - radius and height
        // read only the basis columns (untouched by a translation), and the centre is one add on
        // either path. The box fallback re-derives corners, where the adds associate differently,
        // so the lift of a box is done by translating the local box (as the world reduction does),
        // not by re-classifying a lifted matrix.
        let offset = Vec3::new(1280.0, 0.0, -2560.0);
        let cases = [
            (Primitive::Ellipsoid, Vec3::splat(3.0)),
            (Primitive::Cylinder, Vec3::new(3.0, 4.0, 3.0)),
        ];
        for (primitive, scale) in cases {
            let local = trs(Vec3::new(5.0, 2.0, 5.0), Quat::from_rotation_y(0.7), scale);
            let lifted = Mat4::from_translation(offset) * local;
            let a = classify_collider(primitive, local).translated(offset);
            let b = classify_collider(primitive, lifted);
            assert_eq!(a, b, "{primitive:?}: the lift must commute with classification");
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
    }

    #[test]
    fn from_aabb_wraps_part_1_boxes_unchanged() {
        let aabb = Aabb::new(Vec3::ZERO, Vec3::ONE);
        assert_eq!(Collider::from(aabb), Collider::Aabb(aabb));
    }

    #[test]
    fn classification_is_deterministic() {
        let m = trs(Vec3::new(5.0, 2.0, 5.0), Quat::from_rotation_y(0.7), Vec3::new(3.0, 4.0, 3.0));
        assert_eq!(
            classify_collider(Primitive::Cylinder, m),
            classify_collider(Primitive::Cylinder, m)
        );
    }
}
