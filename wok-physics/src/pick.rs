//! Ray vs the static [`Collider`] vocabulary: the one query the editor's 3D viewport picking needs.
//!
//! The game never casts a ray - it sweeps shapes - so this is the editor's alone: a left-click in the
//! viewport becomes a world-space ray (the eye, and a direction through the clicked pixel), and
//! [`ray_collider`] returns the distance at which that ray first enters a placement's collider, so the
//! nearest hit across the scene is the picked instance. It runs over the colliders
//! [`classify_collider`] produces, so a pick agrees exactly with what each placement collides as.
//!
//! [`ray_collider`] is total and closed, the same shape as [`Collider::contains`]: a hit strictly
//! ahead returns its entry `t`; a ray starting inside returns `0.0`; a collider entirely behind, a
//! clean miss, or a ray parallel to and outside a bounding slab returns `None`. `dir` is expected
//! non-zero and normalized, so the returned `t` reads as world distance.
//!
//! Per shape the closed forms mirror the rest of the crate (no iteration, exact to float arithmetic):
//! the box is the slab method (the oriented box runs that slab in the box's own frame, the rigid map
//! carrying `t` back unchanged); the sphere is the entering quadratic root, the same end-sphere math
//! as [`crate::sweep_round`]; the vertical cylinder intersects the infinite-cylinder radial quadratic
//! with the cap slab.
//!
//! Determinism (canon contract): fixed arithmetic, no RNG, no parallelism; every arm reads only
//! relative positions, so the query is position-independent to float precision.
//!
//! [`classify_collider`]: crate::classify_collider
//! [`Collider::contains`]: crate::collider::Collider::contains

use glam::Vec3;
use wok_scene::Aabb;

use crate::collider::Collider;

/// Below this absolute component magnitude a normalized `dir` is treated as parallel to that slab's
/// axis: the slab then imposes no entry/exit time, only a stay-inside test. Guards the `1 / d` that
/// would otherwise blow up (or yield `0 * inf = NaN`) for an exactly-axis-parallel ray.
const PARALLEL_EPS: f32 = 1e-8;

/// The smallest `t >= 0` at which the ray `origin + dir * t` first enters `collider`, else `None`.
/// `dir` is expected non-zero and normalized, so `t` reads as world distance. A hit strictly ahead
/// returns its entry `t`; an origin already inside returns `0.0`; a collider entirely behind, missed,
/// or only grazed-parallel-and-outside returns `None`. See the module docs for the per-shape method.
pub fn ray_collider(origin: Vec3, dir: Vec3, collider: &Collider) -> Option<f32> {
    match *collider {
        Collider::Aabb(aabb) => ray_aabb(origin, dir, aabb),
        Collider::Sphere { center, radius } => ray_sphere(origin, dir, center, radius),
        Collider::VertCylinder { center, radius, half_height } => {
            ray_vert_cylinder(origin, dir, center, radius, half_height)
        }
        Collider::Obb { center, half_extents, rotation } => {
            // Into the box's own frame (a unit quaternion's conjugate is its exact inverse), where the
            // oriented box is an origin-centred AABB. The map is rigid, so the box-frame entry time is
            // the world entry time and the direction stays unit - `t` still reads as world distance.
            let inv = rotation.conjugate();
            ray_aabb(inv * (origin - center), inv * dir, Aabb::new(-half_extents, half_extents))
        }
    }
}

/// Ray vs an axis-aligned box by the slab method: intersect the per-axis entry/exit intervals and
/// take the near hit. An axis the ray runs parallel to (`|d|` below [`PARALLEL_EPS`]) contributes no
/// time bound, only a miss when the origin already lies outside that axis's span.
fn ray_aabb(origin: Vec3, dir: Vec3, aabb: Aabb) -> Option<f32> {
    let mut t_near = f32::NEG_INFINITY;
    let mut t_far = f32::INFINITY;
    let axes = [
        (origin.x, dir.x, aabb.min.x, aabb.max.x),
        (origin.y, dir.y, aabb.min.y, aabb.max.y),
        (origin.z, dir.z, aabb.min.z, aabb.max.z),
    ];
    for (o, d, lo, hi) in axes {
        if d.abs() <= PARALLEL_EPS {
            // Parallel to this slab: the ray enters only if the origin already lies within it.
            if o < lo || o > hi {
                return None;
            }
        } else {
            let inv = 1.0 / d;
            let (t0, t1) = ((lo - o) * inv, (hi - o) * inv);
            t_near = t_near.max(t0.min(t1));
            t_far = t_far.min(t0.max(t1));
        }
    }
    // Empty intersection, or the whole box lies behind the origin.
    if t_near > t_far || t_far < 0.0 {
        return None;
    }
    // A negative t_near (with t_far >= 0) means the origin is inside: it has already entered, so 0.
    Some(t_near.max(0.0))
}

/// Ray vs a sphere: the entering root of `|origin + dir t - center|^2 = radius^2`. An origin inside
/// (or on) the surface has already entered, so it returns `0.0`; outside, the smaller root is the
/// entry, and a negative one means the whole sphere is behind. Mirrors [`crate::sweep_round`]'s
/// end-sphere quadratic.
fn ray_sphere(origin: Vec3, dir: Vec3, center: Vec3, radius: f32) -> Option<f32> {
    let m = origin - center;
    let c = m.length_squared() - radius * radius;
    if c <= 0.0 {
        // Inside or on the surface: already entered.
        return Some(0.0);
    }
    // Outside (c > 0): both roots share a sign, so the smaller root being non-negative is the entry
    // and being negative means the sphere is entirely behind. `dir` is normalized, so `a` is ~1.
    let a = dir.length_squared();
    let b = 2.0 * m.dot(dir);
    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 {
        return None;
    }
    let t = (-b - disc.sqrt()) / (2.0 * a);
    (t >= 0.0).then_some(t)
}

/// Ray vs a solid vertical (y-axis) cylinder: the intersection of the infinite-cylinder radial
/// constraint (a 2D ray-vs-circle quadratic in the xz plane) and the end-cap slab in y. The near
/// edge of the intersected interval is the entry; an origin inside returns `0.0`.
fn ray_vert_cylinder(origin: Vec3, dir: Vec3, center: Vec3, radius: f32, half_height: f32) -> Option<f32> {
    let mut t_near = f32::NEG_INFINITY;
    let mut t_far = f32::INFINITY;

    // Radial constraint: the infinite cylinder of `radius` about center's vertical axis, in xz.
    let ox = origin.x - center.x;
    let oz = origin.z - center.z;
    let a = dir.x * dir.x + dir.z * dir.z;
    let c = ox * ox + oz * oz - radius * radius;
    if a <= PARALLEL_EPS {
        // A vertical ray never changes its radial distance: outside the wall is a clean miss
        // (parallel to and outside the curved slab); inside imposes no radial time bound.
        if c > 0.0 {
            return None;
        }
    } else {
        let b = 2.0 * (ox * dir.x + oz * dir.z);
        let disc = b * b - 4.0 * a * c;
        if disc < 0.0 {
            return None;
        }
        let sqrt_disc = disc.sqrt();
        t_near = t_near.max((-b - sqrt_disc) / (2.0 * a));
        t_far = t_far.min((-b + sqrt_disc) / (2.0 * a));
    }

    // Cap constraint: the horizontal slab between the two end caps.
    let (lo, hi) = (center.y - half_height, center.y + half_height);
    if dir.y.abs() <= PARALLEL_EPS {
        if origin.y < lo || origin.y > hi {
            return None;
        }
    } else {
        let inv = 1.0 / dir.y;
        let (t0, t1) = ((lo - origin.y) * inv, (hi - origin.y) * inv);
        t_near = t_near.max(t0.min(t1));
        t_far = t_far.min(t0.max(t1));
    }

    if t_near > t_far || t_far < 0.0 {
        return None;
    }
    Some(t_near.max(0.0))
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use glam::Quat;
    use std::f32::consts::{FRAC_PI_4, SQRT_2};

    // ---- Aabb ----

    fn unit_box(center: Vec3) -> Collider {
        Collider::Aabb(Aabb::from_center_extents(center, Vec3::ONE))
    }

    #[test]
    fn ray_enters_an_aabb_at_its_near_face() {
        // Box centred at x = 5 spanning [4, 6]; a +X ray from the origin enters the near face at x = 4.
        let t = ray_collider(Vec3::ZERO, Vec3::X, &unit_box(Vec3::new(5.0, 0.0, 0.0))).expect("hits");
        assert!((t - 4.0).abs() < 1e-5, "near face at x = 4: t = {t}");
    }

    #[test]
    fn ray_inside_an_aabb_enters_at_zero() {
        let t = ray_collider(Vec3::new(5.0, 0.0, 0.0), Vec3::X, &unit_box(Vec3::new(5.0, 0.0, 0.0)));
        assert_eq!(t, Some(0.0));
    }

    #[test]
    fn ray_behind_or_clearing_an_aabb_misses() {
        let b = unit_box(Vec3::new(5.0, 0.0, 0.0));
        // Entirely behind the origin.
        assert!(ray_collider(Vec3::ZERO, Vec3::NEG_X, &b).is_none(), "behind misses");
        // An oblique ray that rises over the top before reaching the box's x-span.
        let up = Vec3::new(1.0, 1.0, 0.0).normalize();
        assert!(ray_collider(Vec3::ZERO, up, &b).is_none(), "rising over the box misses");
    }

    #[test]
    fn ray_parallel_to_a_face_outside_the_slab_misses() {
        // +X is parallel to the y and z slabs; an origin above the y-span never enters.
        let b = unit_box(Vec3::new(5.0, 0.0, 0.0));
        assert!(ray_collider(Vec3::new(0.0, 2.0, 0.0), Vec3::X, &b).is_none());
    }

    // ---- Sphere ----

    fn sphere(center: Vec3, radius: f32) -> Collider {
        Collider::Sphere { center, radius }
    }

    #[test]
    fn ray_enters_a_sphere_at_its_near_surface() {
        // Radius-1 sphere at x = 5; a +X ray meets the near surface at x = 4.
        let t = ray_collider(Vec3::ZERO, Vec3::X, &sphere(Vec3::new(5.0, 0.0, 0.0), 1.0)).expect("hits");
        assert!((t - 4.0).abs() < 1e-5, "near surface at x = 4: t = {t}");
    }

    #[test]
    fn ray_inside_a_sphere_enters_at_zero() {
        assert_eq!(ray_collider(Vec3::new(5.0, 0.0, 0.0), Vec3::X, &sphere(Vec3::new(5.0, 0.0, 0.0), 1.0)), Some(0.0));
    }

    #[test]
    fn ray_behind_or_missing_a_sphere_returns_none() {
        let s = sphere(Vec3::new(5.0, 0.0, 0.0), 1.0);
        assert!(ray_collider(Vec3::ZERO, Vec3::NEG_X, &s).is_none(), "behind misses");
        // Passes with a lateral gap (y = 2 clears the radius 1): the quadratic never reaches zero.
        assert!(ray_collider(Vec3::new(0.0, 2.0, 0.0), Vec3::X, &s).is_none(), "a clear lateral gap misses");
    }

    // ---- VertCylinder ----

    fn cylinder(center: Vec3, radius: f32, half_height: f32) -> Collider {
        Collider::VertCylinder { center, radius, half_height }
    }

    #[test]
    fn ray_enters_a_vert_cylinder_at_its_wall() {
        // Radius-1 cylinder at x = 5, tall enough that the wall is the contact; +X enters at x = 4.
        let t = ray_collider(Vec3::ZERO, Vec3::X, &cylinder(Vec3::new(5.0, 0.0, 0.0), 1.0, 2.0)).expect("hits");
        assert!((t - 4.0).abs() < 1e-5, "near wall at x = 4: t = {t}");
    }

    #[test]
    fn ray_enters_a_vert_cylinder_through_its_cap() {
        // Straight down the axis from y = 10: enters at the top cap (y = 2), a distance of 8.
        let c = cylinder(Vec3::new(5.0, 0.0, 0.0), 1.0, 2.0);
        let t = ray_collider(Vec3::new(5.0, 10.0, 0.0), Vec3::NEG_Y, &c).expect("down the axis hits the cap");
        assert!((t - 8.0).abs() < 1e-5, "top cap at y = 2: t = {t}");
    }

    #[test]
    fn ray_inside_a_vert_cylinder_enters_at_zero() {
        assert_eq!(ray_collider(Vec3::new(5.0, 0.0, 0.0), Vec3::X, &cylinder(Vec3::new(5.0, 0.0, 0.0), 1.0, 2.0)), Some(0.0));
    }

    #[test]
    fn ray_behind_missing_or_parallel_to_a_vert_cylinder_returns_none() {
        let c = cylinder(Vec3::new(5.0, 0.0, 0.0), 1.0, 2.0);
        // Behind the origin.
        assert!(ray_collider(Vec3::ZERO, Vec3::NEG_X, &c).is_none(), "behind misses");
        // A lateral gap past the wall (z = 2 clears the radius 1).
        assert!(ray_collider(Vec3::new(0.0, 0.0, 2.0), Vec3::X, &c).is_none(), "a clear wall gap misses");
        // A vertical ray off the disc: parallel to the cap normal but never over the footprint.
        assert!(ray_collider(Vec3::new(8.0, 10.0, 0.0), Vec3::NEG_Y, &c).is_none(), "a vertical ray off the disc misses");
        // A horizontal ray above the caps: parallel to the cap slab and outside it.
        assert!(ray_collider(Vec3::new(0.0, 5.0, 0.0), Vec3::X, &c).is_none(), "a ray above the caps misses");
    }

    // ---- Obb ----

    fn yawed_box(center: Vec3) -> Collider {
        Collider::Obb { center, half_extents: Vec3::ONE, rotation: Quat::from_rotation_y(FRAC_PI_4) }
    }

    #[test]
    fn ray_enters_an_obb_at_its_rotated_face() {
        // A 45-degree yaw turns the unit box's xz profile to a diamond whose near vertex sits sqrt(2)
        // before the centre, so a +X ray at a box centred at x = 5 enters at x = 5 - sqrt(2).
        let t = ray_collider(Vec3::ZERO, Vec3::X, &yawed_box(Vec3::new(5.0, 0.0, 0.0))).expect("hits");
        assert!((t - (5.0 - SQRT_2)).abs() < 1e-4, "rotated near face: t = {t}");
    }

    #[test]
    fn ray_inside_or_behind_an_obb_matches_the_box_semantics() {
        let o = yawed_box(Vec3::new(5.0, 0.0, 0.0));
        assert_eq!(ray_collider(Vec3::new(5.0, 0.0, 0.0), Vec3::X, &o), Some(0.0), "the centre is inside");
        assert!(ray_collider(Vec3::new(10.0, 0.0, 0.0), Vec3::X, &o).is_none(), "the box is behind");
    }

    #[test]
    fn ray_through_the_world_box_corner_misses_the_rotated_obb() {
        // The conservative world AABB of the yawed box reaches +/- sqrt(2) in x and z; its cut corner
        // is empty. A vertical ray standing in that corner region must miss the rotated solid - the
        // phantom shelf the Obb retired, so picking agrees with collision (cf. collider.rs containment).
        let o = yawed_box(Vec3::ZERO);
        assert!(ray_collider(Vec3::new(1.3, -5.0, 1.3), Vec3::Y, &o).is_none(), "the cut corner is empty");
    }
}
