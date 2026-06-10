//! The third-person follow camera: game-owned state sequencing wok-physics's camera math.
//!
//! wok-physics provides the pure pieces (orbit transform, spring arm, terrain floor, smoothing) and
//! deliberately holds nothing between calls; this module is the composition its docs sketch, with
//! taste owning the state (angles, eased arm length, eased position):
//!
//!     let dir     = boom_direction(yaw, pitch);                         orbit angles -> boom direction
//!     let want    = spring_arm(target, dir, distance, probe, statics);  clamp the boom against geometry
//!     arm         = smooth(arm, want, arm_half_life, dt);               ease the length
//!     let desired = boom_point(target, dir, arm);                       the clamped camera position
//!     let floored = terrain_floor(desired, terrain, margin);            keep it above the ground
//!     position    = smooth(position, floored, pos_half_life, dt);       ease the follow
//!
//! The spring arm sweeps against the same world-space AABBs the player collides with, so the camera
//! never clips into a prefab; terrain is a vertical floor clamp (wok-physics's terrain_floor design:
//! a swept terrain contact is explicitly out of the camera math's scope), applied in the local frame
//! of the chunk under the camera. The camera updates per rendered frame with the frame's dt, not per
//! fixed step: it is presentation, not simulation, and the smoothing helper is frame-rate
//! independent by construction, so the two rates may differ without changing where the camera
//! settles.

use glam::{Mat4, Vec2, Vec3};
use wok_physics::{boom_direction, boom_point, smooth, spring_arm, terrain_floor};

use crate::constants::{
    CAMERA_ARM_HALF_LIFE, CAMERA_DISTANCE, CAMERA_POS_HALF_LIFE, CAMERA_PROBE_RADIUS, CAMERA_TERRAIN_MARGIN,
    FOV_Y_RADIANS, NEAR_PLANE, PITCH_DEFAULT, PITCH_MAX, PITCH_MIN,
};
use crate::world::World;

/// The camera's whole state between frames: the orbit angles, the eased boom length, and the eased
/// world-space position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FollowCamera {
    pub yaw: f32,
    pub pitch: f32,
    pub arm: f32,
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
            arm: CAMERA_DISTANCE,
            position: boom_point(target, boom_direction(yaw, pitch), CAMERA_DISTANCE),
        }
    }

    /// The combined view-projection matrix, looking from the camera at `target`, with the far plane
    /// supplied per frame (fog distance sets render distance, per the HLD). `perspective_rh` maps
    /// depth to `0..=1`, wgpu's clip-space convention. When the boom has collapsed onto the target
    /// (spring arm at zero), the look direction degenerates; fall back to the boom's own axis so the
    /// matrix stays finite.
    pub fn view_proj(&self, target: Vec3, aspect: f32, far: f32) -> Mat4 {
        let forward = (target - self.position)
            .try_normalize()
            .unwrap_or_else(|| -boom_direction(self.yaw, self.pitch));
        let projection = Mat4::perspective_rh(FOV_Y_RADIANS, aspect, NEAR_PLANE, far);
        let view = Mat4::look_to_rh(self.position, forward, Vec3::Y);
        projection * view
    }
}

/// Advance the camera by one rendered frame: apply the look delta to the orbit, then run the
/// composition above against the world. Pure: identical inputs give an identical next state.
pub fn update(camera: &FollowCamera, target: Vec3, look_delta: Vec2, world: &World, dt: f32) -> FollowCamera {
    let yaw = camera.yaw + look_delta.x;
    let pitch = (camera.pitch + look_delta.y).clamp(PITCH_MIN, PITCH_MAX);

    let dir = boom_direction(yaw, pitch);
    let want = spring_arm(target, dir, CAMERA_DISTANCE, CAMERA_PROBE_RADIUS, &world.statics);
    let arm = smooth(camera.arm, want, CAMERA_ARM_HALF_LIFE, dt);

    let desired = boom_point(target, dir, arm);
    let floored = match world.terrain_under(desired.x, desired.z) {
        Some(t) => terrain_floor(desired - t.origin, &t.heightmap, CAMERA_TERRAIN_MARGIN) + t.origin,
        None => desired,
    };
    let position = smooth(camera.position, floored, CAMERA_POS_HALF_LIFE, dt);

    FollowCamera { yaw, pitch, arm, position }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use wok_scene::{Aabb, CHUNK_GRID_LEN, Heightmap, SurfaceTag};

    const DT: f32 = 1.0 / 60.0;

    fn flat_world(height_m: f32) -> World {
        let raw = Heightmap::meters_to_raw(height_m);
        let heightmap =
            Heightmap::new(vec![raw; CHUNK_GRID_LEN], vec![SurfaceTag::new("g")], vec![0; CHUNK_GRID_LEN]).unwrap();
        World { statics: vec![], terrains: vec![crate::world::ChunkTerrain { origin: Vec3::ZERO, heightmap }] }
    }

    #[test]
    fn spawn_settles_behind_the_target_at_full_boom() {
        let target = Vec3::new(64.0, 5.0, 64.0);
        let cam = FollowCamera::spawn(target);
        assert_eq!(cam.arm, CAMERA_DISTANCE);
        assert!(((cam.position - target).length() - CAMERA_DISTANCE).abs() < 1e-4);
        assert!(cam.position.z > target.z, "zero yaw puts the camera on the +Z side");
        assert!(cam.position.y > target.y, "the default pitch looks gently down");
    }

    #[test]
    fn pitch_clamps_to_its_range() {
        let world = flat_world(0.0);
        let target = Vec3::new(64.0, 10.0, 64.0);
        let cam = update(&FollowCamera::spawn(target), target, Vec2::new(0.0, 100.0), &world, DT);
        assert_eq!(cam.pitch, PITCH_MAX);
        let cam = update(&cam, target, Vec2::new(0.0, -100.0), &world, DT);
        assert_eq!(cam.pitch, PITCH_MIN);
    }

    #[test]
    fn a_wall_behind_the_player_pulls_the_boom_in() {
        // A wall a metre and a half behind the target on the +Z boom: settle long enough and the
        // eased arm converges to the obstructed length, well inside the full distance.
        let target = Vec3::new(64.0, 5.0, 64.0);
        let world = World {
            statics: vec![Aabb::new(Vec3::new(50.0, 0.0, 65.5), Vec3::new(80.0, 10.0, 67.0))],
            terrains: vec![],
        };
        let mut cam = FollowCamera { yaw: 0.0, pitch: 0.0, arm: CAMERA_DISTANCE, position: target + Vec3::Z };
        for _ in 0..240 {
            cam = update(&cam, target, Vec2::ZERO, &world, DT);
        }
        assert!(cam.arm < 1.5, "arm should converge inside the wall distance: {}", cam.arm);
        assert!(cam.position.z < 65.5, "camera should sit in front of the wall: {}", cam.position.z);
    }

    #[test]
    fn the_terrain_floor_keeps_a_low_camera_above_the_ground() {
        // Terrain at 8m, target low and pitch at the floor: the unfloored boom would dip under the
        // surface; the settled camera must hold the margin above it instead.
        let world = flat_world(8.0);
        let ground = world.terrains[0].heightmap.height_at(64.0, 70.0);
        let target = Vec3::new(64.0, 8.5, 64.0);
        let mut cam = FollowCamera { yaw: 0.0, pitch: PITCH_MIN, arm: CAMERA_DISTANCE, position: target + Vec3::Z };
        for _ in 0..240 {
            cam = update(&cam, target, Vec2::ZERO, &world, DT);
        }
        assert!(
            cam.position.y >= ground + CAMERA_TERRAIN_MARGIN - 1e-3,
            "camera y {} should hold the margin above the surface {}",
            cam.position.y,
            ground
        );
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
