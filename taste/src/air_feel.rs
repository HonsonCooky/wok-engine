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
fn the_air_jump_redirects_to_the_stick_at_current_speed() {
    // Moving +x, air-jumping with the stick held +z: the R&C do-over swaps the heading to +z
    // outright while keeping the speed; no stick would keep the +x heading.
    let world = flat_world(2.0);
    let entry = 5.0;
    let p = airborne_at_speed(Vec3::new(entry, 0.0, 0.0));
    let turned = sim::step(p, StepInput { move_dir: Vec3::Z, jump: true }, &world);
    // One steer step rotates first; the jump then redirects fully, so the heading is exactly
    // the stick's and the magnitude is the steered speed.
    assert!(turned.motion.velocity.x.abs() < EPS, "the old heading is gone: {:?}", turned.motion.velocity);
    assert!(turned.motion.velocity.z > 0.9 * entry, "the speed survives the redirect: {:?}", turned.motion.velocity);

    let kept = sim::step(p, StepInput { move_dir: Vec3::ZERO, jump: true }, &world);
    assert!(
        (kept.motion.velocity.x - entry).abs() < EPS && kept.motion.velocity.z.abs() < EPS,
        "no stick keeps the current heading: {:?}",
        kept.motion.velocity
    );
}
