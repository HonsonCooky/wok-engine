//! Shared locomotion replay harness: a hand-built world and the per-step physics composition.
//!
//! This is the first instance of the HLD "Level 2 deterministic replay harness". The game that owns
//! the loop does not exist yet, so this test crate stands in for it: it builds a small wok-scene
//! world in code, slices it the way the game will, reduces the resulting solid hitboxes to AABBs, and
//! drives a capsule "player" through the per-step composition the game's fixed-timestep loop will own:
//!
//!     set the horizontal velocity from the scripted input
//!     -> integrate one fixed step under gravity              (wok-physics: integrate)
//!     -> collide-and-slide the move against the static AABBs  (wok-physics: collide_and_slide)
//!     -> rest the slid capsule on the terrain                 (wok-physics: rest_on_heightmap)
//!
//! It lives in wok-physics's tests (not a separate game crate) because wok-physics already depends on
//! wok-scene, so a test here can name and compose both crates. wok-physics holds no state and no loop:
//! the player (a Capsule plus a Motion) and the loop are the test's, exactly as they will be the
//! game's. Everything runs in chunk-local coordinates, headless, with a fixed dt: no window, no GPU,
//! no wall-clock, which is what makes the run reproducible bit for bit.

use std::collections::HashMap;

use glam::{Quat, Vec3};
use wok_physics::{Capsule, Motion, collide_and_slide, integrate, rest_on_heightmap, world_aabb};
use wok_scene::{
    Aabb, CHUNK_GRID_DIM, CHUNK_GRID_LEN, Chunk, ChunkCoord, ChunkStreaming, Heightmap, InstanceId,
    Placement, Prefab, PrefabRef, PrefabState, Primitive, Shape, SurfaceTag, Transform, slice_chunk,
};

// ---- player and step constants ----

/// Player capsule total height and radius, in metres (the same shape wok-physics's own fixtures use).
pub const PLAYER_HEIGHT: f32 = 2.0;
pub const PLAYER_RADIUS: f32 = 0.5;

/// Constant downward acceleration, and the fixed timestep the loop advances by. A fixed dt (never a
/// wall-clock delta) is the day-one decision behind deterministic scripted-input replay.
const GRAVITY: Vec3 = Vec3::new(0.0, -9.8, 0.0);
const DT: f32 = 1.0 / 60.0;

/// cos(45 deg): the steepest slope still walkable, the same limit the slide and terrain queries take.
const WALKABLE_COS: f32 = std::f32::consts::FRAC_1_SQRT_2;

// ---- world geometry ----

const WALL_CENTER: Vec3 = Vec3::new(15.0, 4.0, 24.0);
const WALL_SIZE: Vec3 = Vec3::new(2.0, 8.0, 48.0);

/// The wall's near (-x) face, derived from its placement so the two cannot drift. The walk and slide
/// tests expect the player centre to pin a radius short of this.
pub const WALL_NEAR_X: f32 = WALL_CENTER.x - WALL_SIZE.x * 0.5;

const FLAT_HEIGHT_M: f32 = 2.0;

/// Cells with x index at or below this are flat; beyond it the terrain ramps up along +x.
const FLAT_MAX_CELL: u16 = 40;

/// Raw-height rise per cell on the slope (~0.098 m/cell, about 5.6 degrees): gentle and walkable.
pub const SLOPE_DELTA: u16 = 100;

/// One recorded simulation step: the player's Motion after the step, and whether it ended grounded.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub motion: Motion,
    pub grounded: bool,
}

/// The authored world the harness drives: an authored chunk, the prefabs its placements reference,
/// and the chunk's terrain. Held in authored form (not pre-sliced) so `simulate` runs the full
/// authored-to-runtime reduction on every call, exercising slice determinism alongside the physics.
pub struct Authored {
    pub chunk: Chunk,
    pub prefabs: HashMap<PrefabRef, Prefab>,
    pub terrain: Heightmap,
}

/// Build the one world all the scenarios share.
pub fn authored_world() -> Authored {
    Authored { chunk: build_chunk(), prefabs: build_prefabs(), terrain: build_heightmap() }
}

/// One fixed simulation step: the per-step composition the game will own, stood up here in the test.
fn step(state: Motion, input: Vec3, statics: &[Aabb], terrain: &Heightmap) -> Sample {
    // The scripted input is a desired horizontal velocity (x, z in m/s; its y is ignored). The
    // vertical velocity carries the gravity the body has accumulated while not yet grounded.
    let mut m = state;
    m.velocity.x = input.x;
    m.velocity.z = input.z;

    // One fixed step under gravity, then slide the resulting move along any static geometry it meets.
    let next = integrate(m, GRAVITY, DT);
    let capsule = Capsule::upright(m.position, PLAYER_HEIGHT, PLAYER_RADIUS);
    let slid = collide_and_slide(capsule, next.position - m.position, next.velocity, statics, Vec3::Y, WALKABLE_COS);

    // Finally rest the slid capsule on the terrain beneath it (lift-only; the box slide handled walls).
    let slid_capsule = Capsule::upright(slid.position, PLAYER_HEIGHT, PLAYER_RADIUS);
    let rested = rest_on_heightmap(slid_capsule, terrain, WALKABLE_COS);

    // Landing policy (the game's call, per the wok-physics docs): when the slide grounded the body on
    // box geometry, or the terrain lifted it back onto the surface, the downward fall is spent.
    let grounded = slid.grounded || rested.grounded;
    let mut velocity = slid.velocity;
    if slid.grounded || rested.position.y > slid.position.y {
        velocity.y = 0.0;
    }

    Sample { motion: Motion { position: rested.position, velocity }, grounded }
}

/// Drive `inputs` through the per-step composition from `start`, returning the per-step trajectory.
///
/// Slices the authored chunk and reduces its solid hitboxes to AABBs on every call, so running the
/// same scenario twice exercises `slice_chunk` plus `world_aabb` determinism, not only the physics.
pub fn simulate(world: &Authored, start: Motion, inputs: &[Vec3]) -> Vec<Sample> {
    let sliced = slice_chunk(&world.chunk, &world.prefabs).expect("authored chunk should slice cleanly");
    let statics: Vec<Aabb> = sliced.hitboxes.iter().map(|h| world_aabb(h.primitive, h.transform)).collect();

    let mut state = start;
    let mut trajectory = Vec::with_capacity(inputs.len());
    for &input in inputs {
        let sample = step(state, input, &statics, &world.terrain);
        state = sample.motion;
        trajectory.push(sample);
    }
    trajectory
}

/// A player Motion standing at rest on the terrain at chunk-local `(x, z)`: feet on the surface.
pub fn grounded_start(terrain: &Heightmap, x: f32, z: f32) -> Motion {
    Motion { position: Vec3::new(x, terrain.height_at(x, z) + PLAYER_HEIGHT * 0.5, z), velocity: Vec3::ZERO }
}

/// The y of the capsule's feet for a centre at `center`. For an upright capsule the base sits half the
/// total height below the centre (segment half-length plus radius), independent of the radius split.
pub fn foot_height(center: Vec3) -> f32 {
    center.y - PLAYER_HEIGHT * 0.5
}

/// Assert two trajectories are identical bit for bit. This is the determinism contract's literal
/// property (identical inputs reproduce bitwise), stronger than equality within a tolerance.
pub fn assert_bitwise_eq(a: &[Sample], b: &[Sample]) {
    assert_eq!(a.len(), b.len(), "trajectory lengths differ");
    for (i, (x, y)) in a.iter().zip(b).enumerate() {
        assert_eq!(bits(x.motion.position), bits(y.motion.position), "position differs at step {i}");
        assert_eq!(bits(x.motion.velocity), bits(y.motion.velocity), "velocity differs at step {i}");
        assert_eq!(x.grounded, y.grounded, "grounded flag differs at step {i}");
    }
}

/// The raw bits of each component, so equality is exact rather than float `==` (which would treat
/// `0.0` and `-0.0` as equal despite differing bits).
fn bits(v: Vec3) -> [u32; 3] {
    [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()]
}

// ---- the hand-built world ----

/// A solid placeholder prefab: a single unit Cube that is both a collision hitbox and visible. The
/// placement transform scales it into a wall or a pillar.
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

/// A trigger-volume prefab: a single unit Cube that is a hitbox but not visible, so the slicer routes
/// it to the trigger array (tagged with the placement instance), never to the collision array.
fn trigger_zone() -> Prefab {
    Prefab {
        states: vec![PrefabState {
            name: "default".into(),
            shapes: vec![Shape {
                primitive: Primitive::Cube,
                transform: Transform::IDENTITY,
                surface: None,
                is_hitbox: true,
                is_visible: false,
            }],
            mesh: None,
        }],
        default_state: "default".into(),
    }
}

fn build_prefabs() -> HashMap<PrefabRef, Prefab> {
    let mut prefabs = HashMap::new();
    prefabs.insert(PrefabRef::new("block"), solid_block());
    prefabs.insert(PrefabRef::new("zone"), trigger_zone());
    prefabs
}

/// One placement of `prefab`, scaled and positioned by `center` and `size`: the unit cube spans the
/// box `center +/- size/2`, per the engine's unit-primitive convention (scale reads as size in m).
fn placement(prefab: &str, id: u32, center: Vec3, size: Vec3) -> Placement {
    Placement {
        prefab: PrefabRef::new(prefab),
        instance_id: InstanceId(id),
        transform: Transform { translation: center, rotation: Quat::IDENTITY, scale: size },
        state: None,
    }
}

fn build_chunk() -> Chunk {
    Chunk {
        coord: ChunkCoord::new(0, 0),
        placements: vec![
            // A long wall along z, near (-x) face at WALL_NEAR_X: the surface the walk/slide tests meet.
            placement("block", 1, WALL_CENTER, WALL_SIZE),
            // A pillar off the walked paths: a second hitbox, so collision is shown to be selective.
            placement("block", 2, Vec3::new(30.0, 4.0, 30.0), Vec3::new(2.0, 8.0, 2.0)),
            // A trigger volume on the flat ground: present and sliced, but never collided against.
            placement("zone", 3, Vec3::new(6.0, 3.0, 6.0), Vec3::new(3.0, 3.0, 3.0)),
        ],
        streaming: ChunkStreaming::default(),
    }
}

/// A heightmap flat at `FLAT_HEIGHT_M` out to `FLAT_MAX_CELL` in x, then ramping up along +x by
/// `SLOPE_DELTA` raw units per cell: a flat region to rest on and a gentle slope to walk across.
fn build_heightmap() -> Heightmap {
    let base = Heightmap::meters_to_raw(FLAT_HEIGHT_M);
    let heights = (0..CHUNK_GRID_LEN)
        .map(|i| {
            let cx = (i % CHUNK_GRID_DIM) as u16;
            if cx <= FLAT_MAX_CELL { base } else { base + (cx - FLAT_MAX_CELL) * SLOPE_DELTA }
        })
        .collect();
    Heightmap::new(heights, vec![SurfaceTag::new("ground")], vec![0; CHUNK_GRID_LEN]).unwrap()
}
