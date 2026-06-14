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
//! editor last wrote to disk. It carries a flat region, a cube wall, and a round pillar, so the
//! script exercises the simple model's whole surface in one run: fall, land on the ground (terrain
//! collision), walk into the wall and stop against it (prefab collision), jump, double-jump, and
//! slide along the wall.

use std::collections::HashMap;

use glam::{Quat, Vec3};
use wok_content::ChunkStore;
use wok_physics::{Collider, Motion};
use wok_scene::{
    CHUNK_GRID_DIM, CHUNK_GRID_LEN, Chunk, ChunkCoord, ChunkStreaming, Heightmap, InstanceId, Placement, Prefab,
    PrefabRef, PrefabState, Primitive, Shape, SurfaceTag, Transform,
};

use crate::constants::{PLAYER_HEIGHT, PLAYER_RADIUS};
use crate::sim::{self, Player, StepInput};
use crate::tuning::Tuning;
use crate::world::World;

// ---- the fixture world ----

/// The wall's centre and size; its near (-x) face is at x = 14, the surface the script runs into.
const WALL_CENTER: Vec3 = Vec3::new(15.0, 4.0, 24.0);
const WALL_SIZE: Vec3 = Vec3::new(2.0, 8.0, 48.0);
const WALL_NEAR_X: f32 = WALL_CENTER.x - WALL_SIZE.x * 0.5;

const FLAT_HEIGHT_M: f32 = 2.0;

/// Cells with x index at or below this are flat; beyond it the terrain ramps up along +x by
/// `SLOPE_DELTA` raw units per cell (about 5.6 degrees).
const FLAT_MAX_CELL: u16 = 40;
const SLOPE_DELTA: u16 = 100;

/// A solid placeholder prefab: a single unit primitive, hitbox and visible. The wall stays a cube;
/// the pillar is a cylinder, so the fixture carries one round hitbox through the store and the
/// world reduction classifies it round (the path the demo's content takes).
fn solid(primitive: Primitive) -> Prefab {
    Prefab {
        states: vec![PrefabState {
            name: "default".into(),
            shapes: vec![Shape {
                primitive,
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

fn placement(prefab: &str, id: u32, center: Vec3, size: Vec3) -> Placement {
    Placement {
        prefab: PrefabRef::new(prefab),
        instance_id: InstanceId(id),
        name: None,
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
            placement("block", 1, WALL_CENTER, WALL_SIZE),
            // A round pillar off the walked path: collision is selective, and the reduction is shown
            // to classify a real cylinder hitbox round.
            placement("pillar", 2, Vec3::new(30.0, 4.0, 30.0), Vec3::new(2.0, 8.0, 2.0)),
        ],
        streaming: ChunkStreaming::default(),
    };
    let mut prefabs = HashMap::new();
    prefabs.insert(PrefabRef::new("block"), solid(Primitive::Cube));
    prefabs.insert(PrefabRef::new("pillar"), solid(Primitive::Cylinder));

    let mut store = ChunkStore::new();
    store.load(chunk, Some(fixture_heightmap()), &prefabs).expect("the fixture chunk should load");
    World::from_store(&store)
}

// ---- the scripted run ----

/// The script: fall from the air and land on the flat ground, walk +x head-on into the wall and
/// stop pinned against it (the slide projects out the into-wall motion), jump and then double-jump
/// against the wall (both holding +x, both flying the same arc), then slide along the wall at a
/// glancing angle so the into-wall part is blocked while the along-wall part carries the body in z.
/// Every arc the simple model has, in one sequence.
fn scripted_inputs() -> Vec<StepInput> {
    let forward = Vec3::new(1.0, 0.0, 0.0);
    let glancing = Vec3::new(1.0, 0.0, 2.0).normalize();
    let mut inputs = Vec::new();
    inputs.extend(std::iter::repeat_n(StepInput::default(), 90));
    inputs.extend(std::iter::repeat_n(StepInput { move_dir: forward, jump: false }, 150));
    inputs.push(StepInput { move_dir: forward, jump: true });
    inputs.extend(std::iter::repeat_n(StepInput { move_dir: forward, jump: false }, 10));
    inputs.push(StepInput { move_dir: forward, jump: true });
    inputs.extend(std::iter::repeat_n(StepInput { move_dir: forward, jump: false }, 40));
    inputs.extend(std::iter::repeat_n(StepInput { move_dir: glancing, jump: false }, 60));
    inputs
}

fn run(world: &World, inputs: &[StepInput]) -> Vec<Player> {
    // Replay constructs the shipped defaults, never the app's live tuning: that is what keeps the
    // determinism contract intact while the file is free to change under a play session.
    let tuning = Tuning::default();
    let start = player_at(Vec3::new(6.0, 8.0, 6.0));
    let mut state = start;
    let mut trajectory = Vec::with_capacity(inputs.len());
    for &input in inputs {
        state = sim::step(state, input, world, &tuning);
        trajectory.push(state);
    }
    trajectory
}

/// The raw bits of each component, so equality is exact rather than float `==` (which would treat
/// `0.0` and `-0.0` as equal despite differing bits).
fn bits(v: Vec3) -> [u32; 3] {
    [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()]
}

/// A player at `position` with no velocity, jumps full, the still timer reset.
fn player_at(position: Vec3) -> Player {
    Player {
        motion: Motion { position, velocity: Vec3::ZERO },
        jumps_remaining: Tuning::default().max_jumps,
        still_time: 0.0,
    }
}

/// Settle a player under no input for `steps` fixed steps.
fn settle(world: &World, start: Player, steps: usize) -> Player {
    let tuning = Tuning::default();
    let mut state = start;
    for _ in 0..steps {
        state = sim::step(state, StepInput::default(), world, &tuning);
    }
    state
}

/// The y of the body's lowest point for a centre at `center`: the cylinder's flat base sits half
/// the total height below the centre.
fn base_height(center: Vec3) -> f32 {
    center.y - PLAYER_HEIGHT * 0.5
}

#[test]
fn the_world_reduction_classifies_each_hitbox_into_its_own_shape() {
    // The fixture's wall is a scaled cube and its pillar a uniform-xz cylinder; through the real
    // store-to-world path the wall must stay a conservative box and the pillar must come out a true
    // vertical cylinder at the placement's dimensions - not be lifted to its box.
    let world = fixture_world();
    assert_eq!(world.statics.len(), 2);
    assert!(
        world.statics.iter().any(|c| matches!(c, Collider::Aabb(_))),
        "the cube wall should reduce to a box"
    );
    let round = world.statics.iter().find_map(|c| match *c {
        Collider::VertCylinder { center, radius, half_height } => Some((center, radius, half_height)),
        _ => None,
    });
    let (center, radius, half_height) = round.expect("the cylinder pillar should classify round");
    assert!((center - Vec3::new(30.0, 4.0, 30.0)).length() < 1e-5, "pillar centre: {center:?}");
    assert!((radius - 1.0).abs() < 1e-5, "pillar radius: {radius}");
    assert!((half_height - 4.0).abs() < 1e-5, "pillar half-height: {half_height}");
}

#[test]
fn at_rest_on_flat_ground_the_base_sits_exactly_on_the_surface() {
    // The at-rest convention tie: after settling on flat terrain the cylinder's lowest point is the
    // sampled surface height - no constant offset anywhere in the foot-vs-centre bookkeeping across
    // sim, world, and the rest query. Measured residue is a couple of ulps: the lift's
    // `base + (ground - base)` add and reconstructing the base out of the centre. A convention bug
    // would be centimetres, which is what this catches.
    let world = fixture_world();
    let rested = settle(&world, player_at(Vec3::new(6.0, 8.0, 6.0)), 240);
    let ground = world.terrains[0].heightmap.height_at(6.0, 6.0);

    let base = base_height(rested.motion.position);
    assert!(
        (base - ground).abs() <= 4.0 * f32::EPSILON * ground.abs(),
        "base {base} should rest exactly on the surface {ground} (gap {})",
        base - ground
    );
    assert!(rested.motion.velocity.y.abs() <= crate::constants::STILL_VY, "a settled body is vertically still");
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
        // The stepped jump state must replay exactly too, or a divergence could hide in state the
        // position has not expressed yet.
        assert_eq!(a.jumps_remaining, b.jumps_remaining, "jumps remaining differ at step {i}");
        assert_eq!(a.still_time.to_bits(), b.still_time.to_bits(), "the still timer differs at step {i}");
    }
}

#[test]
fn the_scripted_run_actually_exercises_the_arcs() {
    // Guard against a degenerate script silently passing the bitwise test: the run really did fall,
    // land, stop at the wall, leave the ground twice, and slide along the wall.
    let world = fixture_world();
    let inputs = scripted_inputs();
    let traj = run(&world, &inputs);
    let t = Tuning::default();

    // The fall lands: the idle phase ends resting on the flat ground.
    assert!(traj[0].motion.position.y > FLAT_HEIGHT_M + 1.0, "the run starts well above the ground");
    let landed = &traj[89];
    assert!((base_height(landed.motion.position) - FLAT_HEIGHT_M).abs() < 1e-2, "the idle phase should land flat");

    // The head-on walk ends pinned at the wall's face: the slide projects out the into-wall motion,
    // so the body stops a skin short of (face - radius) and never penetrates.
    let pin = WALL_NEAR_X - PLAYER_RADIUS;
    let at_wall = &traj[239];
    assert!(at_wall.motion.position.x <= pin + 1e-2, "penetrated the wall: x = {}", at_wall.motion.position.x);
    assert!(at_wall.motion.position.x >= pin - 1e-1, "never reached the wall: x = {}", at_wall.motion.position.x);

    // The jump at step 240 leaves the ground and gains height; the double jump at step 251 spends
    // the second of the two jumps, so the counter bottoms out at zero.
    assert!(
        traj[245].motion.position.y > traj[239].motion.position.y + 0.2,
        "the jump should gain height: {} -> {}",
        traj[239].motion.position.y,
        traj[245].motion.position.y
    );
    assert_eq!(traj[240].jumps_remaining, t.max_jumps - 1, "the first jump spends one");
    assert_eq!(traj[251].jumps_remaining, t.max_jumps - 2, "the double jump spends the second");

    // The glancing phase slides along the wall: z advances while x stays pinned by the wall.
    let before = &traj[291];
    let last = traj.last().unwrap();
    assert!(last.motion.position.z > before.motion.position.z + 2.0, "should have slid along the wall in z");
    assert!(last.motion.position.x <= pin + 1e-2, "still pinned by the wall: x = {}", last.motion.position.x);
}
