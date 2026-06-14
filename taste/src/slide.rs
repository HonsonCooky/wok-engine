//! The player's collide-and-slide over the cylinder sweeps: stop at what you hit, slide along it.
//!
//! The player's collider is a flat-bottomed vertical cylinder (`wok_physics::Cylinder`, swept by
//! `sweep_cylinder_colliders`); the drawn bean stays a capsule, a deliberate visual mismatch
//! documented at the draw site (`crate::app`). This is the plain collide-and-slide: each iteration
//! sweeps the remaining motion to its first contact, advances to that point, and removes the
//! component of both the motion and the velocity that pushes into the contact plane, so the body
//! follows the surface instead of stopping dead or passing through. Walking straight into a wall
//! leaves no tangential component and the body simply stops; a glancing approach keeps the
//! along-wall part and slides. Landing on a surface from above removes the downward component, so
//! the vertical velocity comes to rest on whatever the body lands on (a crate top as much as the
//! ground) - which is what the jump-reset stillness timer reads.
//!
//! One resolve policy lives here: a contact you land on from above - a flat top or a standable
//! slope, prefab faces included - resolves FLAT, killing the body's vertical motion into it as on
//! level ground, so the body rests on the surface instead of sliding down it. That is what lets the
//! body settle on a tilted prefab face (its vertical velocity reaching zero is what the jump reset
//! reads), the same way the terrain rest lets the body stand on any slope. Steeper, near-vertical
//! faces resolve as walls: the body slides along them and gravity still carries it down. The
//! step-up and the wall-stop/friction nuance were stripped with the move to the simple model;
//! nothing is ever walked through.
//!
//! Deterministic: fixed iteration caps, the engine sweep's slice-order contract, fixed arithmetic;
//! no RNG, no state.

use glam::Vec3;
// The loop constants (SKIN, MIN_MOVE_SQ, MAX_ITERS) are the engine slide's own, imported from
// wok_physics::slide rather than restated: this loop runs the engine sweep's shape, and importing
// the canonical values is what keeps the two from drifting.
use wok_physics::slide::{MAX_ITERS, MIN_MOVE_SQ, SKIN};
use wok_physics::{Collider, Cylinder, sweep_cylinder_colliders};

/// The upward tilt at which a contact counts as a surface to rest ON rather than a wall to slide
/// along: a contact whose normal points up by at least this much (its `normal.y`) resolves flat -
/// the body's vertical motion dies into it as on level ground - so the player stands on it instead
/// of sliding down. 0.2 is about a 78-degree face: anything up to genuinely steep is standable (so
/// the body settles and jumps reset there, as on terrain), while near-vertical faces stay walls.
/// Lower it to stand on even steeper faces.
const SUPPORT_NORMAL_Y: f32 = 0.2;

/// The outcome of a player slide: where the body ended up and the resolved velocity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerSlide {
    /// Resolved cylinder-centre position.
    pub position: Vec3,
    /// The input velocity resolved through each contact: a wall projects its normal component out; a
    /// surface landed on from above kills the vertical component too, so a rest settles to zero.
    pub velocity: Vec3,
}

/// Move the player cylinder by `displacement` through the static `colliders`, sliding along
/// whatever it hits. Pure collide-and-slide: advance to each contact, project the leftover motion
/// and the velocity onto the contact plane, repeat to the iteration cap.
pub fn slide_player(cylinder: Cylinder, displacement: Vec3, velocity: Vec3, colliders: &[Collider]) -> PlayerSlide {
    let mut body = cylinder;
    let mut remaining = displacement;
    let mut velocity = velocity;

    for _ in 0..MAX_ITERS {
        if remaining.length_squared() <= MIN_MOVE_SQ {
            break;
        }
        match sweep_cylinder_colliders(&body, remaining, colliders, SKIN) {
            None => {
                body = body.translated(remaining);
                break;
            }
            Some(hit) => {
                let advance = remaining * hit.toi;
                body = body.translated(advance);
                let leftover = remaining - advance;
                if hit.normal.y >= SUPPORT_NORMAL_Y {
                    // A surface to stand on (a flat top or a standable slope, prefab faces
                    // included): the vertical motion dies into the contact exactly as on level
                    // ground (drop +Y first), then the remainder follows the surface. The body
                    // rests instead of sliding down, so its vertical velocity settles - which is
                    // what the jump reset reads.
                    remaining = project_on_plane(remove_vertical(leftover), hit.normal);
                    velocity = project_on_plane(remove_vertical(velocity), hit.normal);
                } else {
                    // A wall (or near-vertical face): slide along it, vertical motion preserved so
                    // gravity still carries the body down it.
                    remaining = project_on_plane(leftover, hit.normal);
                    velocity = project_on_plane(velocity, hit.normal);
                }
            }
        }
    }

    PlayerSlide { position: body.center, velocity }
}

/// Remove the component of `v` along `normal` (the engine slide's projection). `normal` is unit
/// length: the sweep's contact normals are.
fn project_on_plane(v: Vec3, normal: Vec3) -> Vec3 {
    v - normal * v.dot(normal)
}

/// Drop the vertical component, keeping the horizontal: how a floor-ish contact kills the body's
/// descent so it rests on the surface rather than sliding down it.
fn remove_vertical(v: Vec3) -> Vec3 {
    Vec3::new(v.x, 0.0, v.z)
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn the_slide_loop_tuning_is_the_engines() {
        // The constant tie: the names this loop compiles against must be the engine slide module's
        // own values. Trivially true while the imports above stand; the test exists so a later
        // local re-declaration (shadowing the import with a literal) fails here instead of silently
        // forking this loop from the engine's.
        assert_eq!(SKIN, wok_physics::slide::SKIN);
        assert_eq!(MIN_MOVE_SQ, wok_physics::slide::MIN_MOVE_SQ);
        assert_eq!(MAX_ITERS, wok_physics::slide::MAX_ITERS);
    }

    #[test]
    fn unobstructed_motion_travels_the_full_displacement() {
        // Nothing to hit: the body moves by exactly the displacement and the velocity is untouched.
        use crate::constants::{PLAYER_HEIGHT, PLAYER_RADIUS};
        let body = Cylinder::upright(Vec3::new(0.0, 5.0, 0.0), PLAYER_HEIGHT, PLAYER_RADIUS);
        let slid = slide_player(body, Vec3::new(1.0, 0.0, 2.0), Vec3::new(3.0, -1.0, 6.0), &[]);
        assert_eq!(slid.position, Vec3::new(1.0, 5.0, 2.0));
        assert_eq!(slid.velocity, Vec3::new(3.0, -1.0, 6.0));
    }

    #[test]
    fn landing_on_a_box_top_zeroes_the_downward_velocity() {
        // Falling onto a flat top: the contact normal is +Y, so projecting it out leaves the
        // horizontal velocity and removes the descent - the body comes to rest on the surface, and
        // the vertical velocity the stillness timer reads is zero.
        use crate::constants::{PLAYER_HEIGHT, PLAYER_RADIUS};
        let top = wok_scene::Aabb::from_center_extents(Vec3::new(0.0, 0.0, 0.0), Vec3::new(2.0, 1.0, 2.0));
        let colliders = [Collider::from(top)];
        // Start just above the box top (y = 1.0) by the body's half-height plus a hair, falling.
        let start_y = 1.0 + PLAYER_HEIGHT * 0.5 + 0.2;
        let body = Cylinder::upright(Vec3::new(0.0, start_y, 0.0), PLAYER_HEIGHT, PLAYER_RADIUS);
        let slid = slide_player(body, Vec3::new(0.0, -0.5, 0.0), Vec3::new(0.0, -5.0, 0.0), &colliders);
        assert!(slid.velocity.y.abs() < 1e-6, "the descent should be removed at the top: {}", slid.velocity.y);
    }

    #[test]
    fn a_standable_slope_rests_the_body_instead_of_sliding_it_down() {
        // The whole point of the floor-ish resolve: landing on a tilted face (a cube rotated 20
        // degrees, top normal ~0.94 up) must kill the descent rather than leave the body sliding,
        // or its vertical velocity never settles and the jump reset never fires.
        use crate::constants::{PLAYER_HEIGHT, PLAYER_RADIUS};
        use glam::Quat;
        let ramp = Collider::Obb {
            center: Vec3::new(0.0, 0.0, 0.0),
            half_extents: Vec3::splat(2.0),
            rotation: Quat::from_rotation_x(20.0_f32.to_radians()),
        };
        let body = Cylinder::upright(Vec3::new(0.0, 4.0, 0.0), PLAYER_HEIGHT, PLAYER_RADIUS);
        let slid = slide_player(body, Vec3::new(0.0, -1.5, 0.0), Vec3::new(0.0, -5.0, 0.0), &[ramp]);
        assert!(
            slid.velocity.y >= -1e-4,
            "a standable slope must not leave the body sliding down: vy = {}",
            slid.velocity.y
        );
    }
}
