//! wok-physics: pure math for moving things through 3D space.
//!
//! Every item here is a function of its inputs: no actor pool, no integration loop, no stored
//! state, no wall-clock. The game holds actor state and owns the fixed-timestep loop; it calls
//! these functions each step. The editor calls the same functions for placement and picking. This
//! split is HLD principle 5 (math in the engine, state and loops in the game) and is what lets the
//! game's loop be deterministic for scripted-input replay.
//!
//! Part 1 is AABB-only collision plus fixed-step integration:
//!
//! - [`bounds`] - reduce a transformed primitive to a conservative world-space [`Aabb`]
//!   ([`world_aabb`]), and the unit-primitive convention it rests on.
//! - [`collision`] - AABB-vs-AABB overlap and minimum translation ([`aabb_contact`]), and
//!   sequential resolution of a player box against static boxes ([`resolve_statics`]).
//! - [`motion`] - one fixed-step kinematic update under constant acceleration ([`integrate`]).
//!
//! Part 2a added the swept rounded-shape collision the character controller needs:
//!
//! - [`capsule`] - the moving shape, a segment plus a radius ([`Capsule`]), with an upright
//!   convenience constructor.
//! - [`sweep`] - the earliest time-of-impact of a swept capsule against static [`Aabb`]s
//!   ([`sweep_capsule_aabb`], [`sweep_capsule_aabbs`]), by conservative advancement.
//! - [`slide`] - move a capsule by a displacement and slide it along whatever it hits
//!   ([`collide_and_slide`]), returning the slid velocity and a grounded flag.
//! - [`terrain`] - rest a player box ([`resolve_heightmap`]) or a capsule ([`rest_on_heightmap`])
//!   on a chunk [`Heightmap`], lift-only, with a grounded signal for the capsule.
//!
//! The intended per-step composition is the game's to sequence, not this crate's: integrate under
//! gravity, then collide-and-slide against static AABBs, then rest on terrain. wok-physics provides
//! the pieces; it never holds the body between calls.
//!
//! Part 2b (this revision) adds the follow-camera math, the last wok-physics piece for the Phase 4
//! camera. All pure again: no camera entity, no state, no loop - the game owns the camera and calls
//! these each step.
//!
//! - [`camera`] - the orbit transform ([`boom_direction`], [`boom_point`]), the spring-arm boom
//!   clamp against static [`Aabb`]s ([`spring_arm`], reusing 2a's sweep with a zero-length segment),
//!   and a vertical [`terrain_floor`] clamp above the [`Heightmap`].
//! - [`smoothing`] - a frame-rate-independent exponential smoothing helper ([`smooth`]), general
//!   over a scalar or a vector, for the game to ease the arm length, the follow position, or angles.
//!
//! The camera composition (orbit, spring-arm, terrain floor, then smoothing toward the result) is
//! again the game's to sequence and hold state for, the same split as the character controller.
//!
//! Determinism (canon contract): identical inputs and `dt` give identical outputs; resolution over
//! several colliders and the iterative sweep/slide both run sequentially in a defined order, with no
//! parallel reduction and fixed iteration caps; the collision math is position-independent (it reads
//! only relative positions, so a query answers the same regardless of absolute world position, to
//! float precision in chunk-local space). No `Result` appears here: the queries are total over valid
//! inputs, and degenerate shapes (zero radius, zero-length segment or motion) are graceful no-ops
//! rather than errors, per the brief.
//!
//! Deferred to later wok-physics steps (explicitly out of scope now): ellipsoid shapes and full
//! swept capsule-vs-terrain-slope sliding are later refinements.
//!
//! [`Aabb`]: wok_scene::Aabb
//! [`Heightmap`]: wok_scene::Heightmap

pub mod bounds;
pub mod camera;
pub mod capsule;
pub mod collision;
mod geom;
pub mod motion;
pub mod slide;
pub mod smoothing;
pub mod sweep;
pub mod terrain;

pub use bounds::{UNIT_HALF_EXTENT, aabb_center, aabb_half_extents, local_aabb, world_aabb};
pub use camera::{boom_direction, boom_point, spring_arm, terrain_floor};
pub use capsule::Capsule;
pub use collision::{Contact, aabb_contact, aabb_overlap, resolve_statics};
pub use motion::{Motion, integrate};
pub use slide::{SlideResult, collide_and_slide};
pub use smoothing::smooth;
pub use sweep::{SweptHit, sweep_capsule_aabb, sweep_capsule_aabbs};
pub use terrain::{TerrainRest, rest_on_heightmap, resolve_heightmap};
