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
//! the pieces; it never holds the body between calls. [`collide_and_slide`] and [`rest_on_heightmap`]
//! remain supported engine API, but their last application consumer moved to the cylinder path (part
//! 5): the player's current composition sweeps the flat-bottomed [`Cylinder`] under its own policy
//! loop and rests it with [`rest_cylinder_on_heightmap`]. The capsule path stays pinned by
//! `tests/locomotion_replay.rs`.
//!
//! Part 2b added the follow-camera math, the last wok-physics piece for the Phase 4 camera. All
//! pure again: no camera entity, no state, no loop - the game owns the camera and calls these each
//! step.
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
//! Part 3 widened the static side from boxes to a small collider vocabulary, so the felt surface
//! of a round prefab matches the drawn one:
//!
//! - [`collider`] - the [`Collider`] enum (`Aabb`, `Sphere`, `VertCylinder`) and
//!   [`classify_collider`], the shared reduction from a transformed primitive to the collider it
//!   collides as (exact round rules, conservative box fallback). Shared because the editor's
//!   picking wants the identical answer the game's simulation uses.
//! - [`sweep_round`] - swept capsule vs sphere (exact, closed form) and vs vertical cylinder (the
//!   box's conservative advancement over an exact cylinder projection).
//! - [`sweep`], [`slide`], [`camera`] - the multi-collider sweep, [`collide_and_slide`], and
//!   [`spring_arm`] now take `&[Collider]`; AABB-only callers wrap with `Collider::from`, and box
//!   behavior is unchanged.
//!
//! Part 4 added the oriented box, retiring the phantom shelf a rotated solid cube used to carry
//! (its conservative world AABB reached past the drawn faces):
//!
//! - [`collider`] / [`classify`] - [`Collider::Obb`] (centre, per-axis half-extents, a unit-quat
//!   rotation), classified from a `Cube` under any rigid rotation plus per-axis scale; axis-aligned
//!   cubes stay on the cheaper `Aabb` path, and the shear guard still falls back conservatively.
//!   [`basis_is_axis_aligned`] is exported so the editor's conservative-box warning shares the
//!   classification tolerance.
//! - [`sweep_obb`] - the box sweep run in the box's local frame (a rotated capsule is still a
//!   capsule, so the map is exact), contact rotated back.
//!
//! Part 5 added the moving flat-bottomed shape, the player-collider brief: the capsule's rounded
//! bottom made tilted faces unstandable and rolled bodies off edges, and a flat bottom bears on
//! anything under its disc. It is the player's current collision path (the game's policy slide in
//! taste runs over these sweeps):
//!
//! - [`cylinder`] - the moving vertical [`Cylinder`] (centre, radius, half-height; flat caps).
//! - [`sweep_cyl`] / [`sweep_cyl_round`] / [`sweep_cyl_obb`] - the swept cylinder against every
//!   [`Collider`] shape, by the same conservative advancement run on solid-to-solid closest pairs
//!   (alternating projection over the exact [`geom`] projections), with the earliest-impact
//!   dispatch ([`sweep_cylinder_colliders`]) mirroring the capsule's and taking the slide skin
//!   directly. Flat faces, walls, and caps are exact to the shared `GAP_EPS`; rim and edge
//!   contacts may be conservatively rounded within the projection loop's residual (documented in
//!   [`sweep_cyl`]).
//! - [`terrain_cyl`] - flat-bottom terrain rest ([`rest_cylinder_on_heightmap`]): footprint-MAX
//!   over disc samples with no curvature discount (a disc rests on the highest point under it),
//!   lift-only like every rest here.
//!
//! Editor picking adds the one ray query the simulation never needed (the game sweeps shapes; only
//! the viewport's cursor casts a ray):
//!
//! - [`pick`] - [`ray_collider`], the smallest non-negative distance at which a ray enters a
//!   [`Collider`], over all four variants (the slab method for the boxes, the closed-form entering
//!   root for the sphere mirroring [`sweep_round`]'s end-sphere math, the radial quadratic plus cap
//!   slab for the vertical cylinder). It runs over the same [`classify_collider`] output the game
//!   collides against, so a pick selects the solid the placement actually collides as.
//!
//! Determinism (canon contract): identical inputs and `dt` give identical outputs; resolution over
//! several colliders and the iterative sweep/slide both run sequentially in a defined order, with no
//! parallel reduction and fixed iteration caps; the collision math is position-independent (it reads
//! only relative positions, so a query answers the same regardless of absolute world position, to
//! float precision in chunk-local space). No `Result` appears here: the queries are total over valid
//! inputs, and degenerate shapes (zero radius, zero-length segment or motion) are graceful no-ops
//! rather than errors, per the brief.
//!
//! Deferred to later wok-physics steps (explicitly out of scope now): tilted or non-uniformly
//! scaled round colliders (they stay conservative boxes by classification), capsule prefab
//! colliders, broadphase, and full swept capsule-vs-terrain-slope sliding.
//!
//! [`Aabb`]: wok_scene::Aabb
//! [`Heightmap`]: wok_scene::Heightmap

pub mod bounds;
pub mod camera;
pub mod capsule;
pub mod classify;
pub mod collider;
pub mod collision;
pub mod cylinder;
mod geom;
pub mod motion;
pub mod pick;
pub mod slide;
pub mod smoothing;
pub mod sweep;
pub mod sweep_cyl;
pub mod sweep_cyl_obb;
pub mod sweep_cyl_round;
pub mod sweep_obb;
pub mod sweep_round;
pub mod terrain;
pub mod terrain_cyl;

pub use bounds::{aabb_center, aabb_half_extents, world_aabb};
pub use camera::{boom_direction, boom_point, spring_arm, terrain_floor};
pub use capsule::Capsule;
pub use classify::{basis_is_axis_aligned, classify_collider};
pub use collider::Collider;
pub use collision::{Contact, aabb_contact, aabb_overlap, resolve_statics};
pub use cylinder::Cylinder;
pub use motion::{Motion, integrate};
pub use pick::ray_collider;
pub use slide::{SlideResult, collide_and_slide};
pub use smoothing::smooth;
pub use sweep::{SweptHit, sweep_capsule_aabb, sweep_capsule_aabbs, sweep_capsule_collider, sweep_capsule_colliders};
pub use sweep_cyl::{sweep_cylinder_aabb, sweep_cylinder_collider, sweep_cylinder_colliders};
pub use sweep_cyl_obb::sweep_cylinder_obb;
pub use sweep_cyl_round::{sweep_cylinder_sphere, sweep_cylinder_vert_cylinder};
pub use sweep_obb::sweep_capsule_obb;
pub use sweep_round::{sweep_capsule_cylinder, sweep_capsule_sphere};
pub use terrain::{TerrainRest, rest_on_heightmap, resolve_heightmap};
pub use terrain_cyl::rest_cylinder_on_heightmap;
