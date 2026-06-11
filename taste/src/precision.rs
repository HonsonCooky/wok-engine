//! The precision-kit pins, through the real step: the parameterized jump arc, variable jump
//! height, coyote time, and their interplay with the buffer and the double jump.
//!
//! `crate::constants` pins the derived numbers algebraically; these tests drive `sim::step`
//! itself, so they prove the composition delivers what the parameters promise - the apex arrives
//! at the authored height and time, a release cuts the climb exactly once, the coyote window
//! grants a full ground jump without spending the double jump, and a buffered press still lands.
//! A test-only module like `crate::replay` and `crate::air_feel`.

// Exact float comparison is intended where it appears: the coyote grace is zeroed by assignment
// and clamped by `.max(0.0)`, so "exactly 0.0" is the contract, not a tolerance.
#![allow(clippy::float_cmp)]

use glam::Vec3;
use wok_physics::Motion;
use wok_scene::{Aabb, CHUNK_GRID_LEN, Heightmap, SurfaceTag};

use crate::constants::{
    AIR_JUMP_SCALE, AIR_JUMPS, ASCENT_GRAVITY, COYOTE_S, JUMP_APEX_HEIGHT, JUMP_CUT_FACTOR, JUMP_TIME_TO_APEX,
    JUMP_VELOCITY, PLAYER_HEIGHT, SIM_DT,
};
use crate::jump::JumpLatch;
use crate::sim::{self, Player, StepInput};
use crate::world::{ChunkTerrain, World};

const EPS: f32 = 1e-5;

fn flat_world(height_m: f32) -> World {
    let raw = Heightmap::meters_to_raw(height_m);
    let heightmap =
        Heightmap::new(vec![raw; CHUNK_GRID_LEN], vec![SurfaceTag::new("g")], vec![0; CHUNK_GRID_LEN]).unwrap();
    World { statics: vec![], terrains: vec![ChunkTerrain { origin: Vec3::ZERO, heightmap }] }
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
    Player {
        motion: Motion { position: Vec3::new(64.0, ground + PLAYER_HEIGHT * 0.5, 64.0), velocity: Vec3::ZERO },
        grounded: true,
        air_jumps: AIR_JUMPS,
        coyote: COYOTE_S,
        cut_armed: false,
    }
}

fn idle(held: bool) -> StepInput {
    StepInput { move_dir: Vec3::ZERO, jump: false, jump_held: held }
}

fn press(held: bool) -> StepInput {
    StepInput { move_dir: Vec3::ZERO, jump: true, jump_held: held }
}

/// Jump from rest and ride the arc to landing, holding the control for the press step and the
/// `held_steps - 1` steps after it (0 is a tap: the press itself arrives released). Returns the
/// trajectory, one entry per step, ending on the landing step.
fn jump_trajectory(world: &World, held_steps: u32) -> Vec<Player> {
    let mut p = sim::step(at_rest(world), press(held_steps > 0), world);
    let mut trajectory = vec![p];
    while !p.grounded && trajectory.len() < 240 {
        p = sim::step(p, idle((trajectory.len() as u32) < held_steps), world);
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
fn a_held_jump_reaches_the_authored_apex_at_the_authored_time() {
    // The derivation made flesh: stepping the real simulation, a held jump must peak at
    // JUMP_APEX_HEIGHT when JUMP_TIME_TO_APEX has elapsed. The integrator reproduces the constant
    // -gravity parabola exactly, so the only slack is sampling: the true apex falls between fixed
    // steps, at most half a step from the nearest sample (under 3mm of height here).
    let world = flat_world(2.0);
    let start_y = at_rest(&world).motion.position.y;
    let trajectory = jump_trajectory(&world, u32::MAX);
    let (apex, apex_step) = apex_of(start_y, &trajectory);
    assert!((apex - JUMP_APEX_HEIGHT).abs() < 0.01, "apex {apex} vs authored {JUMP_APEX_HEIGHT}");
    let apex_time = apex_step as f32 * SIM_DT;
    assert!(
        (apex_time - JUMP_TIME_TO_APEX).abs() <= SIM_DT,
        "apex at {apex_time}s vs authored {JUMP_TIME_TO_APEX}s"
    );
}

#[test]
fn the_fall_is_shorter_than_the_rise() {
    // FALL_GRAVITY_MULT's observable promise: the descent from the apex back to the ground takes
    // fewer steps than the climb took, so the arc commits to its landing instead of floating
    // down the way it went up.
    let world = flat_world(2.0);
    let start_y = at_rest(&world).motion.position.y;
    let trajectory = jump_trajectory(&world, u32::MAX);
    let (_, apex_step) = apex_of(start_y, &trajectory);
    let descent_steps = trajectory.len() as u32 - apex_step;
    assert!(
        descent_steps < apex_step,
        "descent ({descent_steps} steps) should undercut the rise ({apex_step} steps)"
    );
}

// ---- variable jump height ----

#[test]
fn releasing_mid_ascent_cuts_the_climb_exactly_once() {
    // The cut's contract, pinned to the velocity: the first released step while rising scales the
    // vertical velocity by JUMP_CUT_FACTOR (one step of ascent gravity follows), and a second
    // release later in the same ascent applies gravity alone - once per jump.
    let world = flat_world(2.0);
    let mut p = sim::step(at_rest(&world), press(true), &world);
    for _ in 0..5 {
        p = sim::step(p, idle(true), &world);
    }
    let vy = p.motion.velocity.y;
    assert!(vy > 0.0, "fixture: must still be rising at the release");
    p = sim::step(p, idle(false), &world);
    assert!(
        (p.motion.velocity.y - (vy * JUMP_CUT_FACTOR - ASCENT_GRAVITY * SIM_DT)).abs() < EPS,
        "the release must scale the climb by the cut factor: {}",
        p.motion.velocity.y
    );

    // Re-hold, then release again while still rising: no second cut.
    p = sim::step(p, idle(true), &world);
    let vy = p.motion.velocity.y;
    assert!(vy > 0.0, "fixture: the cut arc must still be rising for the second release");
    p = sim::step(p, idle(false), &world);
    assert!(
        (p.motion.velocity.y - (vy - ASCENT_GRAVITY * SIM_DT)).abs() < EPS,
        "a second release in the same ascent must apply gravity alone: {}",
        p.motion.velocity.y
    );
}

#[test]
fn an_early_release_lowers_the_apex_and_a_tap_is_the_minimum_hop() {
    // The feel the cut buys: holding sweeps a real range of jump heights. An early release lands
    // well under the full apex; a tap (press already released) is the floor of that range,
    // JUMP_CUT_FACTOR^2 of the full height (the cut scales velocity, apex goes with its square).
    let world = flat_world(2.0);
    let start_y = at_rest(&world).motion.position.y;
    let (full, _) = apex_of(start_y, &jump_trajectory(&world, u32::MAX));
    let (early, _) = apex_of(start_y, &jump_trajectory(&world, 4));
    let (tap, _) = apex_of(start_y, &jump_trajectory(&world, 0));
    assert!(early < 0.7 * full, "an early release ({early}) must land well under the full apex ({full})");
    assert!(tap < early, "the tap ({tap}) is the floor, under the early release ({early})");
    let min_hop = JUMP_CUT_FACTOR * JUMP_CUT_FACTOR * JUMP_APEX_HEIGHT;
    assert!((tap - min_hop).abs() < 0.02, "tap apex {tap} vs predicted minimum hop {min_hop}");
}

#[test]
fn a_release_after_the_apex_changes_nothing() {
    // The cut only ever shortens a climb: once the body is falling, releasing is inert, so the
    // post-release trajectory must be bitwise the held trajectory - the same guarantee replay
    // leans on, applied to the one input that differs.
    let world = flat_world(2.0);
    let held = jump_trajectory(&world, u32::MAX);
    let released_late = jump_trajectory(&world, 30); // past the ~23-step apex, into the descent
    assert_eq!(held.len(), released_late.len(), "the arcs must land on the same step");
    for (i, (a, b)) in held.iter().zip(&released_late).enumerate() {
        assert_eq!(
            a.motion.position.y.to_bits(),
            b.motion.position.y.to_bits(),
            "a post-apex release must not alter the arc (step {i})"
        );
    }
}

#[test]
fn the_air_jump_gets_its_own_cut() {
    // Once per JUMP, not once per airtime: the air jump re-arms the cut, so a release during its
    // ascent scales it like a ground jump's - both halves of the brief's "works for both".
    let world = flat_world(2.0);
    let mut p = sim::step(at_rest(&world), press(true), &world);
    for _ in 0..3 {
        p = sim::step(p, idle(true), &world);
    }
    p = sim::step(p, press(true), &world); // the air jump, still held
    for _ in 0..2 {
        p = sim::step(p, idle(true), &world);
    }
    let vy = p.motion.velocity.y;
    assert!(vy > 0.0, "fixture: the air jump must still be rising at the release");
    p = sim::step(p, idle(false), &world);
    assert!(
        (p.motion.velocity.y - (vy * JUMP_CUT_FACTOR - ASCENT_GRAVITY * SIM_DT)).abs() < EPS,
        "the air jump's ascent must cut like the ground jump's: {}",
        p.motion.velocity.y
    );
}

// ---- coyote time ----

/// Settle on the crate top, then walk +x until the step reports airborne: the canonical walk-off.
fn walk_off_the_crate(world: &World) -> Player {
    let mut p = Player {
        motion: Motion { position: Vec3::new(64.0, 4.0 + PLAYER_HEIGHT * 0.5 + 0.05, 64.0), velocity: Vec3::ZERO },
        grounded: false,
        air_jumps: AIR_JUMPS,
        coyote: 0.0,
        cut_armed: false,
    };
    for _ in 0..60 {
        p = sim::step(p, idle(false), world);
    }
    assert!(p.grounded, "fixture: should stand on the crate top");
    let run = StepInput { move_dir: Vec3::X, jump: false, jump_held: false };
    for _ in 0..120 {
        p = sim::step(p, run, world);
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
    let mut p = walk_off_the_crate(&world);
    assert!(p.coyote > 0.0, "leaving the ground without jumping must open the coyote window");

    p = sim::step(p, press(true), &world);
    assert!(
        (p.motion.velocity.y - (JUMP_VELOCITY - ASCENT_GRAVITY * SIM_DT)).abs() < EPS,
        "the coyote jump is the full ground jump, not the scaled air jump: {}",
        p.motion.velocity.y
    );
    assert_eq!(p.air_jumps, AIR_JUMPS, "the coyote jump must not spend the air jump");
    assert_eq!(p.coyote, 0.0, "a jump consumes the coyote grace immediately");

    // The double jump is still in hand after the coyote jump.
    p = sim::step(p, press(true), &world);
    assert_eq!(p.air_jumps, AIR_JUMPS - 1, "the air jump should fire after a coyote jump");
}

#[test]
fn the_coyote_window_expires_into_the_air_jump() {
    // Past the window the forgiveness is over: a press falls through to the air jump (scaled
    // velocity, one spent), exactly as if the player had jumped off the edge normally.
    let world = crate_world();
    let mut p = walk_off_the_crate(&world);
    let expiry = (COYOTE_S / SIM_DT).ceil() as u32 + 1;
    for _ in 0..expiry {
        p = sim::step(p, idle(false), &world);
    }
    assert!(!p.grounded, "fixture: must still be falling when the window closes");
    assert_eq!(p.coyote, 0.0, "the window must have expired");

    p = sim::step(p, press(true), &world);
    assert!(
        (p.motion.velocity.y - (JUMP_VELOCITY * AIR_JUMP_SCALE - ASCENT_GRAVITY * SIM_DT)).abs() < EPS,
        "past the window the press must spend the air jump: {}",
        p.motion.velocity.y
    );
    assert_eq!(p.air_jumps, AIR_JUMPS - 1);
}

#[test]
fn a_ground_jump_consumes_the_coyote_grace_immediately() {
    // The no-stacking rule: without consuming the grace, a grounded jump would leave COYOTE_S of
    // "still allowed to ground jump" hanging in the air, and a quick second press would pogo a
    // free full jump. The second press must spend the air jump instead.
    let world = flat_world(2.0);
    let p = sim::step(at_rest(&world), press(true), &world);
    assert!(!p.grounded, "the jump leaves the ground");
    assert_eq!(p.coyote, 0.0, "jumping must close the window the grounded state had open");

    let p = sim::step(p, press(true), &world);
    assert!(
        (p.motion.velocity.y - (JUMP_VELOCITY * AIR_JUMP_SCALE - ASCENT_GRAVITY * SIM_DT)).abs() < EPS,
        "the immediate second press must be the air jump, not a stacked ground jump: {}",
        p.motion.velocity.y
    );
    assert_eq!(p.air_jumps, AIR_JUMPS - 1);
}

#[test]
fn the_latch_honors_the_coyote_window() {
    // The latch and the step share Player::can_jump, so a press latched on a zero-step frame
    // during the window must fire - even with the air jump already spent - and an expired window
    // with nothing in hand must buffer instead.
    let in_window = Player {
        motion: Motion { position: Vec3::new(64.0, 10.0, 64.0), velocity: Vec3::ZERO },
        grounded: false,
        air_jumps: 0,
        coyote: COYOTE_S * 0.5,
        cut_armed: false,
    };
    let mut latch = JumpLatch::new();
    latch.press();
    assert!(latch.consume(in_window.can_jump()), "the window alone must let the latch fire");

    let expired = Player { coyote: 0.0, ..in_window };
    latch.press();
    assert!(!latch.consume(expired.can_jump()), "expired and spent: the press must wait for landing");
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
    let mut p = Player {
        motion: Motion {
            position: Vec3::new(64.0, 2.0 + PLAYER_HEIGHT * 0.5 + 0.2, 64.0),
            velocity: Vec3::new(0.0, -3.0, 0.0),
        },
        grounded: false,
        air_jumps: 0,
        coyote: 0.0,
        cut_armed: false,
    };
    let mut latch = JumpLatch::new();
    latch.press();

    for _ in 0..20 {
        if latch.consume(p.can_jump()) {
            assert!(p.grounded, "nothing in hand: the press may only fire once landed");
            p = sim::step(p, press(true), &world);
            assert!(p.motion.velocity.y > 0.0, "the buffered press must become the landing jump");
            return;
        }
        p = sim::step(p, idle(false), &world);
    }
    panic!("the buffered press never fired: landing took too long or the buffer dropped it");
}
