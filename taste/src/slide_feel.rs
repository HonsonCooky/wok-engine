//! The slide policies pinned: the wall stop's incidence window, the friction scrub, flat-bottom
//! support at the slide level, and the no-drift stands through the real step.
//!
//! `crate::slide` owns the policies; these tests drive `slide_player` directly for the per-contact
//! seams and `sim::step` for the composed stands, the same split `crate::air` / `crate::air_feel`
//! uses. A test-only module so the policy file stays within the size target; the step-up and the
//! tilted-face/overhang pins live in `crate::landing`.

// Exact float comparison is intended where it appears: the flat resolve's claim is that gravity
// introduces exactly zero sideways motion (the projection of a vertical vector onto the +Y plane
// is the zero vector, bit for bit), not that it stays under a tolerance.
#![allow(clippy::float_cmp)]

use glam::Vec3;
use wok_physics::{Collider, Cylinder, Motion};
use wok_scene::{Aabb, CHUNK_GRID_LEN, Heightmap, SurfaceTag};

use crate::constants::{
    AIR_JUMPS, FALL_GRAVITY, MOVE_SPEED, PLAYER_HEIGHT, PLAYER_RADIUS, SIM_DT, WALL_FRICTION,
};
use crate::sim::{self, Player, StepInput};
use crate::slide::slide_player;
use crate::world::{ChunkTerrain, World};

// ---- the policy at the slide level ----

fn upright(center: Vec3) -> Cylinder {
    Cylinder::upright(center, PLAYER_HEIGHT, PLAYER_RADIUS)
}

/// One step's gravity-only probe: the displacement and velocity a standing body brings to the
/// slide (fall gravity; the magnitude only has to be a realistic resting probe).
fn gravity_probe() -> (Vec3, Vec3) {
    let vy = -FALL_GRAVITY * SIM_DT;
    (Vec3::new(0.0, vy * SIM_DT, 0.0), Vec3::new(0.0, vy, 0.0))
}

// ---- the wall stop's incidence window ----
//
// One wall along z, near face at x = 65 (inward normal -x), and a body a nudge from it so every
// approach below genuinely contacts.

fn wall() -> [Collider; 1] {
    [Collider::from(Aabb::new(Vec3::new(65.0, 0.0, 0.0), Vec3::new(66.0, 6.0, 128.0)))]
}

fn at_the_wall() -> Cylinder {
    upright(Vec3::new(64.5, 2.75, 64.0))
}

/// One walk-speed step's motion and velocity at `degrees` off head-on (+x is straight into the
/// wall, the tangent runs +z).
fn approach(degrees: f32) -> (Vec3, Vec3) {
    let (sin, cos) = degrees.to_radians().sin_cos();
    let dir = Vec3::new(cos, 0.0, sin);
    (dir * (MOVE_SPEED * SIM_DT), dir * MOVE_SPEED)
}

#[test]
fn a_head_on_wall_contact_stops_dead() {
    // The verdict's centre: running straight at a wall ends the run. The body pins at the wall's
    // face and the whole velocity dies - nothing redirects, nothing skates. The wall is far
    // taller than STEP_HEIGHT (contact point at the body's mid-height), so the step-up never
    // fires even though the slide is told it is grounded.
    let statics = wall();
    let body = at_the_wall();
    let (d, v) = approach(0.0);
    let r = slide_player(body, d, v, &statics, true);
    assert_eq!(r.velocity, Vec3::ZERO, "a head-on hit must kill the velocity outright");
    assert_eq!(r.position.z, body.center.z, "no tangential drift out of a head-on hit");
    assert!(r.position.x <= 65.0 - PLAYER_RADIUS, "the body stays outside the wall");
    assert!(r.position.x > body.center.x, "the pre-contact advance still happens");
    assert!(!r.supported && !r.grounded, "a wall is neither support nor ground");
}

#[test]
fn a_20_degree_approach_stops_inside_the_window() {
    // Inside the (30-degree) window but off-axis: a plain plane projection would keep most of
    // the tangential motion (the skate the verdict rejected); the policy kills it. The
    // pre-contact advance still carries its tangential share (the body really did move until it
    // touched), so the stop's pins are the dead velocity and that a SECOND step from the
    // now-flush body goes nowhere.
    let statics = wall();
    let body = at_the_wall();
    let (d, v) = approach(20.0);
    let r = slide_player(body, d, v, &statics, false);
    assert_eq!(r.velocity, Vec3::ZERO, "inside the window the redirect dies");
    assert!(
        (r.position.z - body.center.z).abs() <= d.z,
        "no tangential travel beyond the pre-contact advance: z moved {}",
        r.position.z - body.center.z
    );

    let again = slide_player(upright(r.position), d, v, &statics, false);
    assert_eq!(again.velocity, Vec3::ZERO);
    assert!(
        (again.position.z - r.position.z).abs() < 1e-4,
        "flush against the wall, the window admits no tangential creep: z moved {}",
        again.position.z - r.position.z
    );
}

#[test]
fn a_45_degree_approach_slides_with_exactly_one_steps_scrub() {
    // Outside the 30-degree window the contact slides: the into-wall component dies in the plane
    // projection (the wall is vertical, so the tangential component survives in full), and the
    // exit speed is that tangential speed less exactly one step of WALL_FRICTION - the analytic
    // scrub of a glancing wall slide.
    let statics = wall();
    let body = at_the_wall();
    let (d, v) = approach(45.0);
    let r = slide_player(body, d, v, &statics, false);
    assert!(r.velocity.x.abs() < 1e-5, "the wall kills the into-wall component: {:?}", r.velocity);
    let expected_z = v.z - WALL_FRICTION * SIM_DT;
    assert!(
        (r.velocity.z - expected_z).abs() < 1e-4,
        "exit {} should be the tangential {} less one step's scrub",
        r.velocity.z,
        v.z
    );
    assert!(r.position.z > body.center.z, "the glancing contact keeps sliding along the wall");
    assert!(r.position.x <= 65.0 - PLAYER_RADIUS, "never inside the wall");
}

#[test]
fn a_brief_graze_barely_scrubs_and_free_motion_scrubs_nothing() {
    // One contacting step costs one step of WALL_FRICTION - a sliver of the entry speed - and
    // the moment contact ends the scrub ends with it: a following step clear of the wall returns
    // the velocity untouched.
    let statics = wall();
    let body = at_the_wall();
    let (d, v) = approach(45.0);
    let grazed = slide_player(body, d, v, &statics, false);
    let scrub = WALL_FRICTION * SIM_DT;
    assert!((v.z - grazed.velocity.z - scrub).abs() < 1e-4, "one contact step scrubs one step's friction");
    assert!(scrub < 0.1 * v.z, "the graze's dent must be a sliver of the entry speed");

    let away = slide_player(upright(grazed.position), Vec3::new(-0.05, 0.0, 0.1), grazed.velocity, &statics, false);
    assert_eq!(away.velocity, grazed.velocity, "no contact, no scrub");
}

#[test]
fn a_ceiling_bump_does_not_scrub_the_run() {
    // The scrub is for walls. A flat ceiling's normal is straight down - no horizontal part - so
    // rising into it while running kills the rise (the plane projection) and must leave the run
    // alone: head bumps are not wall slides.
    let ceiling = [Collider::from(Aabb::new(Vec3::new(60.0, 4.0, 60.0), Vec3::new(68.0, 5.0, 68.0)))];
    let body = upright(Vec3::new(64.0, 4.0 - PLAYER_HEIGHT * 0.5 - 0.005, 64.0));
    let v = Vec3::new(MOVE_SPEED, 3.0, 0.0);
    let r = slide_player(body, v * SIM_DT, v, &ceiling, false);
    assert_eq!(r.velocity.x, v.x, "the run must survive the head bump unscrubbed");
    assert_eq!(r.velocity.y, 0.0, "the rise dies in the ceiling");
    assert!(!r.supported, "a ceiling can never be support");
}

#[test]
fn gravity_still_falls_through_a_head_on_stop() {
    // The vertical exemption: a falling body that hits the wall head-on loses its run, not its
    // fall - the wall is vertical, so the downward velocity survives untouched (bitwise:
    // projecting a vertical vector onto a vertical wall's plane is a no-op).
    let statics = wall();
    let body = at_the_wall();
    let (mut d, mut v) = approach(0.0);
    d.y = -0.02;
    v.y = -3.0;
    let r = slide_player(body, d, v, &statics, false);
    assert_eq!(r.velocity, Vec3::new(0.0, -3.0, 0.0), "the fall must survive the wall stop");
    assert!(r.position.y < body.center.y, "the body keeps descending along the wall");
}

// ---- support: the footprint test at the slide level ----

#[test]
fn a_contact_over_the_apex_is_support_and_resolves_flat() {
    // The boulder, flat-bottomed: resting with the axis 0.2 off the apex, the bottom disc covers
    // the apex, so the contact is the apex itself - under the footprint, normal +Y. One gravity
    // probe resolves flat: supported, grounded, zero horizontal bleed, bitwise.
    let boulder = [Collider::Sphere { center: Vec3::new(64.0, 2.0, 64.0), radius: 1.1 }];
    // Base resting a hair over the apex (y = 3.1): centre at apex + half height + 2mm.
    let body = upright(Vec3::new(64.2, 3.1 + PLAYER_HEIGHT * 0.5 + 0.002, 64.0));
    let (d, v) = gravity_probe();
    let r = slide_player(body, d, v, &boulder, false);
    assert!(r.supported, "the apex under the disc is genuine support");
    assert!(r.grounded, "the apex contact grades as ground");
    let bleed = Vec3::new(r.velocity.x, 0.0, r.velocity.z).length();
    assert!(bleed == 0.0, "a supported contact must bleed no horizontal velocity: {bleed}");
    assert_eq!(r.position.x, body.center.x, "gravity must not walk the body sideways");
    assert_eq!(r.position.z, body.center.z);
}

#[test]
fn a_steep_flank_contact_is_not_support_and_sheds() {
    // Far down the flank the bearing normal is past the 60-degree walkable limit: the contact
    // grades as a wall (engine resolve, no flat rescue), so the body sheds. The contact point
    // may sit under the rim - support needs the normal grade AND the footprint, and here the
    // grade fails.
    let center = Vec3::new(64.0, 2.0, 64.0);
    let r_s = 1.1_f32;
    let boulder = [Collider::Sphere { center, radius: r_s }];
    // Touch the flank at 70 degrees off vertical (normal.y = cos 70 ~ 0.34, well past the
    // limit): place the body so its bottom-rim region rests on that surface point.
    let polar = 70.0_f32.to_radians();
    let n = Vec3::new(polar.sin(), polar.cos(), 0.0);
    let p = center + n * r_s;
    let base = p.y + 0.003; // a hair above the contact, gravity closes it
    let body = upright(Vec3::new(p.x + PLAYER_RADIUS - 0.05, base + PLAYER_HEIGHT * 0.5, p.z));
    let (d, v) = gravity_probe();
    let r = slide_player(body, d, v, &boulder, false);
    assert!(!r.supported, "a past-the-limit flank must not read as support");
    assert!(!r.grounded, "nor as ground");
    assert!(r.velocity.y < 0.0, "the fall survives the contact: the body is shedding, not standing");
}

#[test]
fn walking_on_a_supported_top_still_makes_progress() {
    // The stall guard: the flat resolve keeps the horizontal intent, and the second (true plane)
    // projection lets it run along the surface instead of re-colliding every iteration and being
    // dropped at the cap.
    let boulder = [Collider::Sphere { center: Vec3::new(64.0, 2.0, 64.0), radius: 1.1 }];
    let body = upright(Vec3::new(64.1, 3.1 + PLAYER_HEIGHT * 0.5 + 0.002, 64.0));
    let step = MOVE_SPEED * SIM_DT;
    let d = Vec3::new(step, -0.005, 0.0);
    let v = Vec3::new(MOVE_SPEED, -0.3, 0.0);
    let r = slide_player(body, d, v, &boulder, false);
    let moved = r.position.x - body.center.x;
    assert!(moved > step * 0.5, "a supported walk must keep moving: moved {moved} of {step}");
}

// ---- the policy through the real step ----

fn flat_world(height_m: f32) -> World {
    let raw = Heightmap::meters_to_raw(height_m);
    let heightmap =
        Heightmap::new(vec![raw; CHUNK_GRID_LEN], vec![SurfaceTag::new("g")], vec![0; CHUNK_GRID_LEN]).unwrap();
    World { statics: vec![], terrains: vec![ChunkTerrain { origin: Vec3::ZERO, heightmap }], ..World::default() }
}

fn standing(x: f32, z: f32, base_y: f32) -> Player {
    Player {
        motion: Motion { position: Vec3::new(x, base_y + PLAYER_HEIGHT * 0.5 + 0.02, z), velocity: Vec3::ZERO },
        grounded: false,
        air_jumps: AIR_JUMPS,
        coyote: 0.0,
    }
}

/// Settle, then run `steps` idle steps asserting grounded throughout; returns the horizontal
/// drift from the settled position.
fn idle_drift(world: &World, start: Player, steps: usize) -> Vec3 {
    let mut p = start;
    for _ in 0..30 {
        p = sim::step(p, StepInput::default(), world);
    }
    assert!(p.grounded, "fixture: must settle grounded before measuring drift");
    let settled = p.motion.position;
    for i in 0..steps {
        p = sim::step(p, StepInput::default(), world);
        assert!(p.grounded, "step {i}: lost the ground while standing still");
    }
    let d = p.motion.position - settled;
    Vec3::new(d.x, 0.0, d.z)
}

#[test]
fn standing_near_and_past_a_crate_edge_does_not_drift() {
    // The crate-edge cases, overhang included: with the axis inside the footprint the box
    // contact is flat (this never drifted); with the axis PAST the edge (65.3, a 0.3m overhang
    // against the 0.45 radius) the rim bears on the edge - the flat bottom's new stand - and
    // must hold grounded with zero drift too.
    let mut world = flat_world(2.0);
    world.statics.push(Aabb::new(Vec3::new(63.0, 2.0, 63.0), Vec3::new(65.0, 4.0, 65.0)).into());
    for x in [64.9_f32, 64.99, 65.0, 65.3] {
        let drift = idle_drift(&world, standing(x, 64.0, 4.0), 120);
        assert!(drift.length() < 1e-4, "x={x}: drifted {drift:?} standing near the crate edge");
    }
}

#[test]
fn standing_supported_on_a_boulder_apex_does_not_drift() {
    // The edge-drift fix's demonstrator, on the new shape: standing off the apex with the disc
    // still over it, the contact resolves flat and the player stands exactly still.
    let mut world = flat_world(2.0);
    world.statics.push(Collider::Sphere { center: Vec3::new(64.0, 2.0, 64.0), radius: 1.1 });
    let drift = idle_drift(&world, standing(64.2, 64.0, 3.1), 120);
    assert!(drift.length() < 1e-4, "drifted {drift:?} standing supported on the apex");
}

#[test]
fn standing_supported_on_a_gently_tilted_platform_does_not_drift() {
    // A cube pitched 10 degrees (face normal.y ~ 0.985, well inside walkable): supported via the
    // contact point under the disc, resolved flat, zero drift. The capsule needed a tolerance
    // window that gave out at ~15 degrees; the footprint test has no such cliff (the
    // 30/45/59-degree pins live in crate::landing).
    let mut world = flat_world(2.0);
    world.statics.push(Collider::Obb {
        center: Vec3::new(64.0, 3.0, 64.0),
        half_extents: Vec3::ONE,
        rotation: glam::Quat::from_rotation_x(10.0_f32.to_radians()),
    });
    let drift = idle_drift(&world, standing(64.0, 64.0, 4.1), 120);
    assert!(drift.length() < 1e-4, "drifted {drift:?} standing supported on the tilted face");
}

#[test]
fn walking_deliberately_off_the_edge_still_departs() {
    // The flat resolve must never glue the player to an edge forever: holding input over it, the
    // rim eventually leaves the crate top (the axis must clear the edge by the full radius now -
    // the overhang stand is real, then ends), support ends, and the body lands on the terrain
    // below.
    let mut world = flat_world(2.0);
    world.statics.push(Aabb::new(Vec3::new(63.0, 2.0, 63.0), Vec3::new(65.0, 4.0, 65.0)).into());
    let mut p = standing(64.5, 64.0, 4.0);
    for _ in 0..30 {
        p = sim::step(p, StepInput::default(), &world);
    }
    assert!(p.grounded, "fixture: should stand on the crate top");

    let off = StepInput { move_dir: Vec3::X, jump: false };
    let mut went_airborne = false;
    for _ in 0..180 {
        p = sim::step(p, off, &world);
        went_airborne |= !p.grounded;
    }
    assert!(went_airborne, "walking off the edge must still depart");
    assert!(p.grounded, "and land on the terrain below");
    let base = p.motion.position.y - PLAYER_HEIGHT * 0.5;
    assert!((base - 2.0).abs() < 1e-2, "should end on the terrain at 2m, base = {base}");
}
