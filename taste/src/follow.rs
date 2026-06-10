//! The third-person follow camera: game-owned state sequencing wok-physics's camera math.
//!
//! The model splits what the player owns from what the world may do about it. The orbit - yaw,
//! pitch, and the desired boom length - is the player's: look input writes the angles directly and
//! in full the same frame (no smoothing on orbit angles, ever), and walls never write back into
//! it, so no collision can displace the orbit the player set. Everything else is derived from the
//! orbit each rendered frame:
//!
//!     yaw, pitch += look                                                   direct, zero lag
//!     anchor   = smooth(anchor, target, CAMERA_TRACK_SMOOTH, dt)           the one tracking lag
//!     dir      = boom_direction(yaw, pitch)                                orbit -> boom direction
//!     probe    = spring_arm(anchor, dir, boom, ...)                        swept obstruction, from the anchor
//!     arm      = min(smooth(arm, boom, CAMERA_ARM_RECOVER, dt), probe)     instant clamp in, slow recovery out
//!     eye      = boom_point(anchor, dir, arm)
//!     position = terrain_floor(eye)                                        vertical clamp; if it engaged,
//!     pitch, arm <- recomputed from position - anchor                      the clamp writes the orbit back
//!     aim      = anchor + horizontal_forward * LOOK_AHEAD_M (+ lift)       the view leads ahead (render only)
//!
//! The anchor is the boom's hanging point: the player's draw position plus the look-target lift,
//! trailed with one short smooth, vertical included so jumps and falls track. The arm asymmetry is
//! the `min`: a wall clamps the arm inward the same frame the probe sees it (easing into a wall
//! would show the camera inside geometry), while clearance recovers toward the desired boom on the
//! slow smooth, so the boom drifts back out instead of whipping.
//!
//! The terrain floor is the one deliberate exception to "the world never writes the orbit". A
//! purely derived clamp leaves the orbit believing in a virtual eye below the ground, and the
//! player's next pitch input is spent invisibly climbing that belief back through the clamp - a
//! felt dead zone. So while the floor is clamping, the clamped reality is written back: pitch and
//! arm are recomputed from the actual clamped eye relative to the anchor (never yaw - the clamp is
//! vertical, so the boom's heading cannot change), and displayed and believed positions agree, so
//! input responds immediately from the visible position. Walls keep the clamp-and-recover model
//! unchanged: a wall is an obstruction to recover from, the ground is a fact to stand on.
//!
//! The spring arm sweeps against the same world-space AABBs the player collides with; terrain is a
//! vertical floor clamp (wok-physics's terrain_floor design: a swept terrain contact is explicitly
//! out of the camera math's scope), applied in the local frame of the chunk under the camera. The
//! camera updates per rendered frame with the frame's dt, not per fixed step: it is presentation,
//! not simulation, and the smoothing helper is frame-rate independent by construction, so the two
//! rates may differ without changing where the camera settles.

use glam::{Mat4, Vec2, Vec3};
use wok_physics::{boom_direction, boom_point, smooth, spring_arm, terrain_floor};

use crate::constants::{
    CAMERA_ARM_RECOVER, CAMERA_DISTANCE, CAMERA_PROBE_RADIUS, CAMERA_TERRAIN_MARGIN, CAMERA_TRACK_SMOOTH,
    FOV_Y_RADIANS, LOOK_AHEAD_LIFT_M, LOOK_AHEAD_M, NEAR_PLANE, PITCH_DEFAULT, PITCH_MAX, PITCH_MIN,
};
use crate::world::World;

/// The camera's whole state between frames. The orbit (`yaw`, `pitch`, `boom`) belongs to the
/// player: look input writes it (or a future zoom, for `boom`), and the terrain floor's write-back
/// is the one world influence (`pitch` and `arm` take the clamped reality; see the module docs).
/// `arm` and `anchor` are the two smoothed follow values; `position` is the derived eye, kept only
/// so the renderer reads the same point the last update produced.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FollowCamera {
    pub yaw: f32,
    pub pitch: f32,
    /// Desired boom length: where the arm settles with nothing in the way.
    pub boom: f32,
    /// Current arm length: clamped inward instantly by obstruction, recovering slowly toward
    /// `boom` when clear. In `0.0..=boom` while only walls act on it; the floor write-back can
    /// briefly hold it past `boom`, and recovery eases it back.
    pub arm: f32,
    /// The boom's hanging point: the look target trailed by the tracking smooth.
    pub anchor: Vec3,
    /// The derived eye position from the last update, after the terrain floor clamp.
    pub position: Vec3,
}

impl FollowCamera {
    /// A camera already settled behind `target` at the default pitch and full boom: the spawn shot,
    /// with no easing-in from some arbitrary origin.
    pub fn spawn(target: Vec3) -> FollowCamera {
        let (yaw, pitch) = (0.0, PITCH_DEFAULT);
        FollowCamera {
            yaw,
            pitch,
            boom: CAMERA_DISTANCE,
            arm: CAMERA_DISTANCE,
            anchor: target,
            position: boom_point(target, boom_direction(yaw, pitch), CAMERA_DISTANCE),
        }
    }

    /// The point the camera aims at: the anchor led `LOOK_AHEAD_M` along the camera's horizontal
    /// forward (the yaw direction at zero pitch, the same "forward" movement resolves against),
    /// trimmed by `LOOK_AHEAD_LIFT_M`. The lead must be horizontal: along the pitched boom axis
    /// the aim stays collinear with the eye and anchor and the framing would not change at all.
    /// Led flat, the eye-to-aim ray passes over the anchor, so the player drops to low-centre and
    /// the pitch the player holds decides how strongly. The eye, orbit, and arm math never see
    /// this point.
    pub fn look_target(&self) -> Vec3 {
        let ahead = -boom_direction(self.yaw, 0.0);
        self.anchor + ahead * LOOK_AHEAD_M + Vec3::Y * LOOK_AHEAD_LIFT_M
    }

    /// The combined view-projection matrix, looking from the camera at its look-ahead target, with
    /// the far plane supplied per frame (fog distance sets render distance, per the HLD).
    /// `perspective_rh` maps depth to `0..=1`, wgpu's clip-space convention. Should eye and target
    /// ever coincide (a zero look-ahead on a fully collapsed boom) the look direction degenerates;
    /// fall back to the boom's own axis so the matrix stays finite.
    pub fn view_proj(&self, aspect: f32, far: f32) -> Mat4 {
        let forward = (self.look_target() - self.position)
            .try_normalize()
            .unwrap_or_else(|| -boom_direction(self.yaw, self.pitch));
        let projection = Mat4::perspective_rh(FOV_Y_RADIANS, aspect, NEAR_PLANE, far);
        let view = Mat4::look_to_rh(self.position, forward, Vec3::Y);
        projection * view
    }
}

/// Advance the camera by one rendered frame: the composition in the module docs. Pure: identical
/// inputs give an identical next state.
pub fn update(camera: &FollowCamera, target: Vec3, look_delta: Vec2, world: &World, dt: f32) -> FollowCamera {
    // The orbit is the player's: the look delta lands whole, this frame.
    let yaw = camera.yaw + look_delta.x;
    let pitch = (camera.pitch + look_delta.y).clamp(PITCH_MIN, PITCH_MAX);

    // The anchor trails the target with the one tracking lag, vertical included.
    let anchor = smooth(camera.anchor, target, CAMERA_TRACK_SMOOTH, dt);

    // The arm: obstruction clamps instantly inward (the min against the probe), clearance recovers
    // toward the desired boom on the slow smooth. The probe sweeps from the actual anchor, so the
    // clamp matches what the eye would really pass through.
    let dir = boom_direction(yaw, pitch);
    let probe = spring_arm(anchor, dir, camera.boom, CAMERA_PROBE_RADIUS, &world.statics);
    let arm = smooth(camera.arm, camera.boom, CAMERA_ARM_RECOVER, dt).min(probe);

    // The eye is derived, then the terrain floor clamps it vertically. When the clamp engages,
    // the clamped reality is written back into pitch and arm (yaw cannot change: the clamp is
    // vertical, so the boom's heading is exactly preserved), so the orbit never believes in a
    // virtual eye below the ground and the next pitch input acts from the visible position - the
    // dead-zone fix in the module docs. Without the clamp, nothing is written back.
    let eye = boom_point(anchor, dir, arm);
    let floored = match world.terrain_under(eye.x, eye.z) {
        Some(t) => terrain_floor(eye - t.origin, &t.heightmap, CAMERA_TERRAIN_MARGIN) + t.origin,
        None => eye,
    };
    let (pitch, arm) = if floored.y > eye.y {
        let v = floored - anchor;
        (v.y.atan2(Vec3::new(v.x, 0.0, v.z).length()), v.length())
    } else {
        (pitch, arm)
    };

    FollowCamera { yaw, pitch, boom: camera.boom, arm, anchor, position: floored }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use wok_scene::{Aabb, CHUNK_GRID_LEN, Heightmap, SurfaceTag};

    const DT: f32 = 1.0 / 60.0;

    fn empty_world() -> World {
        World { statics: vec![], terrains: vec![] }
    }

    fn flat_world(height_m: f32) -> World {
        let raw = Heightmap::meters_to_raw(height_m);
        let heightmap =
            Heightmap::new(vec![raw; CHUNK_GRID_LEN], vec![SurfaceTag::new("g")], vec![0; CHUNK_GRID_LEN]).unwrap();
        World { statics: vec![], terrains: vec![crate::world::ChunkTerrain { origin: Vec3::ZERO, heightmap }] }
    }

    /// A wall a metre and a half behind a target at z = 64 on the +Z boom: its near face is at
    /// z = 65.5, wide in x and tall enough to block the boom at any test pitch.
    fn walled_world() -> World {
        World {
            statics: vec![Aabb::new(Vec3::new(50.0, 0.0, 65.5), Vec3::new(80.0, 10.0, 67.0)).into()],
            terrains: vec![],
        }
    }

    #[test]
    fn spawn_settles_behind_the_target_at_full_boom() {
        let target = Vec3::new(64.0, 5.0, 64.0);
        let cam = FollowCamera::spawn(target);
        assert_eq!(cam.arm, CAMERA_DISTANCE);
        assert_eq!(cam.anchor, target, "the anchor starts settled on the target, no easing-in");
        assert!(((cam.position - target).length() - CAMERA_DISTANCE).abs() < 1e-4);
        assert!(cam.position.z > target.z, "zero yaw puts the camera on the +Z side");
        assert!(cam.position.y > target.y, "the default pitch looks gently down");
    }

    #[test]
    fn pitch_clamps_to_its_range() {
        let world = empty_world();
        let target = Vec3::new(64.0, 10.0, 64.0);
        let cam = update(&FollowCamera::spawn(target), target, Vec2::new(0.0, 100.0), &world, DT);
        assert_eq!(cam.pitch, PITCH_MAX);
        let cam = update(&cam, target, Vec2::new(0.0, -100.0), &world, DT);
        assert_eq!(cam.pitch, PITCH_MIN);
    }

    #[test]
    fn look_input_lands_in_full_the_same_frame() {
        // No smoothing on orbit angles, ever: one update applies the whole delta, exactly, and the
        // derived eye already sits on the new boom rather than easing toward it.
        let world = empty_world();
        let target = Vec3::new(64.0, 5.0, 64.0);
        let cam = FollowCamera::spawn(target);
        let turned = update(&cam, target, Vec2::new(0.3, 0.2), &world, DT);
        assert_eq!(turned.yaw, cam.yaw + 0.3);
        assert_eq!(turned.pitch, cam.pitch + 0.2);
        let on_boom = (turned.position - turned.anchor) / turned.arm;
        assert!(
            (on_boom - boom_direction(turned.yaw, turned.pitch)).length() < 1e-5,
            "the eye should sit on the new boom immediately: {on_boom:?}"
        );
    }

    #[test]
    fn an_obstruction_clamps_the_arm_the_same_frame() {
        // The wall's near face is 1.5m behind the target: a single update pulls the arm from the
        // full boom to inside that distance - no frames of the camera easing through the wall.
        let target = Vec3::new(64.0, 5.0, 64.0);
        let world = walled_world();
        let cam = FollowCamera { yaw: 0.0, pitch: 0.0, boom: CAMERA_DISTANCE, arm: CAMERA_DISTANCE,
            anchor: target, position: target + Vec3::Z * CAMERA_DISTANCE };
        let clamped = update(&cam, target, Vec2::ZERO, &world, DT);
        assert!(clamped.arm < 1.5, "arm should clamp inside the wall distance at once: {}", clamped.arm);
        assert!(clamped.arm > 0.5, "but not collapse onto the target: {}", clamped.arm);
        assert!(clamped.position.z < 65.5, "camera should sit in front of the wall: {}", clamped.position.z);
        assert_eq!(clamped.boom, CAMERA_DISTANCE, "obstruction must not write into the desired boom");
    }

    #[test]
    fn clearance_recovers_to_the_exact_prior_eye() {
        // Obstruction then clearance: because the wall never wrote into yaw, pitch, or the desired
        // boom, recovery converges back to the eye position the player had before the wall.
        let target = Vec3::new(64.0, 5.0, 64.0);
        let open = empty_world();
        let walled = walled_world();
        let mut cam = FollowCamera::spawn(target);
        let before = cam.position;

        for _ in 0..120 {
            cam = update(&cam, target, Vec2::ZERO, &walled, DT);
        }
        assert!(cam.arm < CAMERA_DISTANCE * 0.5, "the wall should have pulled the arm in: {}", cam.arm);
        assert!((cam.position - before).length() > 1.0, "the clamped eye should have moved well inward");

        // 10 seconds open: 25 half-lives of recovery, the residual gap is sub-micrometre.
        for _ in 0..600 {
            cam = update(&cam, target, Vec2::ZERO, &open, DT);
        }
        assert_eq!(cam.yaw, 0.0, "nothing the world did may have touched the orbit");
        assert_eq!(cam.pitch, PITCH_DEFAULT);
        assert!(
            (cam.position - before).length() < 1e-3,
            "the eye should recover to where it was before the wall: {} vs {}",
            cam.position,
            before
        );
    }

    #[test]
    fn the_anchor_tracks_a_vertical_step_within_the_settling_time() {
        // A jump-sized step straight up: after ten tracking half-lives the anchor's remaining gap
        // is about a thousandth of the step - settled, vertical included.
        let world = empty_world();
        let target = Vec3::new(64.0, 5.0, 64.0);
        let mut cam = FollowCamera::spawn(target);
        let stepped = target + Vec3::Y * 2.0;
        let frames = (10.0 * CAMERA_TRACK_SMOOTH / DT).ceil() as usize;
        for _ in 0..frames {
            cam = update(&cam, stepped, Vec2::ZERO, &world, DT);
        }
        assert!(
            (cam.anchor - stepped).length() < 0.005,
            "anchor should have settled on the stepped target: {} vs {}",
            cam.anchor,
            stepped
        );
        assert!(cam.position.y > FollowCamera::spawn(target).position.y + 1.9, "the eye rides the anchor up");
    }

    #[test]
    fn the_terrain_floor_keeps_a_low_camera_above_the_ground() {
        // Terrain at 8m, target low and pitch at the floor: the unfloored boom dips under the
        // surface; the clamp holds the margin the same frame, and the write-back makes the orbit
        // believe exactly the eye that is displayed (re-deriving the boom from the written-back
        // pitch and arm lands on the clamped position).
        let world = flat_world(8.0);
        let ground = world.terrains[0].heightmap.height_at(64.0, 70.0);
        let target = Vec3::new(64.0, 8.5, 64.0);
        let cam = FollowCamera { yaw: 0.0, pitch: PITCH_MIN, boom: CAMERA_DISTANCE, arm: CAMERA_DISTANCE,
            anchor: target, position: target + Vec3::Z };
        let floored = update(&cam, target, Vec2::ZERO, &world, DT);
        assert!(
            floored.position.y >= ground + CAMERA_TERRAIN_MARGIN - 1e-3,
            "camera y {} should hold the margin above the surface {}",
            floored.position.y,
            ground
        );
        let believed = boom_point(floored.anchor, boom_direction(floored.yaw, floored.pitch), floored.arm);
        assert!(
            (believed - floored.position).length() < 1e-4,
            "the written-back orbit must reproduce the displayed eye: {believed:?} vs {:?}",
            floored.position
        );
        assert!(floored.pitch > PITCH_MIN, "the clamp should have raised the believed pitch");
        assert_eq!(floored.yaw, 0.0, "a vertical clamp can never write yaw");
    }

    #[test]
    fn a_floor_clamped_camera_answers_pitch_input_the_same_frame() {
        // The dead-zone fix: settle into the floor clamp, then push the pitch up. Because the
        // orbit believes the clamped pitch (not the virtual below-ground one), the input must
        // move the eye this very frame instead of being spent climbing back through the clamp.
        let world = flat_world(8.0);
        let target = Vec3::new(64.0, 8.5, 64.0);
        let mut cam = FollowCamera { yaw: 0.0, pitch: PITCH_MIN, boom: CAMERA_DISTANCE, arm: CAMERA_DISTANCE,
            anchor: target, position: target + Vec3::Z };
        for _ in 0..30 {
            cam = update(&cam, target, Vec2::ZERO, &world, DT);
        }
        let clamped = cam;
        let raised = update(&clamped, target, Vec2::new(0.0, 0.05), &world, DT);
        assert_eq!(raised.pitch, clamped.pitch + 0.05, "input lands on the believed (clamped) pitch");
        assert!(
            raised.position.y > clamped.position.y + 0.01,
            "no dead zone: the eye must rise the same frame ({} -> {})",
            clamped.position.y,
            raised.position.y
        );
    }

    #[test]
    fn the_floor_writes_nothing_back_when_it_is_not_clamping() {
        // High over low terrain the clamp is idle, and the orbit stays exactly what the input
        // path computed: pitch is the precise sum, the arm the plain smooth-and-probe value.
        let world = flat_world(2.0);
        let target = Vec3::new(64.0, 10.0, 64.0);
        let cam = FollowCamera::spawn(target);
        let next = update(&cam, target, Vec2::new(0.0, 0.1), &world, DT);
        assert_eq!(next.pitch, cam.pitch + 0.1, "pitch must be untouched by the idle clamp");
        assert_eq!(next.arm, CAMERA_DISTANCE, "arm must be untouched by the idle clamp");
    }

    #[test]
    fn the_view_leads_ahead_of_the_anchor() {
        // The look-ahead framing: the aim point sits LOOK_AHEAD_M past the anchor along the
        // camera's horizontal forward, lifted by the trim, and the view really looks from the eye
        // through that point - the player frames low-centre, the view leads ahead.
        let target = Vec3::new(64.0, 5.0, 64.0);
        let cam = FollowCamera::spawn(target);
        let ahead = -boom_direction(cam.yaw, 0.0);
        let expected = cam.anchor + ahead * LOOK_AHEAD_M + Vec3::Y * LOOK_AHEAD_LIFT_M;
        assert!((cam.look_target() - expected).length() < 1e-6, "aim = {:?}", cam.look_target());

        // The view matrix maps the aim point onto the view axis: in view space the camera looks
        // down -Z, so the aim lands at x = 0, y = 0, z < 0.
        let view = Mat4::look_to_rh(
            cam.position,
            (cam.look_target() - cam.position).normalize(),
            Vec3::Y,
        );
        let in_view = view.transform_point3(cam.look_target());
        assert!(in_view.x.abs() < 1e-4 && in_view.y.abs() < 1e-4, "aim off the view axis: {in_view:?}");
        assert!(in_view.z < 0.0, "aim should sit in front of the camera: {in_view:?}");

        // And the anchor (the player, near enough) now sits below the view axis: low-centre.
        let anchor_in_view = view.transform_point3(cam.anchor);
        assert!(anchor_in_view.y < -0.1, "the player should frame below centre: {anchor_in_view:?}");
    }

    #[test]
    fn update_is_deterministic() {
        let world = flat_world(2.0);
        let target = Vec3::new(64.0, 4.0, 64.0);
        let cam = FollowCamera::spawn(target);
        let a = update(&cam, target, Vec2::new(0.01, -0.005), &world, DT);
        let b = update(&cam, target, Vec2::new(0.01, -0.005), &world, DT);
        assert_eq!(a, b);
    }
}
