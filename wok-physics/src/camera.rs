//! Follow-camera math: where the camera wants to sit, and how the world pulls it back.
//!
//! A follow camera orbits a target on a boom of some length. Three pure pieces, which the game
//! sequences each step (it owns the camera entity, its angles, its follow target, and its smoothing
//! state; this module holds nothing between calls):
//!
//! - [`boom_direction`] / [`boom_point`] - the orbit transform: turn two angles into the direction
//!   from the target out to the camera, and a point a given length along it.
//! - [`spring_arm`] - shorten the boom so the camera does not pass through static geometry between
//!   it and the target, by sweeping a probe sphere (2a's capsule sweep with a zero-length segment).
//! - [`terrain_floor`] - a vertical clamp keeping the camera above the heightmap surface.
//!
//! Orientation is left to the consumer: the camera looks at the target, so its forward is
//! `target - position`, which is `-boom_direction`. The view and projection matrices, and their
//! handedness, are wok-render's; this module builds no matrix (HLD: handedness lives in the
//! renderer).
//!
//! ## Orbit conventions (Y-up)
//!
//! At `yaw = 0`, `pitch = 0` the boom points along `+Z`: the camera sits at `target + (0, 0, 1) *
//! distance`, directly behind a target whose forward faces `-Z`.
//!
//! - `yaw` rotates about `+Y`; positive yaw swings the boom from `+Z` toward `+X` (a quarter turn,
//!   `PI / 2`, puts the camera off the target's `+X` side).
//! - `pitch` lifts the boom toward `+Y`; positive pitch raises the camera, `PI / 2` puts it directly
//!   overhead. Pitch is applied before yaw, so the camera rides a cone about the target as it swings
//!   rather than a circle that tilts with the yaw.
//!
//! ## Composition (game-owned)
//!
//! The intended per-step shape, which the game sequences and holds the state for:
//!
//! ```text
//! let dir     = boom_direction(yaw, pitch);                        // orbit angles -> boom direction
//! let want    = spring_arm(target, dir, distance, probe, statics); // clamp the boom against walls
//! arm         = smooth(arm, want, arm_half_life, dt);              // ease the length (game holds it)
//! let desired = boom_point(target, dir, arm);                      // the clamped camera position
//! let floored = terrain_floor(desired, terrain, margin);           // keep it above the ground
//! camera_pos  = smooth(camera_pos, floored, pos_half_life, dt);    // ease the follow (game holds it)
//! ```
//!
//! wok-physics provides the pieces; the game decides the order, what to smooth, and the rates. See
//! [`crate::smooth`] for the smoothing helper.
//!
//! Determinism (canon contract): every function is pure trig/arithmetic of its inputs with no state
//! and no wall-clock, and the spring arm's sweep is 2a's deterministic one (fixed iteration cap,
//! earliest impact in slice order, no parallelism). The math is relative to the target (or a purely
//! vertical clamp), so a world offset shifts target and camera together and changes nothing else -
//! position-independence to float precision. No `Result`: the queries are total over valid inputs,
//! and degenerate cases (zero distance, target inside a collider) are graceful, not errors.
//!
//! [`Aabb`]: wok_scene::Aabb
//! [`Heightmap`]: wok_scene::Heightmap

use glam::{Quat, Vec3};
use wok_scene::{Aabb, Heightmap};

use crate::capsule::Capsule;
use crate::sweep::sweep_capsule_aabbs;

/// Unit direction from the target out to the camera for orbit angles `yaw` and `pitch` (radians).
///
/// `+Z` at zero angles; positive `yaw` swings toward `+X`, positive `pitch` toward `+Y` (see the
/// module docs for the full convention). Unit length to float precision - it is a rotation of a unit
/// vector - so scaling it by a boom length gives a metric distance.
pub fn boom_direction(yaw: f32, pitch: f32) -> Vec3 {
    // Elevation first, in the y-z plane (+Z at pitch 0, tilting toward +Y as pitch grows); then the
    // yaw rotation about +Y swings that whole tilted boom around the target.
    let elevated = Vec3::new(0.0, pitch.sin(), pitch.cos());
    Quat::from_rotation_y(yaw) * elevated
}

/// The point a distance `length` out along `boom_direction` from `target`:
/// `target + boom_direction * length`.
///
/// With the full orbit `distance` it is the desired (unobstructed) camera position; with the
/// [`spring_arm`]-clamped arm length it is the collision-shortened camera position. One function for
/// both, because each is just a point on the boom.
pub fn boom_point(target: Vec3, boom_direction: Vec3, length: f32) -> Vec3 {
    target + boom_direction * length
}

/// Boom length that keeps the camera clear of static geometry between the target and the camera.
///
/// Sweeps a sphere of radius `probe_radius` from `target` along `boom_direction` (expected unit, as
/// [`boom_direction`] returns) for `distance` metres - 2a's [`sweep_capsule_aabbs`] on a capsule
/// whose segment has collapsed to a point. With an obstruction the arm is the time-of-impact
/// fraction of `distance`, the point where the probe sphere stops; that leaves its centre - where
/// the camera rides - about `probe_radius` in front of the surface, so the probe radius is the
/// standoff (the brief's "small skin"). With nothing in the way it is the full `distance`. The
/// result lies in `0.0..=distance`, since the sweep's time of impact is in `0.0..=1.0`.
///
/// Total over valid inputs: a zero `distance` produces no motion to sweep and returns `0.0`; a
/// target already inside a collider impacts at time zero and returns `0.0` (the camera collapses
/// onto the target), rather than erroring.
pub fn spring_arm(
    target: Vec3,
    boom_direction: Vec3,
    distance: f32,
    probe_radius: f32,
    statics: &[Aabb],
) -> f32 {
    let probe = Capsule::new(target, target, probe_radius);
    match sweep_capsule_aabbs(&probe, boom_direction * distance, statics) {
        // The sphere surface meets the box at `toi`; its centre is that fraction of the way out,
        // i.e. `probe_radius` short of the surface - which is where we want the camera.
        Some(hit) => hit.toi * distance,
        None => distance,
    }
}

/// Lift `camera` to sit at least `margin` metres above the terrain beneath it, returning the
/// corrected position.
///
/// The floor is `terrain.height_at(camera.x, camera.z) + margin`. If `camera.y` is below it the
/// camera is raised to the floor with its `xz` unchanged; otherwise it is returned as-is. Lift only:
/// a camera already above the floor is never pulled down. This is a coarse single-sample vertical
/// clamp, not a swept terrain contact, which is all the camera needs to avoid sinking into the
/// ground (the brief: swept terrain-vs-camera collision is out of scope).
///
/// Runs in the heightmap's frame (`xz` chunk-local metres, up `+Y`); the game maps world coordinates
/// into chunk-local space first, the same as the body rests in [`crate::terrain`]. The correction is
/// purely vertical, so it is invariant to the chunk's horizontal world offset (position-independence).
/// A zero or negative `margin` is allowed - it is the game's tuning knob, not an error.
pub fn terrain_floor(camera: Vec3, terrain: &Heightmap, margin: f32) -> Vec3 {
    let floor = terrain.height_at(camera.x, camera.z) + margin;
    if camera.y < floor {
        Vec3::new(camera.x, floor, camera.z)
    } else {
        camera
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use crate::smooth;
    use std::f32::consts::{FRAC_PI_2, FRAC_PI_4};
    use wok_scene::{CHUNK_GRID_LEN, SurfaceTag};

    const EPS: f32 = 1e-5;

    // Flat terrain at a nominal height; tests compare against the sampled height, not the nominal
    // metres (the heightmap quantizes to u16).
    fn flat(height_m: f32) -> Heightmap {
        let raw = Heightmap::meters_to_raw(height_m);
        Heightmap::new(vec![raw; CHUNK_GRID_LEN], vec![SurfaceTag::new("g")], vec![0; CHUNK_GRID_LEN]).unwrap()
    }

    // A wall blocking the +Z boom: its -Z face is at z = 5, wide in x and y.
    fn wall_at_z5() -> Aabb {
        Aabb::new(Vec3::new(-5.0, -5.0, 5.0), Vec3::new(5.0, 5.0, 6.0))
    }

    // ---- orbit ----

    #[test]
    fn zero_angles_put_the_camera_behind_along_plus_z() {
        let dir = boom_direction(0.0, 0.0);
        assert!((dir - Vec3::Z).length() < EPS, "dir = {dir:?}");
        // A 10m boom from a target at the origin sits at +Z * 10.
        let pos = boom_point(Vec3::ZERO, dir, 10.0);
        assert!((pos - Vec3::new(0.0, 0.0, 10.0)).length() < EPS, "pos = {pos:?}");
    }

    #[test]
    fn quarter_turn_yaw_swings_to_the_plus_x_side() {
        let dir = boom_direction(FRAC_PI_2, 0.0);
        assert!((dir - Vec3::X).length() < EPS, "dir = {dir:?}");
    }

    #[test]
    fn positive_pitch_raises_the_camera() {
        let flat_y = boom_point(Vec3::ZERO, boom_direction(0.0, 0.0), 5.0).y;
        let lifted_y = boom_point(Vec3::ZERO, boom_direction(0.0, FRAC_PI_4), 5.0).y;
        assert!(lifted_y > flat_y + 1.0, "positive pitch should raise the camera: {flat_y} -> {lifted_y}");
        // Straight overhead at pitch = 90 degrees.
        assert!((boom_direction(0.0, FRAC_PI_2) - Vec3::Y).length() < EPS);
    }

    #[test]
    fn boom_direction_is_unit_length() {
        for &(y, p) in &[(0.0, 0.0), (FRAC_PI_4, FRAC_PI_4), (1.3, -0.7), (-2.1, 0.9)] {
            let len = boom_direction(y, p).length();
            assert!((len - 1.0).abs() < EPS, "len = {len} for yaw {y} pitch {p}");
        }
    }

    #[test]
    fn orbit_is_position_independent() {
        // The boom is relative to the target: moving the target by an offset moves the camera by the
        // same offset and leaves the boom direction untouched.
        let dir = boom_direction(0.9, 0.3);
        let here = boom_point(Vec3::ZERO, dir, 6.0);
        let offset = Vec3::new(1000.0, -200.0, 512.0);
        let there = boom_point(offset, dir, 6.0);
        assert!((there - (here + offset)).length() < 1e-3, "orbit drifted under a world offset");
    }

    // ---- spring arm ----

    #[test]
    fn no_obstruction_returns_the_full_distance() {
        assert_eq!(spring_arm(Vec3::ZERO, Vec3::Z, 10.0, 0.5, &[]), 10.0);
    }

    #[test]
    fn an_obstruction_shortens_the_arm_to_just_in_front() {
        // Probe radius 0.5, wall face at z = 5: the sphere centre stops at z = 4.5, so the arm is 4.5
        // of the 10, and the camera sits just in front of the wall.
        let arm = spring_arm(Vec3::ZERO, Vec3::Z, 10.0, 0.5, &[wall_at_z5()]);
        assert!((arm - 4.5).abs() < 1e-2, "arm = {arm}");
        let cam = boom_point(Vec3::ZERO, Vec3::Z, arm);
        assert!(cam.z < 5.0, "camera should sit in front of the wall face, z = {}", cam.z);
    }

    #[test]
    fn a_larger_probe_holds_the_camera_further_from_the_wall() {
        // The probe radius is the standoff: a bigger sphere stops sooner, shortening the arm more.
        let small = spring_arm(Vec3::ZERO, Vec3::Z, 10.0, 0.5, &[wall_at_z5()]);
        let big = spring_arm(Vec3::ZERO, Vec3::Z, 10.0, 1.0, &[wall_at_z5()]);
        assert!((small - 4.5).abs() < 1e-2, "small = {small}");
        assert!((big - 4.0).abs() < 1e-2, "big = {big}");
        assert!(big < small, "a larger probe radius must shorten the arm more");
    }

    #[test]
    fn target_inside_a_collider_collapses_the_arm() {
        // Degenerate but must be graceful: the probe starts inside the box, impacts at time zero, so
        // the arm is zero (the camera collapses onto the target) rather than erroring.
        let around = Aabb::new(Vec3::splat(-2.0), Vec3::splat(2.0));
        assert_eq!(spring_arm(Vec3::ZERO, Vec3::Z, 10.0, 0.5, &[around]), 0.0);
    }

    #[test]
    fn zero_distance_returns_a_zero_arm() {
        assert_eq!(spring_arm(Vec3::ZERO, Vec3::Z, 0.0, 0.5, &[wall_at_z5()]), 0.0);
    }

    // ---- terrain floor ----

    #[test]
    fn camera_below_the_surface_is_lifted_above_it() {
        let terrain = flat(2.0);
        let ground = terrain.height_at(64.0, 64.0);
        let lifted = terrain_floor(Vec3::new(64.0, -10.0, 64.0), &terrain, 0.5);
        assert!((lifted.y - (ground + 0.5)).abs() < 1e-4, "y = {}", lifted.y);
        // Lifted straight up: xz unchanged.
        assert_eq!(lifted.x, 64.0);
        assert_eq!(lifted.z, 64.0);
    }

    #[test]
    fn camera_above_the_floor_is_untouched() {
        let terrain = flat(2.0);
        let cam = Vec3::new(64.0, 20.0, 64.0);
        assert_eq!(terrain_floor(cam, &terrain, 0.5), cam);
    }

    #[test]
    fn margin_holds_the_camera_clear_of_the_surface() {
        let terrain = flat(2.0);
        let ground = terrain.height_at(10.0, 10.0);
        // Sitting exactly on the surface, with a positive margin, lifts by the margin.
        let lifted = terrain_floor(Vec3::new(10.0, ground, 10.0), &terrain, 1.0);
        assert!((lifted.y - (ground + 1.0)).abs() < 1e-4, "y = {}", lifted.y);
    }

    // ---- determinism (composition) ----

    #[test]
    fn a_camera_step_sequence_reproduces_bitwise() {
        // The game-side composition (orbit -> spring arm -> smooth -> boom point -> terrain floor ->
        // smooth) over many steps with a turning camera must reproduce bitwise both runs - the
        // determinism the Level 2 replay harness relies on. wok-physics provides the pieces; this
        // just sequences them, standing in for the game loop.
        let terrain = flat(1.0);
        let statics = [wall_at_z5()];
        let dt = 1.0 / 60.0;

        let run = || {
            let target = Vec3::new(0.0, 2.0, 0.0);
            let mut arm = 8.0_f32;
            let mut cam = boom_point(target, boom_direction(0.0, 0.2), arm);
            for i in 0..240 {
                let dir = boom_direction(i as f32 * 0.01, 0.2);
                let want = spring_arm(target, dir, 8.0, 0.5, &statics);
                arm = smooth(arm, want, 0.15, dt);
                let floored = terrain_floor(boom_point(target, dir, arm), &terrain, 1.0);
                cam = smooth(cam, floored, 0.1, dt);
            }
            (cam, arm)
        };

        assert_eq!(run(), run());
    }
}
