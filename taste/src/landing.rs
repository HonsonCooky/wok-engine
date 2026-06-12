//! The support and step-up pins, through the real step: what the flat-bottomed cylinder stands
//! on, what it sheds, and what it climbs.
//!
//! The capsule era had a support PROBE here (`supported_below`, a per-collider bearing-surface
//! lookup under the axis with a vertical tolerance) because a rounded bottom's contact says
//! little about whether the body is over anything. The flat bottom retires it: support is now
//! read off the contact itself in `crate::slide` (a walkable normal whose point lies under the
//! disc footprint), so this module keeps only the behavioral pins, driven through `sim::step`
//! the way play exercises them. A test-only module like `crate::replay` and `crate::air_feel`.
//!
//! The pins, from the player-collider brief:
//! - tilted crate faces stand to the walkable limit (30/45/59 degrees) and shed past it (61);
//! - the body stands with its axis past a ledge while the rim is supported (the overhang), and a
//!   fall past the rim's reach still descends to the ground (the halt bug stays dead);
//! - corner landings stand instead of rolling off;
//! - a 0.2m lip is climbed mid-walk, a 0.5m face is a wall, and the jump is unaffected;
//! - the all-angles jump-off scan: every arc off every crate settles on a legal stand.

use glam::Vec3;
use wok_physics::{Collider, Motion};
use wok_scene::{Aabb, CHUNK_GRID_LEN, Heightmap, SurfaceTag};

use crate::constants::{AIR_JUMPS, JUMP_APEX_HEIGHT, PLAYER_HEIGHT, PLAYER_RADIUS, STEP_HEIGHT};
use crate::sim::{self, Player, StepInput};
use crate::world::{ChunkTerrain, World};

fn flat_world(height_m: f32) -> World {
    let raw = Heightmap::meters_to_raw(height_m);
    let heightmap =
        Heightmap::new(vec![raw; CHUNK_GRID_LEN], vec![SurfaceTag::new("g")], vec![0; CHUNK_GRID_LEN]).unwrap();
    World { statics: vec![], terrains: vec![ChunkTerrain { origin: Vec3::ZERO, heightmap }], ..World::default() }
}

/// Flat terrain at 2m with a 2m crate on it: top face at y = 4, +x face at x = 65.
fn crate_world() -> World {
    let mut world = flat_world(2.0);
    world.statics.push(Aabb::new(Vec3::new(63.0, 2.0, 63.0), Vec3::new(65.0, 4.0, 65.0)).into());
    world
}

fn airborne_at(position: Vec3) -> Player {
    Player {
        motion: Motion { position, velocity: Vec3::ZERO },
        grounded: false,
        air_jumps: AIR_JUMPS,
        coyote: 0.0,
    }
}

fn base_y(p: &Player) -> f32 {
    p.motion.position.y - PLAYER_HEIGHT * 0.5
}

/// Drive idle steps until grounded (panicking past `limit`), then return the player.
fn settle(mut p: Player, world: &World, limit: usize) -> Player {
    for _ in 0..limit {
        p = sim::step(p, StepInput::default(), world);
        if p.grounded {
            return p;
        }
    }
    panic!("never settled grounded: {p:?}");
}

// ---- tilted faces ----

/// A unit cube pitched `degrees` about x at `center`, plus where its top face's centre sits.
fn tilted_cube(center: Vec3, degrees: f32) -> (Collider, Vec3) {
    let rotation = glam::Quat::from_rotation_x(degrees.to_radians());
    let n = rotation * Vec3::Y;
    (Collider::Obb { center, half_extents: Vec3::ONE, rotation }, center + n)
}

#[test]
fn tilted_faces_inside_the_walkable_limit_are_standable_with_zero_drift() {
    // The brief's new stands: 30, 45, and 59 degrees (face normal.y = cos of each, all at or
    // above WALKABLE_NORMAL_Y). The body dropped over the face settles grounded and then stands
    // exactly still for two seconds - the flat resolve holds it, where the capsule's curvature
    // shed everything past ~15 degrees.
    for degrees in [30.0_f32, 45.0, 59.0] {
        let mut world = flat_world(2.0);
        let (cube, face_center) = tilted_cube(Vec3::new(64.0, 4.0, 64.0), degrees);
        world.statics.push(cube);

        let p = settle(airborne_at(face_center + Vec3::new(0.0, 2.0, 0.0)), &world, 240);
        assert!(
            base_y(&p) > 3.0,
            "{degrees} deg: should be standing on the face, not the terrain (base {})",
            base_y(&p)
        );
        let stood = p.motion.position;
        let mut q = p;
        for i in 0..120 {
            q = sim::step(q, StepInput::default(), &world);
            assert!(q.grounded, "{degrees} deg, step {i}: lost the stand");
        }
        let drift = (q.motion.position - stood).length();
        assert!(drift < 1e-4, "{degrees} deg: drifted {drift} while standing on the tilted face");
    }
}

#[test]
fn a_61_degree_face_sheds_to_the_terrain() {
    // One degree past the limit the contact grades as a wall: no support, no grounding on the
    // face - the body slides off and lands on the terrain. The shed must never report grounded
    // anywhere above the ground.
    let mut world = flat_world(2.0);
    let (cube, face_center) = tilted_cube(Vec3::new(64.0, 4.0, 64.0), 61.0);
    world.statics.push(cube);

    let mut p = airborne_at(face_center + Vec3::new(0.0, 2.0, 0.0));
    for i in 0..600 {
        p = sim::step(p, StepInput::default(), &world);
        if p.grounded {
            assert!(
                (base_y(&p) - 2.0).abs() < 0.05,
                "step {i}: grounded at base {} - a 61-degree face must not stand",
                base_y(&p)
            );
            return;
        }
    }
    panic!("never landed anywhere: {p:?}");
}

// ---- the overhang, the corner, and the rim's edge ----

#[test]
fn standing_with_the_axis_past_the_ledge_but_the_rim_supported_is_grounded() {
    // The overhang, the flat bottom's signature stand: a fall beside the crate with the axis
    // 0.2m past the +x face (rim reaches 0.25m back over the top) lands ON the crate edge and
    // stays - under the capsule this exact spot was the halt bug's hover, killed by denying
    // support; the cylinder makes it a real stand.
    let world = crate_world();
    let p = settle(airborne_at(Vec3::new(65.2, 5.5, 64.0)), &world, 240);
    assert!((base_y(&p) - 4.0).abs() < 0.02, "should stand at crate-top height, base {}", base_y(&p));

    let stood = p.motion.position;
    let mut q = p;
    for i in 0..120 {
        q = sim::step(q, StepInput::default(), &world);
        assert!(q.grounded, "step {i}: the overhang stand gave out");
    }
    assert!((q.motion.position - stood).length() < 1e-4, "the overhang stand must not creep");
}

#[test]
fn a_corner_landing_stands_instead_of_rolling_off() {
    // Landing on the crate's top corner (both axes past the faces, the rim's quarter over the
    // top): the capsule rolled off; the disc bears and stands.
    let world = crate_world();
    let p = settle(airborne_at(Vec3::new(65.15, 5.5, 65.15)), &world, 240);
    assert!((base_y(&p) - 4.0).abs() < 0.02, "should stand on the corner, base {}", base_y(&p));

    let stood = p.motion.position;
    let mut q = p;
    for i in 0..120 {
        q = sim::step(q, StepInput::default(), &world);
        assert!(q.grounded, "step {i}: the corner stand gave out");
    }
    assert!((q.motion.position - stood).length() < 1e-4, "the corner landing must not roll off");
}

#[test]
fn a_fall_past_the_rims_reach_descends_to_the_terrain() {
    // Just past where the rim can bear (axis 0.5m out, the disc's near edge 0.05m past the
    // face): no support exists, and the body must reach the ground rather than hang - the
    // halt-bug guard, relocated to where the flat bottom's support genuinely ends.
    let world = crate_world();
    let mut p = airborne_at(Vec3::new(65.5, 5.5, 64.0));
    let mut prev_y = p.motion.position.y;
    for i in 0..240 {
        p = sim::step(p, StepInput::default(), &world);
        assert!(p.motion.position.y <= prev_y + 1e-5, "step {i}: rose during the descent");
        prev_y = p.motion.position.y;
        if p.grounded {
            break;
        }
    }
    assert!(p.grounded, "should have landed: {p:?}");
    assert!((base_y(&p) - 2.0).abs() < 0.02, "should end on the terrain, base {}", base_y(&p));
}

// ---- the step-up ----

/// Flat terrain at 2m with a lip of height `lip` spanning x in [66, 70]: the wall the walk meets.
fn lip_world(lip: f32) -> World {
    let mut world = flat_world(2.0);
    world.statics.push(Aabb::new(Vec3::new(66.0, 2.0, 56.0), Vec3::new(70.0, 2.0 + lip, 72.0)).into());
    world
}

fn walk_east(mut p: Player, world: &World, steps: usize) -> Player {
    let run = StepInput { move_dir: Vec3::X, jump: false };
    for _ in 0..steps {
        p = sim::step(p, run, world);
    }
    p
}

#[test]
fn a_02m_lip_is_climbed_mid_walk() {
    // The policy's purpose: a 0.2m lip (under STEP_HEIGHT) blocks the flat bottom square; the
    // lift-move-drop carries the walk up onto it without a jump and without going airborne. The
    // walk stops mid-lip (40 steps reaches ~68.5 of the lip's [66, 70] span) so the pin reads the
    // stand on top, not the eventual walk off its far edge.
    let world = lip_world(0.2);
    let p = settle(airborne_at(Vec3::new(64.0, 3.0, 64.0)), &world, 240);
    let mut q = p;
    let run = StepInput { move_dir: Vec3::X, jump: false };
    for i in 0..40 {
        q = sim::step(q, run, &world);
        assert!(q.grounded, "step {i}: the climb must never read airborne");
    }
    assert!((base_y(&q) - 2.2).abs() < 0.02, "should be walking on the lip top, base {}", base_y(&q));
    assert!(q.motion.position.x > 66.5, "should have kept walking past the lip's face, x {}", q.motion.position.x);
}

#[test]
fn a_05m_face_is_a_wall() {
    // Above STEP_HEIGHT the contact point sits higher than the policy admits: the face stops the
    // run per the wall stop, and the body stays on the ground in front of it.
    let world = lip_world(0.5);
    let p = settle(airborne_at(Vec3::new(64.0, 3.0, 64.0)), &world, 240);
    let q = walk_east(p, &world, 120);
    assert!((base_y(&q) - 2.0).abs() < 0.02, "must stay at ground level, base {}", base_y(&q));
    assert!(
        q.motion.position.x <= 66.0 - PLAYER_RADIUS + 1e-2,
        "must be stopped at the face, x {}",
        q.motion.position.x
    );
    let speed = Vec3::new(q.motion.velocity.x, 0.0, q.motion.velocity.z).length();
    assert!(speed < 1e-3, "the head-on wall stop applies: speed {speed}");
}

#[test]
fn the_jump_is_unaffected_by_the_step_up() {
    // Jump steps never climb (the gate is grounded-and-not-jumped): jumping while running at the
    // 0.5m face flies the full authored arc - apex height intact - rather than being converted
    // into a step or clipped by a phantom lift.
    let world = lip_world(0.5);
    let p = settle(airborne_at(Vec3::new(65.0, 3.0, 64.0)), &world, 240);
    let start_y = p.motion.position.y;
    let mut q = sim::step(p, StepInput { move_dir: Vec3::X, jump: true }, &world);
    assert!(!q.grounded, "the jump step must leave the ground");
    let mut apex = q.motion.position.y;
    for _ in 0..240 {
        q = sim::step(q, StepInput { move_dir: Vec3::X, jump: false }, &world);
        apex = apex.max(q.motion.position.y);
        if q.grounded {
            break;
        }
    }
    assert!(
        (apex - start_y - JUMP_APEX_HEIGHT).abs() < 0.05,
        "the arc must fly the authored apex: climbed {} of {JUMP_APEX_HEIGHT}",
        apex - start_y
    );
    assert!(q.grounded, "and land again");
    assert!((base_y(&q) - 2.5).abs() < 0.05, "a 1.9m apex clears the 0.5m lip: base {}", base_y(&q));
}

#[test]
fn the_step_height_boundary_separates_climb_from_wall() {
    // The constant is the contract: a lip just under STEP_HEIGHT climbs, just over stops. Driven
    // a margin (2cm) each side of the boundary so skin and contact-point rounding never decide;
    // 40 steps lands the climbing case mid-lip, before its far edge.
    let climbable = lip_world(STEP_HEIGHT - 0.02);
    let p = settle(airborne_at(Vec3::new(64.0, 3.0, 64.0)), &climbable, 240);
    let q = walk_east(p, &climbable, 40);
    assert!((base_y(&q) - (2.0 + STEP_HEIGHT - 0.02)).abs() < 0.02, "under the limit climbs: base {}", base_y(&q));

    let wall = lip_world(STEP_HEIGHT + 0.02);
    let p = settle(airborne_at(Vec3::new(64.0, 3.0, 64.0)), &wall, 240);
    let q = walk_east(p, &wall, 40);
    assert!((base_y(&q) - 2.0).abs() < 0.02, "over the limit walls: base {}", base_y(&q));
}

// ---- the all-angles jump-off scan ----

#[test]
fn a_jump_off_each_crate_settles_on_a_legal_stand_from_every_angle() {
    // The halt-bug scan, kept and re-scoped for the flat bottom: jump from each crate top in
    // eight directions, releasing the held direction at scan points from "on the jump" to "held
    // throughout". A grounded frame is legal on the terrain or on the crate's top AT its height -
    // the top's footprint now extends a rim's reach past the faces (the overhang stand) - and
    // every run must settle on one within five seconds. What this guards: no grounded frame in
    // mid-air, and no lasting hover anywhere.
    let terrain_h = 2.0;
    for &size in &[1.0_f32, 1.5, 2.0] {
        let half = size * 0.5;
        let raw = Heightmap::meters_to_raw(terrain_h);
        let heightmap =
            Heightmap::new(vec![raw; CHUNK_GRID_LEN], vec![SurfaceTag::new("g")], vec![0; CHUNK_GRID_LEN]).unwrap();
        let world = World {
            statics: vec![
                Aabb::new(
                    Vec3::new(64.0 - half, terrain_h, 64.0 - half),
                    Vec3::new(64.0 + half, terrain_h + size, 64.0 + half),
                )
                .into(),
            ],
            terrains: vec![ChunkTerrain { origin: Vec3::ZERO, heightmap }],
            ..World::default()
        };
        let top = terrain_h + size;
        let reach = half + PLAYER_RADIUS + 0.02;
        let on_terrain = |p: &Player| (base_y(p) - terrain_h).abs() <= 0.05;
        let on_top = |p: &Player| {
            (base_y(p) - top).abs() <= 0.05
                && (p.motion.position.x - 64.0).abs() <= reach
                && (p.motion.position.z - 64.0).abs() <= reach
        };

        for k in 0..8 {
            let angle = std::f32::consts::TAU * (k as f32 / 8.0);
            let dir = Vec3::new(angle.cos(), 0.0, angle.sin());
            for hold_after in [0usize, 1, 2, 3, 4, 6, 8, 12, 600] {
                let label = format!("size {size} angle {k} hold {hold_after}");
                let mut p = airborne_at(Vec3::new(64.0, top + PLAYER_HEIGHT * 0.5 + 0.05, 64.0));
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
                        settled = true;
                        break;
                    }
                }
                assert!(settled, "{label}: never settled on a legal stand - a halt remains at {:?}", p.motion.position);
            }
        }
    }
}
