//! Collide-and-slide: move a capsule through static colliders, sliding along whatever it hits.
//!
//! [`collide_and_slide`] takes a desired displacement and walks it out against the static world (a
//! mixed [`Collider`] set: boxes, spheres, vertical cylinders): sweep to the first impact, advance
//! to just before it, drop the into-surface part of the leftover motion (project it onto the
//! contact plane), and repeat up to a small cap. The capsule ends up flush against the geometry it
//! met, having slid along it rather than stopping dead. Round shapes need nothing extra here: each
//! iteration projects onto the tangent plane at its contact, and on a curved surface successive
//! contacts tilt that plane a little each time, which is exactly what makes a slide along a sphere
//! curve around it.
//!
//! ## Why this returns the slid velocity (part 1 did not)
//!
//! Part 1's AABB resolve left velocity-zeroing to the game, because *whether a resting body keeps
//! its speed* is a policy choice. Sliding is not a policy: projecting the motion onto the contact
//! plane *is* the collision's geometry. The same projection that redirects the displacement is
//! applied to the body's velocity, so the value handed back is the velocity actually consistent
//! with the slide - the game does not have to reconstruct it. The game still owns desired motion,
//! gravity, and jump; it supplies the displacement and velocity, and decides what to do with
//! `grounded`.
//!
//! ## Composition (game-owned)
//!
//! The intended per-step shape, which the game sequences:
//!
//! ```text
//! let next     = integrate(motion, gravity, dt);          // part 1: gravity into velocity + a desired move
//! let capsule  = Capsule::upright(motion.position, height, radius);
//! let slid     = collide_and_slide(capsule, next.position - motion.position, next.velocity, statics, Vec3::Y, cos);
//! motion.position = slid.position;                         // resolved centre
//! motion.velocity = slid.velocity;                         // slid, so next step does not ram the wall
//! ```
//!
//! `grounded` is read from the contacts this move made, so a body needs a downward probe each step
//! (gravity supplies it) to keep reporting grounded while resting on box geometry.
//!
//! Determinism (canon contract): a fixed iteration cap, sequential sweeps in slice order, no RNG,
//! no parallelism; position-independent to float precision.

use glam::Vec3;

use crate::capsule::Capsule;
use crate::collider::Collider;
use crate::sweep::sweep_capsule_colliders_inflated;

/// Small separation kept between the capsule and surfaces while sliding. Stopping a hair short of
/// contact keeps the next sweep's gap cleanly positive, so a capsule flush against a wall reads as
/// "parallel, no impact" and slides freely instead of stalling on a zero-distance contact.
const SKIN: f32 = 1e-3;

/// Below this squared length the leftover motion is negligible and the slide stops.
const MIN_MOVE_SQ: f32 = 1e-10;

/// Cap on slide iterations. Each iteration resolves one contact plane, so this bounds how many
/// distinct surfaces a single move folds in: floor plus a wall plus a second wall (a corner) fits,
/// with a step to spare. Leftover motion past the cap is dropped rather than risk creeping into
/// geometry.
const MAX_ITERS: usize = 4;

/// The outcome of a slide: where the capsule centre ended up, the velocity projected onto every
/// contact plane it met, and whether any contact was walkable ground.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SlideResult {
    /// Resolved capsule-centre position. The net move applied is `position - capsule.center()`; add
    /// that to whatever reference point the game tracks (it can track the centre directly).
    pub position: Vec3,
    /// The input velocity projected onto each contact plane encountered, in slice/iteration order.
    /// With no contact it is the input velocity unchanged.
    pub velocity: Vec3,
    /// True if any contact's normal was within the walkable-slope threshold of `up`.
    pub grounded: bool,
}

/// Move `capsule` by `displacement` through the static `colliders`, sliding along contacts.
///
/// `velocity` is the body's velocity, returned projected onto the same contact planes as the
/// motion. `up` is the world up axis (`Vec3::Y` in chunk-local space) and `walkable_cos` is the
/// cosine of the steepest slope that still counts as ground: a contact is grounding when
/// `normal.dot(up) >= walkable_cos`. Pass `walkable_cos = cos(max_slope_angle)`; the game owns that
/// limit. AABB-only callers wrap their boxes (`Collider::from`); the box behavior is part 1's,
/// unchanged.
///
/// Total over valid inputs: a zero displacement returns the start centre and the velocity
/// unchanged; degenerate capsules (zero radius or zero-length segment) resolve through the sweep
/// without a special case.
pub fn collide_and_slide(
    capsule: Capsule,
    displacement: Vec3,
    velocity: Vec3,
    colliders: &[Collider],
    up: Vec3,
    walkable_cos: f32,
) -> SlideResult {
    let up = if up.length_squared() > 1e-12 { up.normalize() } else { Vec3::Y };
    let mut cap = capsule;
    let mut remaining = displacement;
    let mut velocity = velocity;
    let mut grounded = false;

    for _ in 0..MAX_ITERS {
        if remaining.length_squared() <= MIN_MOVE_SQ {
            break;
        }
        match sweep_capsule_colliders_inflated(&cap, remaining, colliders, SKIN) {
            None => {
                // Nothing in the way: take the whole remaining move and finish.
                cap = cap.translated(remaining);
                break;
            }
            Some(hit) => {
                let advance = remaining * hit.toi;
                cap = cap.translated(advance);
                if hit.normal.dot(up) >= walkable_cos {
                    grounded = true;
                }
                // Drop the part of the motion (and velocity) that pushes into the surface; what is
                // left runs along the contact plane.
                let leftover = remaining - advance;
                remaining = project_on_plane(leftover, hit.normal);
                velocity = project_on_plane(velocity, hit.normal);
            }
        }
    }

    SlideResult { position: cap.center(), velocity, grounded }
}

/// Remove the component of `v` along `normal`, leaving the part that lies in the plane. `normal`
/// must be unit length (the sweep's normals are).
fn project_on_plane(v: Vec3, normal: Vec3) -> Vec3 {
    v - normal * v.dot(normal)
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use crate::motion::{Motion, integrate};
    use wok_scene::Aabb;

    // cos(45 deg): a slope at or below 45 degrees is walkable.
    const WALKABLE_COS: f32 = std::f32::consts::FRAC_1_SQRT_2;

    fn player(feet: Vec3) -> Capsule {
        Capsule::upright(feet + Vec3::new(0.0, 1.0, 0.0), 2.0, 0.5)
    }

    // The part 1 fixtures as colliders: the AABB callers' wrap, applied where the tests built
    // bare boxes before the collider migration. Same boxes, same behavior.
    fn boxed(min: Vec3, max: Vec3) -> Collider {
        Collider::from(Aabb::new(min, max))
    }

    #[test]
    fn unobstructed_move_applies_in_full() {
        let c = player(Vec3::ZERO);
        let d = Vec3::new(2.0, 0.0, 3.0);
        let r = collide_and_slide(c, d, d, &[], Vec3::Y, WALKABLE_COS);
        assert_eq!(r.position, c.center() + d);
        assert_eq!(r.velocity, d);
        assert!(!r.grounded);
    }

    #[test]
    fn moving_into_a_wall_at_an_angle_slides_along_it() {
        // Long wall whose -x face is at x = 2. Moving +x +z diagonally, the capsule should stop
        // advancing in x (centre cannot pass 1.5) and keep moving in z.
        let c = player(Vec3::ZERO);
        let wall = boxed(Vec3::new(2.0, 0.0, -50.0), Vec3::new(3.0, 3.0, 50.0));
        let d = Vec3::new(5.0, 0.0, 5.0);
        let r = collide_and_slide(c, d, d, &[wall], Vec3::Y, WALKABLE_COS);

        // x is blocked at the wall (centre <= face - radius = 1.5), z still progressed.
        assert!(r.position.x <= 1.5 + 1e-3, "x = {}", r.position.x);
        assert!(r.position.z > 1.0, "should have slid along z, z = {}", r.position.z);
        // Velocity ends parallel to the wall: no x, z retained.
        assert!(r.velocity.x.abs() < 1e-3, "vel.x = {}", r.velocity.x);
        assert!((r.velocity.z - 5.0).abs() < 1e-3, "vel.z = {}", r.velocity.z);
        // A vertical wall is not ground.
        assert!(!r.grounded);
    }

    #[test]
    fn sliding_into_a_corner_resolves_against_both_walls() {
        // Two perpendicular walls: faces at x = 2 and z = 2. A diagonal move into the corner must
        // not penetrate either: the centre stops at 1.5 on both axes.
        let c = player(Vec3::ZERO);
        let wall_x = boxed(Vec3::new(2.0, 0.0, -50.0), Vec3::new(3.0, 3.0, 50.0));
        let wall_z = boxed(Vec3::new(-50.0, 0.0, 2.0), Vec3::new(50.0, 3.0, 3.0));
        let d = Vec3::new(5.0, 0.0, 5.0);
        let r = collide_and_slide(c, d, d, &[wall_x, wall_z], Vec3::Y, WALKABLE_COS);

        assert!(r.position.x <= 1.5 + 1e-3, "x penetrated: {}", r.position.x);
        assert!(r.position.z <= 1.5 + 1e-3, "z penetrated: {}", r.position.z);
        // Wedged in the corner: horizontal velocity fully killed.
        assert!(r.velocity.x.abs() < 1e-3, "vel.x = {}", r.velocity.x);
        assert!(r.velocity.z.abs() < 1e-3, "vel.z = {}", r.velocity.z);
    }

    #[test]
    fn resting_on_a_floor_reads_grounded() {
        // Feet at y = 0 on a floor whose top is y = 0; a small downward probe finds the floor.
        let c = player(Vec3::ZERO);
        let floor = boxed(Vec3::new(-50.0, -1.0, -50.0), Vec3::new(50.0, 0.0, 50.0));
        let r = collide_and_slide(c, Vec3::new(0.0, -0.1, 0.0), Vec3::new(0.0, -1.0, 0.0), &[floor], Vec3::Y, WALKABLE_COS);
        assert!(r.grounded, "flat floor should be grounded");
        // The downward velocity is removed by the floor plane.
        assert!(r.velocity.y.abs() < 1e-3, "vel.y = {}", r.velocity.y);
    }

    #[test]
    fn hitting_only_a_wall_is_not_grounded() {
        let c = player(Vec3::ZERO);
        let wall = boxed(Vec3::new(2.0, 0.0, -50.0), Vec3::new(3.0, 3.0, 50.0));
        let r = collide_and_slide(c, Vec3::new(5.0, 0.0, 0.0), Vec3::new(5.0, 0.0, 0.0), &[wall], Vec3::Y, WALKABLE_COS);
        assert!(!r.grounded, "a vertical wall must not count as ground");
    }

    #[test]
    fn does_not_penetrate_a_wall_hit_head_on() {
        let c = player(Vec3::ZERO);
        let wall = boxed(Vec3::new(2.0, 0.0, -50.0), Vec3::new(3.0, 3.0, 50.0));
        let r = collide_and_slide(c, Vec3::new(5.0, 0.0, 0.0), Vec3::new(5.0, 0.0, 0.0), &[wall], Vec3::Y, WALKABLE_COS);
        assert!(r.position.x <= 1.5 + 1e-3, "x = {}", r.position.x);
        assert!(r.velocity.x.abs() < 1e-3, "head-on velocity should be killed, vel.x = {}", r.velocity.x);
    }

    #[test]
    fn slide_is_deterministic() {
        let c = player(Vec3::ZERO);
        let wall = boxed(Vec3::new(2.0, 0.0, -50.0), Vec3::new(3.0, 3.0, 50.0));
        let d = Vec3::new(5.0, -0.2, 5.0);
        let first = collide_and_slide(c, d, d, &[wall], Vec3::Y, WALKABLE_COS);
        let second = collide_and_slide(c, d, d, &[wall], Vec3::Y, WALKABLE_COS);
        assert_eq!(first, second);
    }

    // ---- round colliders ----

    #[test]
    fn sliding_along_a_sphere_curves_around_it() {
        // A sphere just off the path's right side: the first contact's tangent plane deflects the
        // motion left, and each subsequent contact tilts the plane a little further - the slide
        // bends around the ball instead of treating it as a box corner. Walking +x with the sphere
        // centre offset to -z, the deflection must be toward +z, and the velocity must come out
        // bent the same way, not killed.
        let c = player(Vec3::ZERO);
        let sphere = Collider::Sphere { center: Vec3::new(4.0, 1.0, -0.4), radius: 2.0 };
        let d = Vec3::new(5.0, 0.0, 0.0);
        let r = collide_and_slide(c, d, d, &[sphere], Vec3::Y, WALKABLE_COS);

        assert!(r.position.z > 0.2, "should deflect around the sphere's +z side: z = {}", r.position.z);
        assert!(r.position.x > 1.0, "should keep making +x progress around the ball: x = {}", r.position.x);
        assert!(r.velocity.z > 0.5, "velocity should bend around, not die: {:?}", r.velocity);
        // Never inside: the centre keeps the combined radius (2 + 0.5, less the slide skin).
        let gap = (r.position - Vec3::new(4.0, r.position.y.clamp(0.5, 1.5), -0.4)).length();
        assert!(gap >= 2.5 - 2e-3, "penetrated the sphere: centre gap {gap}");
    }

    #[test]
    fn a_head_on_sphere_centerline_hit_stops_dead() {
        // Dead centre, the tangent plane is exactly perpendicular to the motion: nothing to slide
        // along, the same stop a flat wall gives.
        let c = player(Vec3::ZERO);
        let sphere = Collider::Sphere { center: Vec3::new(4.0, 1.0, 0.0), radius: 2.0 };
        let d = Vec3::new(5.0, 0.0, 0.0);
        let r = collide_and_slide(c, d, d, &[sphere], Vec3::Y, WALKABLE_COS);
        assert!(r.position.x <= 1.5 + 1e-2, "stopped at the combined radius: x = {}", r.position.x);
        assert!(r.velocity.x.abs() < 1e-3, "head-on velocity should be killed: {:?}", r.velocity);
    }

    #[test]
    fn sliding_into_a_cylinder_wall_at_an_angle_keeps_the_along_component() {
        // The vertical wall of a cylinder behaves like a curved wall: the into component dies at
        // contact, the along component survives and carries the capsule past.
        let c = player(Vec3::ZERO);
        let pillar = Collider::VertCylinder { center: Vec3::new(3.0, 1.0, 0.0), radius: 1.0, half_height: 3.0 };
        let d = Vec3::new(4.0, 0.0, 1.5);
        let r = collide_and_slide(c, d, d, &[pillar], Vec3::Y, WALKABLE_COS);

        assert!(r.position.z > 1.0, "should have slid along the pillar in z: {:?}", r.position);
        assert!(!r.grounded, "a vertical wall is not ground");
        // The centre never enters the combined radius around the axis.
        let radial = Vec3::new(r.position.x - 3.0, 0.0, r.position.z);
        assert!(radial.length() >= 1.5 - 2e-3, "penetrated the pillar wall: {:?}", r.position);
    }

    #[test]
    fn resting_on_a_cylinder_cap_reads_grounded() {
        // Standing on a pillar top: the cap contact has a straight-up normal, so it grounds and
        // spends the downward probe exactly as a box top does.
        let pillar = Collider::VertCylinder { center: Vec3::new(0.0, 1.0, 0.0), radius: 2.0, half_height: 1.0 };
        let c = player(Vec3::new(0.0, 2.0, 0.0)); // feet on the cap at y = 2
        let r = collide_and_slide(c, Vec3::new(0.0, -0.1, 0.0), Vec3::new(0.0, -1.0, 0.0), &[pillar], Vec3::Y, WALKABLE_COS);
        assert!(r.grounded, "the cap should ground");
        assert!(r.velocity.y.abs() < 1e-3, "the downward velocity dies on the cap: {:?}", r.velocity);
    }

    #[test]
    fn sliding_into_a_yawed_wall_runs_along_the_rotated_face() {
        // A long box yawed about Y: the contact plane is the rotated face, so the slide must
        // carry the motion along that face's direction - the velocity comes out perpendicular to
        // the rotated normal, not to a world axis.
        let yaw = 0.4_f32;
        let rotation = glam::Quat::from_rotation_y(yaw);
        let wall = Collider::Obb {
            center: Vec3::new(4.0, 1.5, 0.0),
            half_extents: Vec3::new(1.0, 3.0, 50.0),
            rotation,
        };
        let c = player(Vec3::ZERO);
        let d = Vec3::new(5.0, 0.0, 0.0);
        let r = collide_and_slide(c, d, d, &[wall], Vec3::Y, WALKABLE_COS);

        let face_normal = rotation * Vec3::NEG_X;
        assert!(r.velocity.dot(face_normal).abs() < 1e-3, "velocity must lie in the rotated face: {:?}", r.velocity);
        assert!(r.velocity.length() > 1.0, "the along-face component survives: {:?}", r.velocity);
        // Slid along the face: the face runs along rotation * Z, so z moved while the capsule
        // stayed outside the face plane.
        assert!(r.position.z.abs() > 0.3, "should have slid along the yawed face: {:?}", r.position);
        assert!((r.position - Vec3::new(4.0, r.position.y, 0.0)).dot(face_normal) >= 1.5 - 2e-3, "penetrated the yawed wall");
        assert!(!r.grounded, "a vertical face is not ground however it is yawed");
    }

    #[test]
    fn resting_on_a_yawed_box_top_reads_grounded() {
        // Yaw leaves the top face horizontal: standing on a yawed crate grounds exactly as the
        // axis-aligned one does.
        let crate_top = Collider::Obb {
            center: Vec3::new(0.0, 1.0, 0.0),
            half_extents: Vec3::ONE,
            rotation: glam::Quat::from_rotation_y(0.6),
        };
        let c = player(Vec3::new(0.0, 2.0, 0.0)); // feet on the top at y = 2
        let r = collide_and_slide(c, Vec3::new(0.0, -0.1, 0.0), Vec3::new(0.0, -1.0, 0.0), &[crate_top], Vec3::Y, WALKABLE_COS);
        assert!(r.grounded, "the yawed top should ground");
        assert!(r.velocity.y.abs() < 1e-3, "the downward velocity dies on the top: {:?}", r.velocity);
    }

    #[test]
    fn a_mixed_collider_slide_is_deterministic() {
        let c = player(Vec3::ZERO);
        let statics = [
            boxed(Vec3::new(2.0, 0.0, -50.0), Vec3::new(3.0, 3.0, 50.0)),
            Collider::Sphere { center: Vec3::new(1.0, 1.0, 2.0), radius: 0.8 },
            Collider::VertCylinder { center: Vec3::new(0.0, 1.0, 4.0), radius: 0.7, half_height: 2.0 },
        ];
        let d = Vec3::new(3.0, -0.2, 5.0);
        let first = collide_and_slide(c, d, d, &statics, Vec3::Y, WALKABLE_COS);
        let second = collide_and_slide(c, d, d, &statics, Vec3::Y, WALKABLE_COS);
        assert_eq!(first, second);
    }

    #[test]
    fn a_scripted_step_sequence_reproduces_bitwise() {
        // The game loop shape: each step integrate under gravity, then collide-and-slide the
        // resulting move. A body driven into a floor-and-wall corner over many steps must produce
        // the identical state both runs - the determinism the Level 2 replay harness relies on.
        let gravity = Vec3::new(0.0, -9.8, 0.0);
        let dt = 1.0 / 60.0;
        let floor = boxed(Vec3::new(-50.0, -1.0, -50.0), Vec3::new(50.0, 0.0, 50.0));
        let wall = boxed(Vec3::new(2.0, 0.0, -50.0), Vec3::new(3.0, 3.0, 50.0));
        let statics = [floor, wall];

        let run = || {
            // Centre at (0, 1, 0): feet on the floor, moving +x toward the wall.
            let mut m = Motion { position: Vec3::new(0.0, 1.0, 0.0), velocity: Vec3::new(3.0, 0.0, 0.0) };
            for _ in 0..240 {
                let next = integrate(m, gravity, dt);
                let cap = Capsule::upright(m.position, 2.0, 0.5);
                let slid = collide_and_slide(cap, next.position - m.position, next.velocity, &statics, Vec3::Y, WALKABLE_COS);
                m = Motion { position: slid.position, velocity: slid.velocity };
            }
            m
        };

        assert_eq!(run(), run());
    }
}
