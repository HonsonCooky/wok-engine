//! Locomotion replay scenarios: the deterministic composition of wok-scene and wok-physics.
//!
//! Each test builds the same hand-authored world (see `common`) and drives a scripted per-step input
//! sequence through the game's eventual per-step composition, asserting the resulting trajectory. The
//! four locomotion scenarios are the ones the integration brief calls for: fall and rest, walk into a
//! wall, slide along a wall, and walk across a slope. A leading test checks the scene reduces into the
//! physics inputs as expected, and a final test runs an identical sequence twice and checks the two
//! trajectories are bitwise identical, exercising the determinism contract end to end across both
//! crates.

mod common;

use common::{
    PLAYER_RADIUS, SLOPE_DELTA, WALL_NEAR_X, assert_bitwise_eq, authored_world, foot_height,
    grounded_start, simulate,
};
use glam::Vec3;
use wok_physics::{Motion, world_aabb};
use wok_scene::{Aabb, HEIGHT_MAX_M, HEIGHT_MIN_M, InstanceId, slice_chunk};

/// The scene composes into the physics inputs the way the game expects: solids become collision
/// hitboxes, the trigger volume stays out of collision, and reducing a hitbox reproduces its face.
#[test]
fn the_authored_world_slices_and_reduces_for_collision() {
    let world = authored_world();
    let sliced = slice_chunk(&world.chunk, &world.prefabs).expect("authored chunk should slice");

    // Two solids (wall, pillar) become hitboxes; the zone becomes a trigger, not collision geometry.
    assert_eq!(sliced.hitboxes.len(), 2, "wall and pillar are solid hitboxes");
    assert_eq!(sliced.triggers.len(), 1, "the zone is a trigger volume, kept out of collision");
    assert_eq!(sliced.triggers[0].instance, InstanceId(3), "trigger carries its placement instance");
    assert_eq!(sliced.visible.len(), 2, "both solids are visible placeholders; the trigger is not");

    // Reducing the hitboxes to AABBs reproduces the wall's near face, the surface physics collides on.
    let statics: Vec<Aabb> = sliced.hitboxes.iter().map(|h| world_aabb(h.primitive, h.transform)).collect();
    let near_x = statics.iter().map(|a| a.min.x).fold(f32::INFINITY, f32::min);
    assert!((near_x - WALL_NEAR_X).abs() < 1e-5, "wall near face should reduce to {WALL_NEAR_X}, got {near_x}");
}

/// Spawn above the flat ground with no input: the body falls and comes to rest on the terrain surface,
/// reading grounded, with no horizontal drift.
#[test]
fn fall_and_rest_settles_on_the_surface_grounded() {
    let world = authored_world();
    let start = Motion { position: Vec3::new(6.0, 20.0, 6.0), velocity: Vec3::ZERO };
    let traj = simulate(&world, start, &vec![Vec3::ZERO; 240]);

    assert!(!traj[0].grounded, "should still be falling on the first step");
    let last = traj.last().unwrap();
    let ground = world.terrain.height_at(6.0, 6.0);
    assert!(last.grounded, "should come to rest grounded on the surface");
    assert!(
        (foot_height(last.motion.position) - ground).abs() < 1e-2,
        "feet should rest on the surface {ground}, got {}",
        foot_height(last.motion.position),
    );
    assert!((last.motion.position.x - 6.0).abs() < 1e-4, "no horizontal drift in x");
    assert!((last.motion.position.z - 6.0).abs() < 1e-4, "no horizontal drift in z");
    assert!(last.motion.velocity.y.abs() < 1e-3, "a resting body has no vertical velocity");
}

/// Walk straight into the wall: the player stops a radius short of its face without penetrating, and
/// the into-wall velocity is killed.
#[test]
fn walk_into_a_wall_stops_without_penetrating() {
    let world = authored_world();
    let start = grounded_start(&world.terrain, 6.0, 6.0);
    let traj = simulate(&world, start, &vec![Vec3::new(4.0, 0.0, 0.0); 300]);

    let last = traj.last().unwrap();
    let end = last.motion.position;
    let pin = WALL_NEAR_X - PLAYER_RADIUS; // 13.5: the centre cannot pass this without penetrating
    assert!(end.x <= pin + 1e-2, "penetrated the wall: x = {}", end.x);
    assert!(end.x >= pin - 1e-2, "should have reached the wall: x = {}", end.x);
    assert!((end.z - 6.0).abs() < 1e-3, "no sideways drift expected: z = {}", end.z);
    assert!(last.grounded, "should stay grounded on the flat ground at the wall");
    assert!(last.motion.velocity.x.abs() < 1e-2, "head-on velocity should be killed: vx = {}", last.motion.velocity.x);
}

/// Walk diagonally into the wall: the player ends moving parallel to it (into-wall velocity gone,
/// along-wall velocity retained) without penetrating, having slid several metres along it.
#[test]
fn slide_along_a_wall_ends_parallel_without_penetrating() {
    let world = authored_world();
    let start = grounded_start(&world.terrain, 6.0, 6.0);
    let traj = simulate(&world, start, &vec![Vec3::new(4.0, 0.0, 4.0); 240]);

    let last = traj.last().unwrap();
    let end = last.motion.position;
    let pin = WALL_NEAR_X - PLAYER_RADIUS;
    assert!(end.x <= pin + 1e-2, "penetrated the wall: x = {}", end.x);
    assert!(end.z > 6.0 + 5.0, "should have slid several metres along z: z = {}", end.z);
    assert!(last.motion.velocity.x.abs() < 1e-2, "into-wall velocity should be gone: vx = {}", last.motion.velocity.x);
    assert!(last.motion.velocity.z > 3.0, "along-wall velocity should be retained: vz = {}", last.motion.velocity.z);
    assert!(last.grounded, "should stay grounded while sliding");
}

/// Walk up the gentle slope: the player tracks the surface height under it and stays grounded at every
/// step, and ends up meaningfully higher than it started.
#[test]
fn walk_across_the_slope_tracks_the_surface_and_stays_grounded() {
    let world = authored_world();
    let (x0, z) = (50.0, 64.0);
    let start = grounded_start(&world.terrain, x0, z);
    let traj = simulate(&world, start, &vec![Vec3::new(4.0, 0.0, 0.0); 240]);

    // Slope rise per metre in +x, from the heightmap quantization (mirrors wok-scene's ramp math). The
    // footprint rest lifts the feet onto the highest of five samples, a radius up-slope of the centre.
    let slope = SLOPE_DELTA as f32 * (HEIGHT_MAX_M - HEIGHT_MIN_M) / u16::MAX as f32;
    let foot_band = PLAYER_RADIUS * slope + 5e-3;

    for (i, s) in traj.iter().enumerate() {
        let p = s.motion.position;
        let ground = world.terrain.height_at(p.x, p.z);
        let foot = foot_height(p);
        assert!(s.grounded, "step {i}: should stay grounded on a gentle slope");
        assert!(foot >= ground - 1e-3, "step {i}: feet {foot} sank below the surface {ground}");
        assert!(foot <= ground + foot_band, "step {i}: feet {foot} float above the surface {ground}");
    }

    let end = traj.last().unwrap().motion.position;
    let start_ground = world.terrain.height_at(x0, z);
    let end_ground = world.terrain.height_at(end.x, z);
    assert!(end.x > x0 + 5.0, "should have walked several metres up-slope: x = {}", end.x);
    assert!(end_ground > start_ground + 0.4, "should have climbed: {start_ground} -> {end_ground}");
}

/// The whole point of the spike: an identical scripted sequence reproduces bit for bit, through the
/// slice, the AABB reduction, and the integrate/slide/terrain composition on both crates.
#[test]
fn an_identical_scripted_run_reproduces_bitwise() {
    let world = authored_world();
    // Falls onto the flat ground, then walks diagonally into the wall: gravity, terrain rest, and a
    // box slide all exercised in one run.
    let start = Motion { position: Vec3::new(6.0, 8.0, 6.0), velocity: Vec3::ZERO };
    let inputs = vec![Vec3::new(4.0, 0.0, 4.0); 300];

    let first = simulate(&world, start, &inputs);
    let second = simulate(&world, start, &inputs);
    assert_bitwise_eq(&first, &second);

    // Guard against a degenerate run silently passing: it really did fall, ground, and meet the wall.
    assert!(first.iter().any(|s| s.grounded), "the run should become grounded at some point");
    let end = first.last().unwrap().motion.position;
    assert!(end.x <= WALL_NEAR_X - PLAYER_RADIUS + 1e-2, "the run should be stopped by the wall: x = {}", end.x);
}
