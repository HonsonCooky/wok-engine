//! The air feel pins, through the real step: redirection under gravity and the double jump.
//!
//! `crate::air` tests the steering math in isolation; these tests drive `sim::step` itself, so
//! they cover the composition - steering, the jump impulses, integration, and the grounding reset
//! - the way play exercises it. A test-only module like `crate::replay`, kept out of `sim` so the
//! step function's file stays within the size target.

use glam::Vec3;
use wok_physics::Motion;
use wok_scene::{CHUNK_GRID_LEN, Heightmap, SurfaceTag};

use crate::constants::{
    AIR_JUMP_SCALE, AIR_JUMPS, AIR_TURN_RATE, ASCENT_GRAVITY, COYOTE_S, JUMP_VELOCITY, MOVE_SPEED, PLAYER_HEIGHT,
    SIM_DT,
};
use crate::sim::{self, Player, StepInput};
use crate::world::{ChunkTerrain, World};

const EPS: f32 = 1e-5;

fn flat_world(height_m: f32) -> World {
    let raw = Heightmap::meters_to_raw(height_m);
    let heightmap =
        Heightmap::new(vec![raw; CHUNK_GRID_LEN], vec![SurfaceTag::new("g")], vec![0; CHUNK_GRID_LEN]).unwrap();
    World { statics: vec![], terrains: vec![ChunkTerrain { origin: Vec3::ZERO, heightmap }], ..World::default() }
}

/// A player standing at rest mid-chunk: capsule base exactly on the surface, grounded (carrying
/// the coyote grace a grounded step would).
fn at_rest(world: &World) -> Player {
    let ground = world.terrains[0].heightmap.height_at(64.0, 64.0);
    Player {
        motion: Motion { position: Vec3::new(64.0, ground + PLAYER_HEIGHT * 0.5, 64.0), velocity: Vec3::ZERO },
        grounded: true,
        air_jumps: AIR_JUMPS,
        coyote: COYOTE_S,
    }
}

/// Airborne high over the terrain, moving at `velocity`, the air jump unspent and the coyote
/// grace long expired.
fn airborne_at_speed(velocity: Vec3) -> Player {
    Player {
        motion: Motion { position: Vec3::new(64.0, 30.0, 64.0), velocity },
        grounded: false,
        air_jumps: AIR_JUMPS,
        coyote: 0.0,
    }
}

fn horizontal_speed(p: &Player) -> f32 {
    Vec3::new(p.motion.velocity.x, 0.0, p.motion.velocity.z).length()
}

#[test]
fn an_airborne_reversal_redirects_without_passing_through_a_stop() {
    // The brief's pin, through the full step (gravity and all): holding the stick against a
    // full-speed jump turns the heading around while the horizontal speed stays above 80% of
    // entry at every step of the turn.
    let world = flat_world(2.0);
    let entry = MOVE_SPEED;
    let mut p = airborne_at_speed(Vec3::new(entry, 0.0, 0.0));
    let back = StepInput { move_dir: Vec3::NEG_X, jump: false };
    let steps = (std::f32::consts::PI / AIR_TURN_RATE / SIM_DT).ceil() as usize;
    for i in 0..steps {
        p = sim::step(p, back, &world);
        assert!(
            horizontal_speed(&p) >= 0.8 * entry,
            "step {i}: mid-turn speed {} collapsed below 80% of entry",
            horizontal_speed(&p)
        );
    }
    assert!(p.motion.velocity.x < -0.9 * entry, "should end heading -x: {:?}", p.motion.velocity);
}

#[test]
fn a_100ms_tap_mid_jump_drifts_about_half_a_metre_not_metres() {
    // The AIR_ACCEL decoupling's felt promise, through the real step: jump straight up, tap the
    // stick for 100ms (six fixed steps), release, ride the arc down. The drift at landing must
    // stay sub-metre - at the chained AIR_CONTROL * GROUND_ACCEL rate (silently 49.5 m/s^2 after
    // the ground crispness retune) the same tap drifted ~2.5m, which is what made minute air
    // corrections impossible - while still being a real correction, not a dead stick.
    let world = flat_world(2.0);
    let start = at_rest(&world);
    let start_x = start.motion.position.x;
    let mut p = sim::step(start, StepInput { move_dir: Vec3::ZERO, jump: true }, &world);
    let tap_steps = (0.1 / SIM_DT).round() as usize;
    for _ in 0..tap_steps {
        p = sim::step(p, StepInput { move_dir: Vec3::X, jump: false }, &world);
    }
    for _ in 0..600 {
        if p.grounded {
            break;
        }
        p = sim::step(p, StepInput::default(), &world);
    }
    assert!(p.grounded, "the jump must land within ten seconds");
    let drift = (p.motion.position.x - start_x).abs();
    assert!(drift < 1.0, "a 100ms tap drifted {drift}m by landing: minute air corrections are gone again");
    assert!(drift > 0.2, "a 100ms tap drifted only {drift}m: the air control is nearly dead");
}

#[test]
fn the_double_jump_fires_airborne_exactly_once_and_resets_on_landing() {
    let world = flat_world(2.0);
    let jump = StepInput { move_dir: Vec3::ZERO, jump: true };

    // Ground jump, then the air jump: vertical velocity is set to the scaled launch speed. The
    // ground jump consumed the coyote grace, so the second press cannot fire a second free ground
    // jump and must spend the air jump instead.
    let mut p = sim::step(at_rest(&world), jump, &world);
    assert!(!p.grounded && p.air_jumps == AIR_JUMPS, "the ground jump spends no air jump");
    p = sim::step(p, jump, &world);
    assert!(
        (p.motion.velocity.y - (JUMP_VELOCITY * AIR_JUMP_SCALE - ASCENT_GRAVITY * SIM_DT)).abs() < EPS,
        "the air jump sets the scaled launch velocity (one step of gravity follows): {}",
        p.motion.velocity.y
    );
    assert_eq!(p.air_jumps, AIR_JUMPS - 1, "the air jump is spent");

    // A third press while airborne does nothing: the vertical velocity just keeps integrating.
    let vy_before = p.motion.velocity.y;
    p = sim::step(p, jump, &world);
    assert!(
        (p.motion.velocity.y - (vy_before - ASCENT_GRAVITY * SIM_DT)).abs() < EPS,
        "a spent air jump must not fire again: {}",
        p.motion.velocity.y
    );

    // Ride the arc down; landing restores the air jump.
    for _ in 0..600 {
        if p.grounded {
            break;
        }
        p = sim::step(p, StepInput::default(), &world);
    }
    assert!(p.grounded, "should land within ten seconds");
    assert_eq!(p.air_jumps, AIR_JUMPS, "grounding restores the air jumps");
}

#[test]
fn the_air_jump_sets_the_vertical_and_leaves_the_horizontal_bitwise_untouched() {
    // The redirect is gone: the air jump's impulse is vertical only. Whatever the stick, the jump
    // step's horizontal velocity is bitwise the matching no-jump step's - the press changes
    // nothing but the vertical - so direction changes are the steering's job alone (AIR_TURN_RATE
    // / AIR_ACCEL) and the air jump cannot read as a violent re-aim.
    let world = flat_world(2.0);
    let p = airborne_at_speed(Vec3::new(5.0, 0.0, 0.0));
    for stick in [Vec3::ZERO, Vec3::Z, Vec3::NEG_X] {
        let jumped = sim::step(p, StepInput { move_dir: stick, jump: true }, &world);
        let steered = sim::step(p, StepInput { move_dir: stick, jump: false }, &world);
        assert_eq!(
            jumped.motion.velocity.x.to_bits(),
            steered.motion.velocity.x.to_bits(),
            "stick {stick:?}: the jump must not touch x: {} vs {}",
            jumped.motion.velocity.x,
            steered.motion.velocity.x
        );
        assert_eq!(
            jumped.motion.velocity.z.to_bits(),
            steered.motion.velocity.z.to_bits(),
            "stick {stick:?}: the jump must not touch z: {} vs {}",
            jumped.motion.velocity.z,
            steered.motion.velocity.z
        );
        assert!(
            (jumped.motion.velocity.y - (JUMP_VELOCITY * AIR_JUMP_SCALE - ASCENT_GRAVITY * SIM_DT)).abs() < EPS,
            "stick {stick:?}: the air jump sets the scaled launch vertical (one step of gravity follows): {}",
            jumped.motion.velocity.y
        );
    }
}
