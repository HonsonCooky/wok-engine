//! The editor's fly camera: pure update logic and matrix construction.
//!
//! Editor-owned per the brief: wok-physics's camera math (orbit, spring arm, terrain floor) models
//! a follow camera constrained by world geometry, and a free-flying editor camera has none of
//! those constraints, so building it from glam directly is the smaller idea. The camera is plain
//! state plus a pure step function: [`update`] takes the previous state, a frame's [`CameraInput`]
//! (already mapped from raw input by the caller), and `dt`, and returns the next state. Input
//! mapping (which keys, which mouse button) lives in `crate::app`; everything here is unit
//! testable with no window.
//!
//! Conventions: right-handed, `+Y` up. `yaw` is radians about `+Y` with `0` facing `-Z`, positive
//! turning right (toward `+X`); `pitch` is radians with positive looking up, clamped short of the
//! poles so the view never flips. The camera flies where it looks: forward motion follows the
//! pitched forward vector, not its horizontal projection.

use glam::{Mat4, Vec2, Vec3};

/// Hard pitch limit, just shy of straight up/down (about 88.9 degrees).
const PITCH_LIMIT: f32 = 1.55;

/// Movement speed bounds and the per-scroll-notch factor. One notch scales speed by 1.3, so the
/// range covers a slow inspect (1 m/s) to crossing a chunk in under a second (200 m/s) in about
/// 20 notches.
const SPEED_MIN: f32 = 1.0;
const SPEED_MAX: f32 = 200.0;
const SPEED_STEP_FACTOR: f32 = 1.3;

/// Vertical field of view and near plane for the projection. The far plane is per-frame data
/// (fog distance sets render distance, per the HLD), so it is a [`view_proj`] parameter.
const FOV_Y_RADIANS: f32 = std::f32::consts::FRAC_PI_3;
const NEAR_PLANE: f32 = 0.1;

/// The camera's whole state between frames.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlyCamera {
    pub position: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    /// Movement speed in metres per second, adjusted by scroll.
    pub speed: f32,
}

/// One frame's worth of camera-relevant input, already mapped from device input by the caller:
/// movement axes in `-1..=1`, the look delta in radians (zero when the user is not holding the
/// look button), and scroll notches for speed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CameraInput {
    /// Forward minus backward (W minus S).
    pub move_forward: f32,
    /// Right minus left (D minus A).
    pub move_right: f32,
    /// Up minus down (E minus Q).
    pub move_up: f32,
    /// Radians to add this frame: `x` to yaw, `y` to pitch.
    pub look_delta: Vec2,
    /// Scroll notches this frame; positive speeds up.
    pub speed_steps: f32,
}

impl FlyCamera {
    /// The unit vector the camera looks along.
    pub fn forward(&self) -> Vec3 {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        Vec3::new(sin_yaw * cos_pitch, sin_pitch, -cos_yaw * cos_pitch)
    }

    /// The unit vector to the camera's right, always horizontal (roll is never introduced).
    pub fn right(&self) -> Vec3 {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        Vec3::new(cos_yaw, 0.0, sin_yaw)
    }

    /// The combined view-projection matrix for a target with the given aspect ratio, with the far
    /// plane supplied per frame (the editor derives it from the fog distance). `perspective_rh`
    /// maps depth to `0..=1`, which is wgpu's clip-space convention.
    pub fn view_proj(&self, aspect: f32, far: f32) -> Mat4 {
        let projection = Mat4::perspective_rh(FOV_Y_RADIANS, aspect, NEAR_PLANE, far);
        let view = Mat4::look_to_rh(self.position, self.forward(), Vec3::Y);
        projection * view
    }
}

/// Margin factor on the framing distance: the framed bounds' enclosing sphere fills roughly 70%
/// of the vertical field of view instead of touching its edges.
const FRAME_MARGIN: f32 = 1.4;

/// Smallest radius framing treats as real, so a tiny or flat placement still gets a readable
/// view instead of the camera diving onto it.
const FRAME_MIN_RADIUS: f32 = 1.0;

/// The framing pitch band, radians: a gentle look down at the subject. Framing keeps the user's
/// yaw (their sense of direction survives the jump) but a level or upward pitch would frame the
/// subject against the sky edge-on, so pitch is clamped into this band.
const FRAME_PITCH_MIN: f32 = -0.9;
const FRAME_PITCH_MAX: f32 = -0.15;

/// Move the camera to a sensible view of an axis-aligned bounds (the double-click-to-frame jump):
/// keep the current yaw, clamp pitch into the gentle downward band, and back off along the
/// resulting forward until the bounds' enclosing sphere fits the vertical fov with margin. Pure:
/// camera and bounds in, camera out.
pub fn frame(camera: &FlyCamera, min: Vec3, max: Vec3) -> FlyCamera {
    let center = (min + max) * 0.5;
    let radius = ((max - min).length() * 0.5).max(FRAME_MIN_RADIUS);
    let aimed = FlyCamera {
        pitch: camera.pitch.clamp(FRAME_PITCH_MIN, FRAME_PITCH_MAX),
        ..*camera
    };
    // A sphere of `radius` subtends the full vertical fov at distance radius / sin(fov / 2);
    // the margin backs off further so the subject sits inside the frame, not against it.
    let distance = radius * FRAME_MARGIN / (FOV_Y_RADIANS * 0.5).sin();
    FlyCamera { position: center - aimed.forward() * distance, ..aimed }
}

/// Advance the camera by one frame. Pure: identical inputs give an identical next state.
///
/// Look is applied before movement so a frame's motion follows where the user just turned; speed
/// is applied before movement for the same reason. Movement direction is the combined wish vector
/// clamped to unit length, so diagonals are no faster than a single axis.
pub fn update(camera: &FlyCamera, input: &CameraInput, dt: f32) -> FlyCamera {
    let yaw = camera.yaw + input.look_delta.x;
    let pitch = (camera.pitch + input.look_delta.y).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    let speed = (camera.speed * SPEED_STEP_FACTOR.powf(input.speed_steps)).clamp(SPEED_MIN, SPEED_MAX);

    let turned = FlyCamera { yaw, pitch, speed, ..*camera };
    let wish = (turned.forward() * input.move_forward
        + turned.right() * input.move_right
        + Vec3::Y * input.move_up)
        .clamp_length_max(1.0);

    FlyCamera { position: camera.position + wish * speed * dt, ..turned }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-5;

    fn camera() -> FlyCamera {
        FlyCamera { position: Vec3::ZERO, yaw: 0.0, pitch: 0.0, speed: 10.0 }
    }

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < EPS
    }

    // ---- orientation ----

    #[test]
    fn yaw_zero_faces_negative_z() {
        assert!(close(camera().forward(), Vec3::NEG_Z));
        assert!(close(camera().right(), Vec3::X));
    }

    #[test]
    fn positive_yaw_turns_right_toward_positive_x() {
        let cam = update(
            &camera(),
            &CameraInput { look_delta: Vec2::new(std::f32::consts::FRAC_PI_2, 0.0), ..Default::default() },
            0.016,
        );
        assert!(close(cam.forward(), Vec3::X), "forward was {:?}", cam.forward());
    }

    #[test]
    fn positive_look_y_pitches_up() {
        let cam = update(
            &camera(),
            &CameraInput { look_delta: Vec2::new(0.0, 0.5), ..Default::default() },
            0.016,
        );
        assert!(cam.forward().y > 0.0);
    }

    #[test]
    fn pitch_clamps_short_of_the_poles() {
        let look_up = CameraInput { look_delta: Vec2::new(0.0, 10.0), ..Default::default() };
        let cam = update(&camera(), &look_up, 0.016);
        assert_eq!(cam.pitch, PITCH_LIMIT);
        let look_down = CameraInput { look_delta: Vec2::new(0.0, -20.0), ..Default::default() };
        let cam = update(&cam, &look_down, 0.016);
        assert_eq!(cam.pitch, -PITCH_LIMIT);
    }

    // ---- movement ----

    #[test]
    fn forward_motion_follows_the_view_direction() {
        let cam = FlyCamera { yaw: 1.1, pitch: 0.4, ..camera() };
        let moved = update(&cam, &CameraInput { move_forward: 1.0, ..Default::default() }, 0.5);
        assert!(close(moved.position, cam.forward() * cam.speed * 0.5));
    }

    #[test]
    fn strafe_and_vertical_motion_use_right_and_world_up() {
        let moved = update(&camera(), &CameraInput { move_right: 1.0, ..Default::default() }, 1.0);
        assert!(close(moved.position, Vec3::X * 10.0));
        let moved = update(&camera(), &CameraInput { move_up: -1.0, ..Default::default() }, 1.0);
        assert!(close(moved.position, Vec3::NEG_Y * 10.0));
    }

    #[test]
    fn diagonal_motion_is_no_faster_than_one_axis() {
        let input = CameraInput { move_forward: 1.0, move_right: 1.0, move_up: 1.0, ..Default::default() };
        let moved = update(&camera(), &input, 1.0);
        assert!(moved.position.length() <= 10.0 + EPS);
    }

    #[test]
    fn zero_dt_does_not_move() {
        let input = CameraInput { move_forward: 1.0, ..Default::default() };
        let moved = update(&camera(), &input, 0.0);
        assert_eq!(moved.position, Vec3::ZERO);
    }

    // ---- speed ----

    #[test]
    fn scroll_scales_speed_and_applies_to_the_same_frame() {
        let input = CameraInput { move_forward: 1.0, speed_steps: 1.0, ..Default::default() };
        let moved = update(&camera(), &input, 1.0);
        assert!((moved.speed - 13.0).abs() < 1e-3);
        assert!(close(moved.position, Vec3::NEG_Z * moved.speed));
    }

    #[test]
    fn speed_clamps_at_both_ends() {
        let fast = update(&camera(), &CameraInput { speed_steps: 100.0, ..Default::default() }, 0.016);
        assert_eq!(fast.speed, SPEED_MAX);
        let slow = update(&camera(), &CameraInput { speed_steps: -100.0, ..Default::default() }, 0.016);
        assert_eq!(slow.speed, SPEED_MIN);
    }

    // ---- framing ----

    #[test]
    fn framing_looks_straight_at_the_bounds_center() {
        let cam = FlyCamera { position: Vec3::new(50.0, 3.0, -20.0), yaw: 1.2, pitch: 0.4, speed: 16.0 };
        let (min, max) = (Vec3::new(10.0, 2.0, 10.0), Vec3::new(12.0, 4.0, 13.0));
        let framed = frame(&cam, min, max);
        let center = (min + max) * 0.5;
        let to_center = (center - framed.position).normalize();
        assert!(close(to_center, framed.forward()), "forward {:?} vs {to_center:?}", framed.forward());
    }

    #[test]
    fn framing_backs_off_far_enough_for_the_bounds_to_fit_the_fov() {
        let cam = FlyCamera { position: Vec3::ZERO, yaw: 0.3, pitch: -0.4, speed: 16.0 };
        let (min, max) = (Vec3::new(0.0, 0.0, 0.0), Vec3::new(8.0, 4.0, 6.0));
        let framed = frame(&cam, min, max);
        let center = (min + max) * 0.5;
        let radius = (max - min).length() * 0.5;
        let fits_at = radius / (FOV_Y_RADIANS * 0.5).sin();
        assert!((center - framed.position).length() >= fits_at, "the sphere must fit the vertical fov");
    }

    #[test]
    fn framing_keeps_yaw_and_clamps_pitch_into_the_downward_band() {
        let cam = FlyCamera { position: Vec3::ZERO, yaw: 2.1, pitch: 0.8, speed: 16.0 };
        let framed = frame(&cam, Vec3::ZERO, Vec3::ONE);
        assert_eq!(framed.yaw, 2.1, "the user's sense of direction survives the jump");
        assert_eq!(framed.pitch, FRAME_PITCH_MAX, "an upward pitch clamps to the gentle look-down");
        let steep = FlyCamera { pitch: -1.4, ..cam };
        assert_eq!(frame(&steep, Vec3::ZERO, Vec3::ONE).pitch, FRAME_PITCH_MIN);
    }

    #[test]
    fn framing_degenerate_bounds_still_gives_a_readable_distance() {
        // A point-sized bounds (a shapeless placement) frames from at least the minimum radius'
        // distance, never on top of the point.
        let cam = FlyCamera { position: Vec3::ZERO, yaw: 0.0, pitch: -0.3, speed: 16.0 };
        let at = Vec3::new(5.0, 2.0, 5.0);
        let framed = frame(&cam, at, at);
        let distance = (at - framed.position).length();
        assert!(distance >= FRAME_MIN_RADIUS, "distance {distance} too close for a readable view");
        assert!(framed.position.is_finite());
    }

    // ---- matrices ----

    #[test]
    fn view_proj_centers_what_the_camera_looks_at() {
        let cam = FlyCamera { position: Vec3::new(3.0, 4.0, 5.0), yaw: 0.7, pitch: -0.2, speed: 10.0 };
        let target = cam.position + cam.forward() * 50.0;
        let clip = cam.view_proj(16.0 / 9.0, 400.0).project_point3(target);
        assert!(clip.x.abs() < EPS && clip.y.abs() < EPS, "clip was {clip:?}");
        assert!(clip.z > 0.0 && clip.z < 1.0, "depth should be inside wgpu's 0..1 range: {}", clip.z);
    }
}
