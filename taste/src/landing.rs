//! Genuine-support test for the landing policy: is there really something to stand on?
//!
//! The slide's `grounded` answers "did any contact's normal pass the walkable threshold". That is
//! necessary but not sufficient for a landing: a capsule's rounded bottom grazing the top corner
//! of a crate while the body is beside the crate produces a near-vertical contact normal, and
//! treating that as landed zeroes the fall, lets ground friction hold the body in place, and the
//! player halts in mid-air at roughly box-top height - the walk-off / jump-off halt bug.
//!
//! What separates standing from a corner graze is not the normal but the support: standing means
//! the body's weight line passes through the surface that bears it. For an upright capsule that is
//! a question this module can answer directly from the collider list: is there a bearing surface
//! of some static collider directly under the capsule's axis, at the height its base is resting?
//! Over the corner of a crate the axis has left the box's footprint, so the answer is no, however
//! vertical the graze's normal happens to be.
//!
//! Per collider, the bearing surface under an axis at `(x, z)`:
//!
//! - `Aabb`: the top face, where the footprint contains the axis.
//! - `VertCylinder`: the top cap, where the axis is within the radius.
//! - `Sphere`: the upper hemisphere's height under the axis. The base-to-surface gap grows with
//!   the horizontal offset from the apex, so the vertical tolerance doubles as the bearing window:
//!   near the apex a rested capsule reads supported, further out it reads unsupported and slides
//!   off - which is the feel a boulder should have.
//! - `Obb`: the highest point where the vertical line under the axis leaves the box (a slab test
//!   in the box frame). For a yaw-only box that is the top face over the rotated footprint -
//!   crucially not over the conservative world box's footprint, which is the phantom shelf this
//!   support test exists to deny.
//!
//! Deterministic and position-independent: fixed arithmetic over the collider list in slice order,
//! reading only relative offsets.

use glam::Vec3;
use wok_physics::Collider;

use crate::constants::PLAYER_HEIGHT;

/// Vertical slack between the capsule's base and the bearing surface, in metres. It must cover the
/// slide's resting skin (a millimetre) with margin; on spheres it doubles as the bearing window
/// (see the module docs), where 0.02 puts the edge of support about 20 degrees off a
/// boulder-sized apex.
const SUPPORT_TOLERANCE_M: f32 = 0.02;

/// Is there a static bearing surface directly under the capsule axis at `position` (the capsule
/// centre), within [`SUPPORT_TOLERANCE_M`] of its base? The landing policy requires this alongside
/// the slide's walkable contact: contact says the body touched something ground-like, support says
/// the body is actually over it.
pub fn supported_below(position: Vec3, statics: &[Collider]) -> bool {
    let base = position.y - PLAYER_HEIGHT * 0.5;
    let near = |surface: f32| (base - surface).abs() <= SUPPORT_TOLERANCE_M;
    statics.iter().any(|collider| match *collider {
        Collider::Aabb(ref aabb) => {
            (aabb.min.x..=aabb.max.x).contains(&position.x)
                && (aabb.min.z..=aabb.max.z).contains(&position.z)
                && near(aabb.max.y)
        }
        Collider::Sphere { center, radius } => {
            let d_sq = (position.x - center.x).powi(2) + (position.z - center.z).powi(2);
            d_sq < radius * radius && near(center.y + (radius * radius - d_sq).sqrt())
        }
        Collider::VertCylinder { center, radius, half_height } => {
            let d_sq = (position.x - center.x).powi(2) + (position.z - center.z).powi(2);
            d_sq <= radius * radius && near(center.y + half_height)
        }
        Collider::Obb { center, half_extents, rotation } => {
            obb_top_under(position, center, half_extents, rotation).is_some_and(near)
        }
    })
}

/// The world height where the vertical line under the capsule axis at `position` last leaves the
/// oriented box, or `None` when the line misses it: a slab test in the box frame. The line
/// `(x, t, z)` maps to `l0 + t * d` locally (with `d` the rotated world-up), and each local axis
/// clips the world-y interval `[t_min, t_max]`; the top of the bearing surface under the axis is
/// `t_max`.
fn obb_top_under(position: Vec3, center: Vec3, half_extents: Vec3, rotation: glam::Quat) -> Option<f32> {
    let inv = rotation.conjugate();
    let l0 = inv * (Vec3::new(position.x, 0.0, position.z) - center);
    let d = inv * Vec3::Y;
    let (mut t_min, mut t_max) = (f32::NEG_INFINITY, f32::INFINITY);
    for i in 0..3 {
        if d[i].abs() <= 1e-8 {
            // The line runs parallel to this slab: it must already be inside it.
            if l0[i].abs() > half_extents[i] {
                return None;
            }
        } else {
            let a = (-half_extents[i] - l0[i]) / d[i];
            let b = (half_extents[i] - l0[i]) / d[i];
            t_min = t_min.max(a.min(b));
            t_max = t_max.min(a.max(b));
        }
    }
    (t_min <= t_max).then_some(t_max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::PLAYER_RADIUS;
    use crate::sim::{self, Player, StepInput};
    use crate::world::{ChunkTerrain, World};
    use wok_physics::Motion;
    use wok_scene::{Aabb, CHUNK_GRID_LEN, Heightmap, SurfaceTag};

    /// A capsule centre whose base sits exactly on `surface` at `(x, z)`.
    fn resting_at(x: f32, surface: f32, z: f32) -> Vec3 {
        Vec3::new(x, surface + PLAYER_HEIGHT * 0.5, z)
    }

    // ---- the probe per collider ----

    #[test]
    fn a_box_supports_inside_its_footprint_and_not_past_its_edge() {
        let cube = [Collider::from(Aabb::new(Vec3::new(63.0, 2.0, 63.0), Vec3::new(65.0, 4.0, 65.0)))];
        assert!(supported_below(resting_at(64.0, 4.0, 64.0), &cube), "mid-face is support");
        assert!(supported_below(resting_at(64.99, 4.0, 64.0), &cube), "on the face right at the edge");
        assert!(!supported_below(resting_at(65.05, 4.0, 64.0), &cube), "past the edge is a graze, not support");
        assert!(!supported_below(resting_at(64.0, 4.5, 64.0), &cube), "hovering above the top is not support");
        assert!(!supported_below(resting_at(64.0, 2.0, 65.8), &cube), "beside the box at ground level");
    }

    #[test]
    fn a_cylinder_cap_supports_within_its_radius() {
        let pillar = [Collider::VertCylinder { center: Vec3::new(10.0, 2.0, 10.0), radius: 1.0, half_height: 2.0 }];
        assert!(supported_below(resting_at(10.5, 4.0, 10.0), &pillar), "on the cap");
        assert!(!supported_below(resting_at(11.2, 4.0, 10.0), &pillar), "past the rim");
    }

    #[test]
    fn a_yawed_box_supports_its_rotated_footprint_and_not_the_world_box_corner() {
        // Half-extent 1 yawed 45 degrees about Y, top at y = 3: the footprint is a diamond
        // reaching sqrt(2) along the world axes, while the conservative world box also covers the
        // corner region (x = z = 1.3 off the centre) that lies outside the diamond. That corner is
        // the phantom shelf and must read unsupported; the diamond itself still bears.
        let center = Vec3::new(10.0, 2.0, 10.0);
        let yawed = [Collider::Obb {
            center,
            half_extents: Vec3::ONE,
            rotation: glam::Quat::from_rotation_y(std::f32::consts::FRAC_PI_4),
        }];
        assert!(supported_below(resting_at(10.0, 3.0, 10.0), &yawed), "the top's middle bears");
        // Along the diagonal the rotated box reaches sqrt(2): still on the top.
        assert!(supported_below(resting_at(11.3, 3.0, 10.0), &yawed), "the rotated corner reach bears");
        assert!(
            !supported_below(resting_at(11.3, 3.0, 11.3), &yawed),
            "the conservative world box's corner is the retired phantom shelf"
        );
        assert!(!supported_below(resting_at(10.0, 3.5, 10.0), &yawed), "hovering above the top is not support");
    }

    #[test]
    fn a_sphere_supports_near_its_apex_and_sheds_further_out() {
        // Boulder-sized: radius 1.1, apex at y = 3.1. Resting tangentially at horizontal offset d,
        // the base sits sqrt((r_c + r_s)^2 - d^2) - r_c above the centre; near the apex that is
        // within the tolerance of the surface under the axis, further out the gap exceeds it.
        let (r_s, center) = (1.1_f32, Vec3::new(0.0, 2.0, 0.0));
        let boulder = [Collider::Sphere { center, radius: r_s }];
        let combined = PLAYER_RADIUS + r_s;
        let rested = |d: f32| {
            let bottom_sphere_y = center.y + (combined * combined - d * d).sqrt();
            Vec3::new(d, bottom_sphere_y - PLAYER_RADIUS + PLAYER_HEIGHT * 0.5, 0.0)
        };
        assert!(supported_below(rested(0.0), &boulder), "the apex bears");
        assert!(supported_below(rested(0.2), &boulder), "just off the apex still bears");
        assert!(!supported_below(rested(0.8), &boulder), "well down the flank sheds");
    }

    // ---- the halt bug, reproduced through the real step ----

    /// Flat terrain at 2m with a 2m crate on it: top face at y = 4, +x face at x = 65.
    fn crate_world() -> World {
        let raw = Heightmap::meters_to_raw(2.0);
        let heightmap =
            Heightmap::new(vec![raw; CHUNK_GRID_LEN], vec![SurfaceTag::new("g")], vec![0; CHUNK_GRID_LEN]).unwrap();
        World {
            statics: vec![Aabb::new(Vec3::new(63.0, 2.0, 63.0), Vec3::new(65.0, 4.0, 65.0)).into()],
            terrains: vec![ChunkTerrain { origin: Vec3::ZERO, heightmap }],
        }
    }

    fn base_y(p: &Player) -> f32 {
        p.motion.position.y - PLAYER_HEIGHT * 0.5
    }

    /// Drive `steps` idle fixed steps, asserting the descent never reverses.
    fn descend_idle(mut p: Player, world: &World, steps: usize) -> Player {
        let mut prev_y = p.motion.position.y;
        for i in 0..steps {
            p = sim::step(p, StepInput::default(), world);
            assert!(p.motion.position.y <= prev_y + 1e-5, "step {i}: rose during a descent");
            prev_y = p.motion.position.y;
        }
        p
    }

    #[test]
    fn a_gentle_step_off_the_crate_edge_reaches_the_ground() {
        // The walk-off halt: a tap of input pushes the centre just past the +x edge; momentum dies
        // under friction with the bottom sphere still grazing the top corner. The graze's normal
        // is near vertical, so a contact-only landing policy reads it as ground, zeroes the fall,
        // and the player hangs at box-top height. The descent must instead reach the terrain.
        let world = crate_world();
        let mut p = Player {
            motion: Motion { position: Vec3::new(64.95, 4.0 + PLAYER_HEIGHT * 0.5 + 0.05, 64.0), velocity: Vec3::ZERO },
            grounded: false,
            air_jumps: crate::constants::AIR_JUMPS,
        };
        for _ in 0..60 {
            p = sim::step(p, StepInput::default(), &world);
        }
        assert!(p.grounded && base_y(&p) > 3.9, "fixture: should stand on the crate top: {:?}", p);

        for _ in 0..3 {
            p = sim::step(p, StepInput { move_dir: Vec3::X, jump: false }, &world);
        }
        // Two seconds is generous for a 2m drop (the free fall is 0.4s); before the fix the graze
        // held the body hovering beside the crate for five.
        let p = descend_idle(p, &world, 120);
        let ground = world.terrains[0].heightmap.height_at(p.motion.position.x, p.motion.position.z);
        assert!(p.grounded, "should end standing on the terrain, not hovering: {:?}", p);
        assert!(
            (base_y(&p) - ground).abs() < 1e-2,
            "base {} should reach the ground {} instead of hanging near the crate top",
            base_y(&p),
            ground
        );
    }

    #[test]
    fn a_jump_off_each_crate_descends_continuously_from_every_angle() {
        // The full arc, against every crate size the sample scene places (cube scale = side in
        // metres): jump from the top in eight directions, and for each scan the input-release
        // point from "released on the jump itself" to "held well past the rim" - the decayed
        // momentum then parks the descent anywhere from back on the rim to flush beside the wall
        // or corner to well clear of it, sweeping straight through both halting regimes this scan
        // has caught (the grounded corner graze, and the corner ride that air friction used to pin
        // in place). Two requirements: a grounded frame is legal only where standing is real - on
        // the terrain, or on the crate's actual top footprint (a release on the jump step can
        // drift the body back onto the rim; landing there is standing on the box, not a halt) -
        // and every run must settle on a legal stand within five seconds, which an arc plus a
        // brief roll off a caught corner fits easily and any lasting hover does not.
        let terrain_h = 2.0;
        for &size in &[1.0_f32, 1.5, 2.0] {
            let half = size * 0.5;
            let raw = Heightmap::meters_to_raw(terrain_h);
            let heightmap = Heightmap::new(vec![raw; CHUNK_GRID_LEN], vec![SurfaceTag::new("g")], vec![0; CHUNK_GRID_LEN])
                .unwrap();
            let world = World {
                statics: vec![
                    Aabb::new(Vec3::new(64.0 - half, terrain_h, 64.0 - half), Vec3::new(64.0 + half, terrain_h + size, 64.0 + half))
                        .into(),
                ],
                terrains: vec![ChunkTerrain { origin: Vec3::ZERO, heightmap }],
            };
            let top = terrain_h + size;
            let on_terrain = |p: &Player| (base_y(p) - terrain_h).abs() <= 0.05;
            let on_top = |p: &Player| {
                (base_y(p) - top).abs() <= 0.05
                    && (p.motion.position.x - 64.0).abs() <= half
                    && (p.motion.position.z - 64.0).abs() <= half
            };

            for k in 0..8 {
                let angle = std::f32::consts::TAU * (k as f32 / 8.0);
                let dir = Vec3::new(angle.cos(), 0.0, angle.sin());
                for hold_after in [0usize, 1, 2, 3, 4, 6, 8, 12, 600] {
                    let label = format!("size {size} angle {k} hold {hold_after}");
                    let mut p = Player {
                        motion: Motion { position: Vec3::new(64.0, top + PLAYER_HEIGHT * 0.5 + 0.05, 64.0), velocity: Vec3::ZERO },
                        grounded: false,
                        air_jumps: crate::constants::AIR_JUMPS,
                    };
                    for _ in 0..60 {
                        p = sim::step(p, StepInput::default(), &world);
                    }
                    assert!(p.grounded && base_y(&p) > top - 0.05, "{label}: fixture should stand on the top");

                    // A short run-up, then the jump with the direction still held.
                    for _ in 0..6 {
                        p = sim::step(p, StepInput { move_dir: dir, jump: false }, &world);
                    }
                    p = sim::step(p, StepInput { move_dir: dir, jump: true }, &world);
                    assert!(!p.grounded, "{label}: the jump step must leave the ground");

                    let mut settled = false;
                    for i in 0..300 {
                        let move_dir = if i < hold_after { dir } else { Vec3::ZERO };
                        p = sim::step(p, StepInput { move_dir, jump: false }, &world);
                        if p.grounded {
                            assert!(
                                on_terrain(&p) || on_top(&p),
                                "{label} step {i}: grounded in mid-air at {:?} (base {})",
                                p.motion.position,
                                base_y(&p)
                            );
                            // A legal stand ends the arc (legality was just asserted).
                            settled = true;
                            break;
                        }
                    }
                    assert!(settled, "{label}: never settled on a legal stand - a halt remains at {:?}", p.motion.position);
                }
            }
        }
    }

    #[test]
    fn a_fall_beside_the_crate_corner_does_not_hang_on_it() {
        // The jump-off halt, distilled: a body falling beside the crate within corner reach. The
        // bottom sphere catches the top corner with an upward-enough normal; without genuine
        // support under the axis the body must keep descending to the terrain, not park there.
        let world = crate_world();
        let p = Player {
            motion: Motion { position: Vec3::new(65.2, 5.0, 64.0), velocity: Vec3::ZERO },
            grounded: false,
            air_jumps: crate::constants::AIR_JUMPS,
        };
        let p = descend_idle(p, &world, 120);
        let ground = world.terrains[0].heightmap.height_at(p.motion.position.x, p.motion.position.z);
        assert!(p.grounded, "should end standing on the terrain, not hovering: {:?}", p);
        assert!(
            (base_y(&p) - ground).abs() < 1e-2,
            "base {} should reach the ground {} instead of hanging on the corner",
            base_y(&p),
            ground
        );
    }
}
