//! Classification from authored hitbox shapes into the [`Collider`] vocabulary.
//!
//! [`classify_collider`] owns the decision of which collider a transformed primitive becomes.
//! Classification lives here in wok-physics rather than in the game because the editor's picking
//! wants the identical reduction: both consumers must agree on what a placement collides as.
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
//! - `Cube` under any rigid rotation and per-axis scale is exactly an oriented box:
//!   [`Collider::Obb`]. An axis-aligned cube stays [`Collider::Aabb`] - the same solid on the
//!   cheaper, longer-tested path - so only genuinely rotated cubes pay for the frame change.
//! - Everything else - other primitives, tilted or non-uniformly scaled round shapes (out of scope
//!   by design), sheared matrices, degenerate scales - falls back to the conservative box via
//!   [`world_aabb`], exactly the part 1 behavior.
//!
//! Both round rules accept an upside-down placement (local up mapped to world down): sphere and
//! vertical cylinder are symmetric about their horizontal mid-plane, so the flipped solid is the
//! same solid. The box rule likewise accepts a reflected basis (a negative authored scale) by
//! flipping one axis: a box is symmetric in each of its own axes.
//!
//! Determinism (canon contract): pure arithmetic of the transform, no RNG, no parallelism. The
//! checks read only the basis columns, never the translation, so classification is
//! position-independent exactly: lifting a chunk-local hitbox to world space by a translation
//! cannot change which shape it classifies as ([`Collider::translated`] carries the result).
//!
//! [`Aabb`]: wok_scene::Aabb

use glam::{Mat3, Quat, Vec3};
use wok_scene::{Mat4, Primitive};

use crate::bounds::world_aabb;
use crate::collider::Collider;

/// Relative tolerance for the classification checks: axis lengths within this fraction of each
/// other count as uniform, and cross-axis components within this fraction of the axis length count
/// as zero. Authored transforms come through `Transform::to_mat4` (TRS), where the checked values
/// are exact up to a few float operations; 1e-4 is far above that noise and far below any
/// authored difference a designer would mean.
const CLASSIFY_TOL: f32 = 1e-4;

/// Below this axis length the shape is flat or empty; classification falls back to the
/// conservative box rather than building a degenerate rotated or round collider.
const MIN_AXIS_LEN: f32 = 1e-6;

/// Classify a transformed primitive (a sliced hitbox's `primitive` and `transform`) into the
/// collider the narrow phase should test: exact rules first, the conservative box otherwise. See
/// the module docs for the rules. Total: every input classifies, nothing errors.
pub fn classify_collider(primitive: Primitive, transform: Mat4) -> Collider {
    let candidate = match primitive {
        Primitive::Ellipsoid => classify_sphere(&transform),
        Primitive::Cylinder => classify_vert_cylinder(&transform),
        Primitive::Cube => classify_obb(&transform),
        _ => None,
    };
    candidate.unwrap_or_else(|| Collider::Aabb(world_aabb(primitive, transform)))
}

/// Are all three of the transform's basis columns parallel to world axes (within the
/// classification tolerance)? True for every unrotated TRS and for quarter-turn rotations, where
/// the conservative AABB is the transformed box itself rather than an over-estimate. Exported for
/// the editor's inspector: its conservative-box warning applies only when this is false (the box
/// genuinely outgrows the shape because of rotation or shear), sharing this exact tolerance so the
/// warning and the classification cannot disagree.
pub fn basis_is_axis_aligned(transform: &Mat4) -> bool {
    let (x, y, z, _) = basis(transform);
    column_axis_aligned(x) && column_axis_aligned(y) && column_axis_aligned(z)
}

/// Is one basis column parallel to a world axis: at most one component is significant.
fn column_axis_aligned(v: Vec3) -> bool {
    let tol = CLASSIFY_TOL * v.length();
    let small = [v.x.abs() <= tol, v.y.abs() <= tol, v.z.abs() <= tol];
    small.iter().filter(|&&s| s).count() >= 2
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

/// A cube under a rigid rotation and per-axis scale: an exact oriented box. The orthogonality
/// guard rejects shear (the solid is then not a box of the columns' lengths), and an axis-aligned
/// basis returns `None` on purpose: the conservative fallback is the box itself there, on the
/// cheaper part 1 path. A reflected basis (negative determinant, from a negative authored scale)
/// flips its z axis - the box is symmetric in each of its own axes, so it is the same solid - so
/// the quaternion is built from a proper rotation.
fn classify_obb(transform: &Mat4) -> Option<Collider> {
    let (x, y, z, translation) = basis(transform);
    let (lx, ly, lz) = (x.length(), y.length(), z.length());
    if lx.min(ly).min(lz) <= MIN_AXIS_LEN {
        return None;
    }
    if !orthogonal(x, y, z) || basis_is_axis_aligned(transform) {
        return None;
    }
    let (xn, yn, mut zn) = (x / lx, y / ly, z / lz);
    if xn.cross(yn).dot(zn) < 0.0 {
        zn = -zn;
    }
    let rotation = Quat::from_mat3(&Mat3::from_cols(xn, yn, zn)).normalize();
    Some(Collider::Obb {
        center: translation,
        half_extents: 0.5 * Vec3::new(lx, ly, lz),
        rotation,
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
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_3, FRAC_PI_4};
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
    fn non_sphere_ellipsoids_fall_back_to_the_conservative_box() {
        // Stretched, tilted (geometrically still a sphere, but tilted round shapes are out of
        // scope by the brief), and degenerate: all the documented conservative box.
        for (rotation, scale) in [
            (Quat::IDENTITY, Vec3::new(2.0, 1.0, 2.0)),
            (Quat::from_rotation_x(FRAC_PI_4), Vec3::splat(2.0)),
            (Quat::IDENTITY, Vec3::ZERO),
        ] {
            let m = trs(Vec3::new(1.0, 2.0, 3.0), rotation, scale);
            let c = classify_collider(Primitive::Ellipsoid, m);
            assert_eq!(c, Collider::Aabb(world_aabb(Primitive::Ellipsoid, m)), "{rotation:?} {scale:?}");
        }
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
    fn elliptical_or_tilted_cylinders_fall_back_to_the_conservative_box() {
        for (rotation, scale) in [
            (Quat::IDENTITY, Vec3::new(2.0, 4.0, 3.0)),
            (Quat::from_rotation_z(FRAC_PI_4), Vec3::new(2.0, 4.0, 2.0)),
        ] {
            let m = trs(Vec3::ZERO, rotation, scale);
            let c = classify_collider(Primitive::Cylinder, m);
            assert_eq!(c, Collider::Aabb(world_aabb(Primitive::Cylinder, m)), "AABB-grade by scope");
        }
    }

    // ---- oriented box rule ----

    #[test]
    fn a_yawed_cube_classifies_as_an_obb_at_its_authored_dimensions() {
        let rotation = Quat::from_rotation_y(FRAC_PI_3);
        let m = trs(Vec3::new(3.0, 1.0, -2.0), rotation, Vec3::new(2.0, 4.0, 6.0));
        match classify_collider(Primitive::Cube, m) {
            Collider::Obb { center, half_extents, rotation: r } => {
                assert!((center - Vec3::new(3.0, 1.0, -2.0)).length() < 1e-5);
                assert!((half_extents - Vec3::new(1.0, 2.0, 3.0)).length() < 1e-5);
                // Same rotation up to quaternion double-cover.
                assert!(r.dot(rotation).abs() > 1.0 - 1e-5, "rotation drifted: {r:?}");
            }
            other => panic!("expected an oriented box, got {other:?}"),
        }
    }

    #[test]
    fn a_tilted_cube_is_an_obb_too() {
        // Any rigid rotation qualifies, not just yaw: the local frame carries the tilt exactly.
        let rotation = Quat::from_euler(glam::EulerRot::YXZ, 0.7, 0.4, 0.2);
        let m = trs(Vec3::ZERO, rotation, Vec3::splat(2.0));
        assert!(
            matches!(classify_collider(Primitive::Cube, m), Collider::Obb { .. }),
            "a tilted cube must classify as an oriented box"
        );
    }

    #[test]
    fn an_axis_aligned_cube_stays_on_the_cheaper_aabb_path() {
        // Unrotated, and quarter-turned (which only permutes the axes): both are exactly their
        // world AABB, so they keep the part 1 path.
        for rotation in [Quat::IDENTITY, Quat::from_rotation_y(FRAC_PI_2)] {
            let m = trs(Vec3::new(1.0, 2.0, 3.0), rotation, Vec3::new(2.0, 1.0, 3.0));
            let c = classify_collider(Primitive::Cube, m);
            assert_eq!(c, Collider::Aabb(world_aabb(Primitive::Cube, m)), "{rotation:?}");
        }
    }

    #[test]
    fn a_reflected_cube_basis_is_the_same_box() {
        // A negative scale reflects the basis; flipping one axis restores a proper rotation of the
        // identical solid, so the classified box must contain the same points.
        let m = trs(Vec3::ZERO, Quat::from_rotation_y(0.5), Vec3::new(2.0, 2.0, -2.0));
        let c = classify_collider(Primitive::Cube, m);
        let Collider::Obb { rotation, half_extents, .. } = c else {
            panic!("expected an oriented box, got {c:?}");
        };
        assert!(rotation.is_normalized());
        assert_eq!(half_extents, Vec3::splat(1.0));
        // A point just inside the rotated +x face is contained either way.
        let inside = Quat::from_rotation_y(0.5) * Vec3::new(0.99, 0.0, 0.0);
        assert!(c.contains(inside));
    }

    #[test]
    fn a_sheared_matrix_falls_back_even_for_a_cube() {
        // Hand-built shear (not reachable through TRS): the solid is a parallelepiped, not a box,
        // so the orthogonality guard must keep the conservative AABB.
        let m = Mat4::from_cols_array(&[
            1.0, 0.0, 0.0, 0.0, //
            0.0, 1.0, 0.0, 0.0, //
            0.6, 0.0, 0.8, 0.0, // a z axis leaning into x, same unit length
            0.0, 0.0, 0.0, 1.0,
        ]);
        for primitive in [Primitive::Cube, Primitive::Ellipsoid] {
            let c = classify_collider(primitive, m);
            assert_eq!(c, Collider::Aabb(world_aabb(primitive, m)), "{primitive:?}");
        }
    }

    // ---- everything else ----

    #[test]
    fn planes_and_capsules_always_classify_as_their_box() {
        for primitive in [Primitive::Plane, Primitive::Capsule] {
            let m = trs(Vec3::new(1.0, 2.0, 3.0), Quat::from_rotation_y(0.6), Vec3::splat(2.0));
            let c = classify_collider(primitive, m);
            assert_eq!(c, Collider::Aabb(world_aabb(primitive, m)), "{primitive:?} must stay AABB-grade");
        }
    }

    #[test]
    fn basis_is_axis_aligned_reads_rotation_not_scale() {
        let aligned = trs(Vec3::new(4.0, 0.0, 1.0), Quat::IDENTITY, Vec3::new(2.0, 1.0, 3.0));
        assert!(basis_is_axis_aligned(&aligned), "scale alone never rotates the basis");
        let quarter = trs(Vec3::ZERO, Quat::from_rotation_y(FRAC_PI_2), Vec3::ONE);
        assert!(basis_is_axis_aligned(&quarter), "a quarter turn only permutes the axes");
        let yawed = trs(Vec3::ZERO, Quat::from_rotation_y(0.3), Vec3::ONE);
        assert!(!basis_is_axis_aligned(&yawed), "a real yaw leaves the axes");
    }

    // ---- position-independence and the lift ----

    #[test]
    fn classification_reads_only_the_basis_so_translation_commutes_exactly() {
        // The chunk lift: classifying the local transform then translating must equal classifying
        // the lifted transform. For the exact shapes that holds bit for bit - radius, height,
        // half-extents, and rotation read only the basis columns (untouched by a translation), and
        // the centre is one add on either path. The box fallback re-derives corners, where the adds
        // associate differently, so the lift of a box is done by translating the local box (as the
        // world reduction does), not by re-classifying a lifted matrix.
        let offset = Vec3::new(1280.0, 0.0, -2560.0);
        let cases = [
            (Primitive::Ellipsoid, Vec3::splat(3.0)),
            (Primitive::Cylinder, Vec3::new(3.0, 4.0, 3.0)),
            (Primitive::Cube, Vec3::new(2.0, 3.0, 4.0)),
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
    fn classification_is_deterministic() {
        for primitive in [Primitive::Cylinder, Primitive::Cube] {
            let m = trs(Vec3::new(5.0, 2.0, 5.0), Quat::from_rotation_y(0.7), Vec3::new(3.0, 4.0, 3.0));
            assert_eq!(classify_collider(primitive, m), classify_collider(primitive, m), "{primitive:?}");
        }
    }
}
