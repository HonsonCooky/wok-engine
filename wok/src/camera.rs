//! The editor's camera state, the free-fly step, and framing - pure logic and matrix construction.
//!
//! [`FlyCamera`] is the one camera state the renderer reads. This module owns the free-fly step:
//! [`update`] takes the previous state, a frame's [`CameraInput`] (already mapped from raw input by
//! the caller), and `dt`, and returns the next state - a free flight with no world constraints, so
//! it is built from glam directly. Object mode is the resting mode and does not move the camera, so
//! it has no step here. [`look_at`] is the shared bridge - the angles that aim the camera from a
//! point at a target - used by framing. [`frame`] computes a vantage of an axis-aligned bounds and
//! is kept for a later explicit Go action; nothing calls it yet. Input mapping (which keys, which
//! mouse button) lives in `crate::input`; everything here is unit testable with no window.
//!
//! Conventions: right-handed, `+Y` up. `yaw` is radians about `+Y` with `0` facing `-Z`, positive
//! turning right (toward `+X`); `pitch` is radians with positive looking up, clamped short of the
//! poles so the view never flips. Free-fly is a ground-plane god-cam, not an FPS fly: forward and
//! strafe pan along the yaw-only horizontal heading (tilting the view never drags the camera up or
//! down), and Q/E change altitude along world up.

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

    /// The unit heading the camera pans forward along: the yaw direction on the XZ plane, pitch
    /// ignored, so a ground-plane pan stays level whatever the view tilt. Derived from yaw directly
    /// (it is `forward()` at pitch zero), not by flattening `forward`, which collapses to zero
    /// length when looking straight down.
    pub fn heading(&self) -> Vec3 {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        Vec3::new(sin_yaw, 0.0, -cos_yaw)
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

// ---- framing ----
// Lifted with the camera and exercised by the unit tests below; the frame-to-subject jump wires into
// an explicit Go action in a later brief, so these are unused in this frame-only build.

/// The `(yaw, pitch)` that aim a camera at `position` toward `target`, in [`FlyCamera`]'s
/// convention (yaw about `+Y`, `0` facing `-Z`; pitch positive looking up). The inverse of
/// [`FlyCamera::forward`]: feeding these back through `forward` points at the target. Degenerate
/// (`position == target`) yields a level forward rather than a NaN, and the `asin` domain is
/// guarded against float drift past `+/-1`.
#[allow(dead_code)]
pub fn look_at(position: Vec3, target: Vec3) -> (f32, f32) {
    let dir = (target - position).normalize_or_zero();
    (dir.x.atan2(-dir.z), dir.y.clamp(-1.0, 1.0).asin())
}

/// Margin factor on the framing distance: the framed bounds' enclosing sphere fills roughly 70%
/// of the vertical field of view instead of touching its edges.
#[allow(dead_code)]
const FRAME_MARGIN: f32 = 1.4;

/// Smallest radius framing treats as real, so a tiny or flat placement still gets a readable
/// view instead of the camera diving onto it.
#[allow(dead_code)]
const FRAME_MIN_RADIUS: f32 = 1.0;

/// The framing pitch band, radians: a gentle look down at the subject. Framing keeps the user's
/// yaw (their sense of direction survives the jump) but a level or upward pitch would frame the
/// subject against the sky edge-on, so pitch is clamped into this band.
#[allow(dead_code)]
const FRAME_PITCH_MIN: f32 = -0.9;
#[allow(dead_code)]
const FRAME_PITCH_MAX: f32 = -0.15;

/// Move the camera to a sensible view of an axis-aligned bounds (the frame-to-subject jump):
/// keep the current yaw, clamp pitch into the gentle downward band, and back off along the
/// resulting forward until the bounds' enclosing sphere fits the vertical fov with margin. Pure:
/// camera and bounds in, camera out. Kept for a later explicit Go action; nothing calls it yet.
#[allow(dead_code)]
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
/// is applied before movement for the same reason. Forward and strafe pan the horizontal plane (the
/// yaw-only heading and right), Q/E moves along world up, and the combined wish vector is clamped to
/// unit length so diagonals are no faster than a single axis.
pub fn update(camera: &FlyCamera, input: &CameraInput, dt: f32) -> FlyCamera {
    let yaw = camera.yaw + input.look_delta.x;
    let pitch = (camera.pitch + input.look_delta.y).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    let speed = (camera.speed * SPEED_STEP_FACTOR.powf(input.speed_steps)).clamp(SPEED_MIN, SPEED_MAX);

    let turned = FlyCamera { yaw, pitch, speed, ..*camera };
    let wish = (turned.heading() * input.move_forward
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
    fn forward_pan_stays_horizontal_while_q_e_alone_changes_height() {
        // Ground-plane god-cam: tilting the view down to survey must not drag the camera downward.
        // Forward motion has no Y component whatever the pitch; only Q/E changes height, on world up.
        let cam = FlyCamera { yaw: 1.1, pitch: -1.3, ..camera() };
        let panned = update(&cam, &CameraInput { move_forward: 1.0, ..Default::default() }, 0.5);
        assert!((panned.position.y - cam.position.y).abs() < EPS, "forward stayed level: {:?}", panned.position);

        let lifted = update(&cam, &CameraInput { move_up: 1.0, ..Default::default() }, 0.5);
        assert!(close(lifted.position, cam.position + Vec3::Y * cam.speed * 0.5), "Q/E raises along world up");
    }

    #[test]
    fn pan_tracks_yaw_and_ignores_pitch() {
        // The pan heading comes from yaw alone: a level camera and a steeply pitched one at the same
        // yaw move identically, and that shared heading is the yaw direction on the XZ plane. A
        // forward vector flattened (not derived from yaw) would shrink toward zero at this pitch.
        let level = FlyCamera { yaw: 1.1, pitch: 0.0, ..camera() };
        let pitched = FlyCamera { yaw: 1.1, pitch: -PITCH_LIMIT, ..camera() };
        let input = CameraInput { move_forward: 1.0, ..Default::default() };
        let a = update(&level, &input, 0.5);
        let b = update(&pitched, &input, 0.5);
        assert!(close(a.position, b.position), "pitch changed the pan: {:?} vs {:?}", a.position, b.position);

        let (sin_yaw, cos_yaw) = 1.1_f32.sin_cos();
        let heading = Vec3::new(sin_yaw, 0.0, -cos_yaw);
        assert!(close(a.position, heading * level.speed * 0.5), "forward follows the full-length yaw heading");
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

    // ---- look_at ----

    #[test]
    fn look_at_inverts_forward() {
        // The angles look_at returns, fed back through forward, point straight at the target: it is
        // the exact inverse framing relies on.
        let cam = FlyCamera { position: Vec3::new(2.0, 5.0, -3.0), yaw: 1.1, pitch: -0.4, speed: 16.0 };
        let target = cam.position + cam.forward() * 12.0;
        let (yaw, pitch) = look_at(cam.position, target);
        let aimed = FlyCamera { yaw, pitch, ..cam };
        assert!(close(aimed.forward(), cam.forward()), "forward {:?} vs {:?}", aimed.forward(), cam.forward());
    }

    #[test]
    fn look_at_is_graceful_when_position_equals_target() {
        let (yaw, pitch) = look_at(Vec3::splat(4.0), Vec3::splat(4.0));
        assert!(yaw.is_finite() && pitch.is_finite(), "degenerate aim must not be NaN: {yaw}, {pitch}");
    }
}
