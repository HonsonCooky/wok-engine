//! The Level 2 deterministic replay harness, living in its workspace home.
//!
//! The HLD's Level 2 is game-owned: a scripted input sequence over N fixed steps, with the dumped
//! state compared for exact reproduction. The first instance stood in wok-physics's tests because no
//! game existed; taste is the game now, so the harness runs here against taste's own composition -
//! the real `World::from_store` reduction and the real `sim::step` the windowed app drives, headless
//! (no window, no GPU, no wall-clock). The wok-physics locomotion tests stay where they are, pinning
//! the engine-side composition; this pins taste's.
//!
//! The world is a small fixture built in code, not `./content`: tests must not depend on what the
//! editor last wrote to disk. Geometry mirrors the locomotion harness (a flat region, a gentle
//! slope, a long wall) so the script exercises fall, landing, a wall stop, a jump, and a slide in
//! one run.

use std::collections::HashMap;

use glam::{Quat, Vec3};
use wok_content::ChunkStore;
use wok_physics::Motion;
use wok_scene::{
    Aabb, CHUNK_GRID_DIM, CHUNK_GRID_LEN, Chunk, ChunkCoord, ChunkStreaming, HEIGHT_MAX_M, HEIGHT_MIN_M, Heightmap,
    InstanceId, Placement, Prefab, PrefabRef, PrefabState, Primitive, Shape, SurfaceTag, Transform,
};

use crate::constants::{PLAYER_HEIGHT, PLAYER_RADIUS, SNAP_DOWN_DISTANCE};
use crate::sim::{self, Player, StepInput};
use crate::world::World;

// ---- the fixture world ----

/// The wall's centre and size; its near (-x) face is at x = 14, the surface the script runs into.
const WALL_CENTER: Vec3 = Vec3::new(15.0, 4.0, 24.0);
const WALL_SIZE: Vec3 = Vec3::new(2.0, 8.0, 48.0);
const WALL_NEAR_X: f32 = WALL_CENTER.x - WALL_SIZE.x * 0.5;

const FLAT_HEIGHT_M: f32 = 2.0;

/// Cells with x index at or below this are flat; beyond it the terrain ramps up along +x by
/// `SLOPE_DELTA` raw units per cell (about 5.6 degrees, walkable).
const FLAT_MAX_CELL: u16 = 40;
const SLOPE_DELTA: u16 = 100;

/// A solid placeholder prefab: a single unit cube, hitbox and visible.
fn solid_block() -> Prefab {
    Prefab {
        states: vec![PrefabState {
            name: "default".into(),
            shapes: vec![Shape {
                primitive: Primitive::Cube,
                transform: Transform::IDENTITY,
                surface: Some(SurfaceTag::new("stone")),
                is_hitbox: true,
                is_visible: true,
            }],
            mesh: None,
        }],
        default_state: "default".into(),
    }
}

fn placement(id: u32, center: Vec3, size: Vec3) -> Placement {
    Placement {
        prefab: PrefabRef::new("block"),
        instance_id: InstanceId(id),
        transform: Transform { translation: center, rotation: Quat::IDENTITY, scale: size },
        state: None,
    }
}

fn fixture_heightmap() -> Heightmap {
    let base = Heightmap::meters_to_raw(FLAT_HEIGHT_M);
    let heights = (0..CHUNK_GRID_LEN)
        .map(|i| {
            let cx = (i % CHUNK_GRID_DIM) as u16;
            if cx <= FLAT_MAX_CELL { base } else { base + (cx - FLAT_MAX_CELL) * SLOPE_DELTA }
        })
        .collect();
    Heightmap::new(heights, vec![SurfaceTag::new("ground")], vec![0; CHUNK_GRID_LEN]).unwrap()
}

/// Build the fixture through the app's own path: authored chunk into the store, store into the
/// world, so the harness covers the slice, the AABB lift, and the terrain hand-off, not only the
/// step function.
fn fixture_world() -> World {
    let chunk = Chunk {
        coord: ChunkCoord::new(0, 0),
        placements: vec![
            // The long wall along z the script stops against and slides along.
            placement(1, WALL_CENTER, WALL_SIZE),
            // A pillar off the walked path, so collision is shown to be selective.
            placement(2, Vec3::new(30.0, 4.0, 30.0), Vec3::new(2.0, 8.0, 2.0)),
        ],
        streaming: ChunkStreaming::default(),
    };
    let mut prefabs = HashMap::new();
    prefabs.insert(PrefabRef::new("block"), solid_block());

    let mut store = ChunkStore::new();
    store.load(chunk, Some(fixture_heightmap()), &prefabs).expect("the fixture chunk should load");
    World::from_store(&store)
}

// ---- the scripted run ----

/// The script: fall from the air and land, walk +x into the wall and pin against it, jump once at
/// the wall, then slide diagonally along it. Every locomotion arc the demo shows, in one sequence.
fn scripted_inputs() -> Vec<StepInput> {
    let forward = Vec3::new(1.0, 0.0, 0.0);
    let diagonal = Vec3::new(1.0, 0.0, 1.0).normalize();
    let mut inputs = Vec::new();
    inputs.extend(std::iter::repeat_n(StepInput::default(), 90));
    inputs.extend(std::iter::repeat_n(StepInput { move_dir: forward, jump: false }, 150));
    inputs.push(StepInput { move_dir: forward, jump: true });
    inputs.extend(std::iter::repeat_n(StepInput { move_dir: forward, jump: false }, 59));
    inputs.extend(std::iter::repeat_n(StepInput { move_dir: diagonal, jump: false }, 60));
    inputs
}

fn run(world: &World, inputs: &[StepInput]) -> Vec<Player> {
    let start =
        Player { motion: Motion { position: Vec3::new(6.0, 8.0, 6.0), velocity: Vec3::ZERO }, grounded: false };
    let mut state = start;
    let mut trajectory = Vec::with_capacity(inputs.len());
    for &input in inputs {
        state = sim::step(state, input, world);
        trajectory.push(state);
    }
    trajectory
}

/// The raw bits of each component, so equality is exact rather than float `==` (which would treat
/// `0.0` and `-0.0` as equal despite differing bits).
fn bits(v: Vec3) -> [u32; 3] {
    [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()]
}

/// A player at `position` with no velocity, airborne until proven otherwise.
fn player_at(position: Vec3) -> Player {
    Player { motion: Motion { position, velocity: Vec3::ZERO }, grounded: false }
}

/// Settle a player under no input for `steps` fixed steps.
fn settle(world: &World, start: Player, steps: usize) -> Player {
    let mut state = start;
    for _ in 0..steps {
        state = sim::step(state, StepInput::default(), world);
    }
    state
}

/// The y of the capsule's lowest point for a centre at `center`: for an upright capsule the base
/// sits half the total height below the centre (half-segment plus radius), however the height
/// splits between them.
fn base_height(center: Vec3) -> f32 {
    center.y - PLAYER_HEIGHT * 0.5
}

#[test]
fn at_rest_on_flat_ground_the_base_sits_exactly_on_the_surface() {
    // The at-rest convention tie, physics side: after settling on flat terrain the capsule's lowest
    // point is the sampled surface height - no constant offset anywhere in the foot-vs-centre
    // bookkeeping across sim, world, and the rest query. Measured residue is two ulps (about 0.24
    // micrometres at 2m): one from the lift's `base + (ground - base)` add, one from reconstructing
    // the base out of the centre (centre minus height/2 versus the capsule's own endpoint-minus-
    // radius chain). The bound allows a few ulps of that roundoff; a convention bug would be
    // centimetres or more and is what this test exists to catch.
    let world = fixture_world();
    let rested = settle(&world, player_at(Vec3::new(6.0, 8.0, 6.0)), 240);
    let ground = world.terrains[0].heightmap.height_at(6.0, 6.0);

    assert!(rested.grounded, "should have settled grounded on the flat region");
    let base = base_height(rested.motion.position);
    assert!(
        (base - ground).abs() <= 4.0 * f32::EPSILON * ground.abs(),
        "base {base} should rest exactly on the surface {ground} (gap {})",
        base - ground
    );
}

#[test]
fn at_rest_on_the_slope_the_base_sits_on_the_surface_under_the_centre() {
    // The profile-aware footprint rest: each sample's lift candidate is discounted by how far the
    // capsule's spherical bottom has curved away from it, so on a planar ramp the centre candidate
    // wins and the base sits exactly on the surface under the centre - the old radius * slope
    // float is gone. The trade, documented on wok-physics's rest_on_heightmap, is a small sink of
    // the up-slope side of the bottom, bounded by r * (sqrt(1 + g^2) - 1); pinned here so the
    // at-rest convention on slopes cannot quietly drift either way.
    let world = fixture_world();
    let terrain = &world.terrains[0].heightmap;
    let (x, z) = (80.0, 100.0); // on the ramp, clear of the wall and the pillar
    let start = player_at(Vec3::new(x, terrain.height_at(x, z) + PLAYER_HEIGHT, z));
    let rested = settle(&world, start, 240);

    let p = rested.motion.position;
    let base = base_height(p);
    let ground = terrain.height_at(p.x, p.z);
    let slope = SLOPE_DELTA as f32 * (HEIGHT_MAX_M - HEIGHT_MIN_M) / u16::MAX as f32;
    let sink_bound = PLAYER_RADIUS * ((1.0 + slope * slope).sqrt() - 1.0);

    assert!(rested.grounded, "the ramp is well inside the walkable limit");
    assert!(
        (base - ground).abs() <= 1e-4,
        "base {base} should sit on the surface {ground} under the centre (gap {})",
        base - ground
    );
    // The sink the new convention trades for the zero gap stays sub-centimetre on this ramp.
    assert!(sink_bound < 0.01, "fixture drift: the ramp's sink bound {sink_bound} grew past 1cm");
}

// ---- downhill snap-down ----

#[test]
fn walking_downhill_stays_grounded_every_step_with_monotonic_descent() {
    // The glue's purpose: walking down the ramp, the surface falls away faster than one step of
    // gravity follows, so without the snap the walk flickers airborne every step. With it the
    // player reads grounded at every step and the descent is monotonic.
    let world = fixture_world();
    let terrain = &world.terrains[0].heightmap;
    let (x, z) = (100.0, 100.0); // high on the ramp, clear of the wall and the pillar
    let settled = settle(&world, player_at(Vec3::new(x, terrain.height_at(x, z) + PLAYER_HEIGHT, z)), 120);
    assert!(settled.grounded, "should start the walk settled on the ramp");

    let downhill = StepInput { move_dir: Vec3::new(-1.0, 0.0, 0.0), jump: false };
    let mut state = settled;
    let mut prev_y = state.motion.position.y;
    for i in 0..240 {
        state = sim::step(state, downhill, &world);
        assert!(state.grounded, "step {i}: flickered airborne walking downhill");
        assert!(state.motion.position.y <= prev_y + 1e-5, "step {i}: rose during a descent");
        prev_y = state.motion.position.y;
    }
    assert!(
        state.motion.position.y < settled.motion.position.y - 2.0,
        "should have descended the ramp, only dropped to {}",
        state.motion.position.y
    );
}

#[test]
fn walking_off_a_ledge_taller_than_the_glue_goes_airborne() {
    // A genuine drop must not be glued: standing on a 2m box (far beyond SNAP_DOWN_DISTANCE above
    // the terrain) and walking off, the player goes airborne before landing back on the ground.
    let mut world = fixture_world();
    let drop = 2.0;
    assert!(drop > SNAP_DOWN_DISTANCE, "fixture: the ledge must out-reach the glue");
    world.statics.push(Aabb::from_center_extents(
        Vec3::new(6.0, FLAT_HEIGHT_M + drop * 0.5, 100.0),
        Vec3::new(2.0, drop * 0.5, 2.0),
    ));

    let start = player_at(Vec3::new(6.0, FLAT_HEIGHT_M + drop + PLAYER_HEIGHT, 100.0));
    let on_box = settle(&world, start, 120);
    assert!(on_box.grounded, "should be standing on the box");
    assert!(base_height(on_box.motion.position) > FLAT_HEIGHT_M + drop - 0.1, "should rest on the box top");

    let off = StepInput { move_dir: Vec3::new(1.0, 0.0, 0.0), jump: false };
    let mut state = on_box;
    let mut went_airborne = false;
    for _ in 0..120 {
        state = sim::step(state, off, &world);
        went_airborne |= !state.grounded;
    }
    assert!(went_airborne, "stepping off the ledge should go airborne, not glue down");
    assert!(state.grounded, "should have landed on the terrain below");
    let ground = world.terrains[0].heightmap.height_at(state.motion.position.x, state.motion.position.z);
    assert!(
        (base_height(state.motion.position) - ground).abs() < 1e-2,
        "should end resting on the terrain at {ground}"
    );
}

#[test]
fn a_jump_from_a_downhill_walk_still_leaves_the_ground() {
    // The glue must not eat jumps: jumping mid-descent goes airborne and gains height even though
    // the support right below would otherwise be within snapping distance.
    let world = fixture_world();
    let terrain = &world.terrains[0].heightmap;
    let (x, z) = (100.0, 100.0);
    let settled = settle(&world, player_at(Vec3::new(x, terrain.height_at(x, z) + PLAYER_HEIGHT, z)), 120);

    let downhill = Vec3::new(-1.0, 0.0, 0.0);
    let mut state = settled;
    for _ in 0..60 {
        state = sim::step(state, StepInput { move_dir: downhill, jump: false }, &world);
    }
    assert!(state.grounded, "should still be walking the ramp when the jump comes");
    let before = state.motion.position;

    state = sim::step(state, StepInput { move_dir: downhill, jump: true }, &world);
    assert!(!state.grounded, "the jump step must leave the ground");
    for _ in 0..5 {
        state = sim::step(state, StepInput { move_dir: downhill, jump: false }, &world);
    }
    assert!(!state.grounded, "should still be rising shortly after the jump");
    assert!(
        state.motion.position.y > before.y + 0.3,
        "the jump should gain height downhill: {} -> {}",
        before.y,
        state.motion.position.y
    );
}

#[test]
fn an_identical_scripted_run_reproduces_bitwise() {
    let world = fixture_world();
    let inputs = scripted_inputs();

    let first = run(&world, &inputs);
    let second = run(&world, &inputs);

    assert_eq!(first.len(), second.len());
    for (i, (a, b)) in first.iter().zip(&second).enumerate() {
        assert_eq!(bits(a.motion.position), bits(b.motion.position), "position differs at step {i}");
        assert_eq!(bits(a.motion.velocity), bits(b.motion.velocity), "velocity differs at step {i}");
        assert_eq!(a.grounded, b.grounded, "grounded flag differs at step {i}");
    }
}

#[test]
fn the_scripted_run_actually_exercises_the_arcs() {
    // Guard against a degenerate script silently passing the bitwise test: the run really did fall,
    // land, reach the wall, leave the ground on the jump, and slide along the wall.
    let world = fixture_world();
    let inputs = scripted_inputs();
    let traj = run(&world, &inputs);

    assert!(!traj[0].grounded, "the run starts in the air");
    assert!(traj[89].grounded, "the idle phase should end landed on the flat ground");

    let pin = WALL_NEAR_X - PLAYER_RADIUS;
    let at_wall = &traj[239];
    assert!(at_wall.motion.position.x <= pin + 1e-2, "penetrated the wall: x = {}", at_wall.motion.position.x);
    assert!(at_wall.motion.position.x >= pin - 1e-1, "never reached the wall: x = {}", at_wall.motion.position.x);

    // The jump at step 240: airborne shortly after, having gained height.
    assert!(!traj[245].grounded, "the jump should leave the ground");
    assert!(
        traj[245].motion.position.y > traj[239].motion.position.y + 0.2,
        "the jump should gain height: {} -> {}",
        traj[239].motion.position.y,
        traj[245].motion.position.y
    );

    // The diagonal phase slides along the wall: z advances, x stays pinned.
    let before = &traj[299];
    let last = traj.last().unwrap();
    assert!(last.motion.position.z > before.motion.position.z + 2.0, "should have slid along the wall in z");
    assert!(last.motion.position.x <= pin + 1e-2, "still pinned by the wall: x = {}", last.motion.position.x);
}
