//! The precision-kit pins, through the real step: the parameterized jump arc, coyote time, and
//! their interplay with the buffer and the double jump.
//!
//! `crate::constants` pins the derived numbers algebraically; these tests drive `sim::step`
//! itself, so they prove the composition delivers what the parameters promise - the apex arrives
//! at the authored height and time (every jump flies the full arc; the play verdict removed
//! variable height), the coyote window grants a full ground jump without spending the double
//! jump, and a buffered press still lands. A test-only module like `crate::replay` and
//! `crate::air_feel`.

// Exact float comparison is intended where it appears: the coyote grace is zeroed by assignment
// and clamped by `.max(0.0)`, so "exactly 0.0" is the contract, not a tolerance.
#![allow(clippy::float_cmp)]

use glam::Vec3;
use wok_physics::Motion;
use wok_scene::{Aabb, CHUNK_GRID_LEN, Heightmap, SurfaceTag};

use crate::constants::{PLAYER_HEIGHT, SIM_DT};
use crate::jump::JumpLatch;
use crate::sim::{self, Player, StepInput};
use crate::tuning::Tuning;
use crate::world::{ChunkTerrain, World};

const EPS: f32 = 1e-5;

fn flat_world(height_m: f32) -> World {
    let raw = Heightmap::meters_to_raw(height_m);
    let heightmap =
        Heightmap::new(vec![raw; CHUNK_GRID_LEN], vec![SurfaceTag::new("g")], vec![0; CHUNK_GRID_LEN]).unwrap();
    World { statics: vec![], terrains: vec![ChunkTerrain { origin: Vec3::ZERO, heightmap }], ..World::default() }
}

/// Flat terrain at 2m with a 2m crate mid-chunk (top face at y = 4): the walk-off ledge the
/// coyote tests step from.
fn crate_world() -> World {
    let mut world = flat_world(2.0);
    world.statics.push(Aabb::new(Vec3::new(63.0, 2.0, 63.0), Vec3::new(65.0, 4.0, 65.0)).into());
    world
}

/// A player standing at rest mid-chunk on the flat terrain, carrying the coyote grace any
/// grounded step would.
fn at_rest(world: &World) -> Player {
    let ground = world.terrains[0].heightmap.height_at(64.0, 64.0);
    let t = Tuning::default();
    Player {
        motion: Motion { position: Vec3::new(64.0, ground + PLAYER_HEIGHT * 0.5, 64.0), velocity: Vec3::ZERO },
        grounded: true,
        air_jumps: t.air_jumps,
        coyote: t.coyote_s,
    }
}

fn idle() -> StepInput {
    StepInput { move_dir: Vec3::ZERO, jump: false }
}

fn press() -> StepInput {
    StepInput { move_dir: Vec3::ZERO, jump: true }
}

/// Jump from rest and ride the arc to landing. Returns the trajectory, one entry per step, ending
/// on the landing step.
fn jump_trajectory(world: &World) -> Vec<Player> {
    let t = Tuning::default();
    let mut p = sim::step(at_rest(world), press(), world, &t);
    let mut trajectory = vec![p];
    while !p.grounded && trajectory.len() < 240 {
        p = sim::step(p, idle(), world, &t);
        trajectory.push(p);
    }
    assert!(p.grounded, "every jump test arc must land again");
    trajectory
}

/// The highest climb over the start, and the 1-based step it was sampled on.
fn apex_of(start_y: f32, trajectory: &[Player]) -> (f32, u32) {
    let mut apex = f32::NEG_INFINITY;
    let mut apex_step = 0;
    for (i, p) in trajectory.iter().enumerate() {
        let h = p.motion.position.y - start_y;
        if h > apex {
            apex = h;
            apex_step = i as u32 + 1;
        }
    }
    (apex, apex_step)
}

// ---- the parameterized arc ----

#[test]
fn a_jump_reaches_the_authored_apex_at_the_authored_time() {
    // The derivation made flesh: stepping the real simulation, every jump must peak at
    // JUMP_APEX_HEIGHT when JUMP_TIME_TO_APEX has elapsed. The integrator reproduces the constant
    // -gravity parabola exactly, so the only slack is sampling: the true apex falls between fixed
    // steps, at most half a step from the nearest sample (under 3mm of height here).
    let world = flat_world(2.0);
    let start_y = at_rest(&world).motion.position.y;
    let trajectory = jump_trajectory(&world);
    let (apex, apex_step) = apex_of(start_y, &trajectory);
    let t = Tuning::default();
    assert!((apex - t.jump_apex_height).abs() < 0.01, "apex {apex} vs authored {}", t.jump_apex_height);
    let apex_time = apex_step as f32 * SIM_DT;
    assert!(
        (apex_time - t.jump_time_to_apex).abs() <= SIM_DT,
        "apex at {apex_time}s vs authored {}s",
        t.jump_time_to_apex
    );
}

#[test]
fn the_fall_is_shorter_than_the_rise() {
    // FALL_GRAVITY_MULT's observable promise: the descent from the apex back to the ground takes
    // fewer steps than the climb took, so the arc commits to its landing instead of floating
    // down the way it went up.
    let world = flat_world(2.0);
    let start_y = at_rest(&world).motion.position.y;
    let trajectory = jump_trajectory(&world);
    let (_, apex_step) = apex_of(start_y, &trajectory);
    let descent_steps = trajectory.len() as u32 - apex_step;
    assert!(
        descent_steps < apex_step,
        "descent ({descent_steps} steps) should undercut the rise ({apex_step} steps)"
    );
}

// ---- coyote time ----

/// Settle on the crate top, then walk +x until the step reports airborne: the canonical walk-off.
fn walk_off_the_crate(world: &World) -> Player {
    let t = Tuning::default();
    let mut p = Player {
        motion: Motion { position: Vec3::new(64.0, 4.0 + PLAYER_HEIGHT * 0.5 + 0.05, 64.0), velocity: Vec3::ZERO },
        grounded: false,
        air_jumps: t.air_jumps,
        coyote: 0.0,
    };
    for _ in 0..60 {
        p = sim::step(p, idle(), world, &t);
    }
    assert!(p.grounded, "fixture: should stand on the crate top");
    let run = StepInput { move_dir: Vec3::X, jump: false };
    for _ in 0..120 {
        p = sim::step(p, run, world, &t);
        if !p.grounded {
            return p;
        }
    }
    panic!("fixture: never walked off the crate");
}

#[test]
fn walking_off_a_ledge_leaves_the_full_jump_available_for_the_window() {
    // The forgiveness itself: just past the edge the player is airborne, but a press inside
    // COYOTE_S still fires the full ground jump - launch velocity unscaled - and the air jump is
    // untouched, so the double jump still follows.
    let world = crate_world();
    let t = Tuning::default();
    let mut p = walk_off_the_crate(&world);
    assert!(p.coyote > 0.0, "leaving the ground without jumping must open the coyote window");

    p = sim::step(p, press(), &world, &t);
    assert!(
        (p.motion.velocity.y - (t.jump_velocity() - t.ascent_gravity() * SIM_DT)).abs() < EPS,
        "the coyote jump is the full ground jump, not the scaled air jump: {}",
        p.motion.velocity.y
    );
    assert_eq!(p.air_jumps, t.air_jumps, "the coyote jump must not spend the air jump");
    assert_eq!(p.coyote, 0.0, "a jump consumes the coyote grace immediately");

    // The double jump is still in hand after the coyote jump.
    p = sim::step(p, press(), &world, &t);
    assert_eq!(p.air_jumps, t.air_jumps - 1, "the air jump should fire after a coyote jump");
}

#[test]
fn the_coyote_window_expires_into_the_air_jump() {
    // Past the window the forgiveness is over: a press falls through to the air jump (scaled
    // velocity, one spent), exactly as if the player had jumped off the edge normally.
    let world = crate_world();
    let t = Tuning::default();
    let mut p = walk_off_the_crate(&world);
    let expiry = (t.coyote_s / SIM_DT).ceil() as u32 + 1;
    for _ in 0..expiry {
        p = sim::step(p, idle(), &world, &t);
    }
    assert!(!p.grounded, "fixture: must still be falling when the window closes");
    assert_eq!(p.coyote, 0.0, "the window must have expired");

    p = sim::step(p, press(), &world, &t);
    assert!(
        (p.motion.velocity.y - (t.jump_velocity() * t.air_jump_scale - t.ascent_gravity() * SIM_DT)).abs() < EPS,
        "past the window the press must spend the air jump: {}",
        p.motion.velocity.y
    );
    assert_eq!(p.air_jumps, t.air_jumps - 1);
}

#[test]
fn a_ground_jump_consumes_the_coyote_grace_immediately() {
    // The no-stacking rule: without consuming the grace, a grounded jump would leave COYOTE_S of
    // "still allowed to ground jump" hanging in the air, and a quick second press would pogo a
    // free full jump. The second press must spend the air jump instead.
    let world = flat_world(2.0);
    let t = Tuning::default();
    let p = sim::step(at_rest(&world), press(), &world, &t);
    assert!(!p.grounded, "the jump leaves the ground");
    assert_eq!(p.coyote, 0.0, "jumping must close the window the grounded state had open");

    let p = sim::step(p, press(), &world, &t);
    assert!(
        (p.motion.velocity.y - (t.jump_velocity() * t.air_jump_scale - t.ascent_gravity() * SIM_DT)).abs() < EPS,
        "the immediate second press must be the air jump, not a stacked ground jump: {}",
        p.motion.velocity.y
    );
    assert_eq!(p.air_jumps, t.air_jumps - 1);
}

#[test]
fn the_latch_honors_the_coyote_window() {
    // The latch and the step share Player::can_jump, so a press latched on a zero-step frame
    // during the window must fire - even with the air jump already spent - and an expired window
    // with nothing in hand must buffer instead.
    let t = Tuning::default();
    let in_window = Player {
        motion: Motion { position: Vec3::new(64.0, 10.0, 64.0), velocity: Vec3::ZERO },
        grounded: false,
        air_jumps: 0,
        coyote: t.coyote_s * 0.5,
    };
    let mut latch = JumpLatch::new();
    latch.press();
    assert!(latch.consume(in_window.can_jump(), &t), "the window alone must let the latch fire");

    let expired = Player { coyote: 0.0, ..in_window };
    latch.press();
    assert!(!latch.consume(expired.can_jump(), &t), "expired and spent: the press must wait for landing");
}

// ---- the buffer, end to end ----

#[test]
fn a_buffered_press_fires_on_the_landing_step_through_the_real_step() {
    // The brief's "buffered press still lands", driven the way the app drives it: falling with
    // the air jump spent and the grace expired, a press a few steps before touchdown ages in the
    // latch and fires on the first grounded step, becoming the next jump. The 0.2m drop lands in
    // 3-4 steps, safely inside the 6-step buffer - clear of the boundary, where accumulated
    // SIM_DT roundoff would decide the outcome.
    let world = flat_world(2.0);
    let t = Tuning::default();
    let mut p = Player {
        motion: Motion {
            position: Vec3::new(64.0, 2.0 + PLAYER_HEIGHT * 0.5 + 0.2, 64.0),
            velocity: Vec3::new(0.0, -3.0, 0.0),
        },
        grounded: false,
        air_jumps: 0,
        coyote: 0.0,
    };
    let mut latch = JumpLatch::new();
    latch.press();

    for _ in 0..20 {
        if latch.consume(p.can_jump(), &t) {
            assert!(p.grounded, "nothing in hand: the press may only fire once landed");
            p = sim::step(p, press(), &world, &t);
            assert!(p.motion.velocity.y > 0.0, "the buffered press must become the landing jump");
            return;
        }
        p = sim::step(p, idle(), &world, &t);
    }
    panic!("the buffered press never fired: landing took too long or the buffer dropped it");
}
