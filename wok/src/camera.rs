//! The editor's camera state, the mouse-only navigation step, and framing - pure logic and matrix
//! construction.
//!
//! [`FlyCamera`] is the one camera state the renderer reads. The editor camera is mouse-only and
//! always live (designs/editor-design.md, Input): [`update`] takes the previous state and a frame's
//! [`CameraInput`] (already mapped from raw mouse input by the caller) and returns the next state -
//! look from a right-drag, dolly from scroll, pan from a middle-drag - a free navigation with no
//! world constraints, so it is built from glam directly. [`look_at`] is the shared bridge - the
//! angles that aim the camera from a point at a target - used by framing. [`frame`] computes a
//! vantage of an axis-aligned bounds and is kept for a later explicit Go action; nothing calls it
//! yet. Input mapping (which button, which sensitivity) lives in `crate::input`; everything here is
//! unit testable with no window.
//!
//! Conventions: right-handed, `+Y` up. `yaw` is radians about `+Y` with `0` facing `-Z`, positive
//! turning right (toward `+X`); `pitch` is radians with positive looking up, clamped short of the
//! poles so the view never flips. Look rotates the view in place; dolly slides along the full look
//! direction (a pitched view dollies into the ground or the sky, not level); pan slides along the
//! camera's right and up - the view plane, which tilts with pitch - so a drag stays on screen.

use glam::{Mat4, Vec2, Vec3};

/// Hard pitch limit, just shy of straight up/down (about 88.9 degrees).
const PITCH_LIMIT: f32 = 1.55;

/// Vertical field of view and near plane for the projection. The far plane is per-frame data (the
/// scene's render distance - its streaming extent, per the HLD), so it is a [`view_proj`] parameter.
const FOV_Y_RADIANS: f32 = std::f32::consts::FRAC_PI_3;
const NEAR_PLANE: f32 = 0.1;

/// The camera's whole state between frames.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlyCamera {
    pub position: Vec3,
    pub yaw: f32,
    pub pitch: f32,
}

/// One frame's worth of camera-relevant input, already mapped from device input by the caller: the
/// look delta in radians (zero when the right button is not held), the dolly distance in metres
/// (from scroll), and the pan offset in metres (zero when the middle button is not held). All are
/// per-frame deltas, so the step takes no `dt`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CameraInput {
    /// Radians to add this frame: `x` to yaw, `y` to pitch. Zero unless the look button is held.
    pub look_delta: Vec2,
    /// Metres to dolly along the look direction this frame (from scroll); positive moves forward.
    pub dolly: f32,
    /// Metres to pan this frame: `x` along the camera's right, `y` along its up. Already signed by
    /// the caller so the scene tracks the drag. Zero unless the pan button is held.
    pub pan: Vec2,
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

    /// The unit vector for the camera's up across the view plane: perpendicular to forward and right,
    /// so it tilts with pitch (unlike world up) and a pan slides along the screen's vertical. Equals
    /// `+Y` when the view is level. The basis (`right`, `up`, `-forward`) is orthonormal at any
    /// orientation, so a pan never skews or scales.
    pub fn up(&self) -> Vec3 {
        self.right().cross(self.forward())
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
// an explicit Go action in a later brief, so these are unused in this navigate-only build.

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
/// Look is applied first, so a frame that both turns and translates moves along where the user just
/// turned. Dolly then slides along the post-look forward (the full look direction, pitch included),
/// and pan slides along the post-look right and up (the view plane). The inputs are already per-frame
/// deltas in metres and radians, so there is no `dt` and no speed.
pub fn update(camera: &FlyCamera, input: &CameraInput) -> FlyCamera {
    let yaw = camera.yaw + input.look_delta.x;
    let pitch = (camera.pitch + input.look_delta.y).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    let turned = FlyCamera { yaw, pitch, ..*camera };

    let position = turned.position
        + turned.forward() * input.dolly
        + turned.right() * input.pan.x
        + turned.up() * input.pan.y;
    FlyCamera { position, ..turned }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-5;

    fn camera() -> FlyCamera {
        FlyCamera { position: Vec3::ZERO, yaw: 0.0, pitch: 0.0 }
    }

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < EPS
    }

    /// A look-only input - the common case in these tests, where only the right-drag delta matters.
    fn look(delta: Vec2) -> CameraInput {
        CameraInput { look_delta: delta, ..Default::default() }
    }

    // ---- orientation ----

    #[test]
    fn yaw_zero_faces_negative_z_with_a_level_basis() {
        let cam = camera();
        assert!(close(cam.forward(), Vec3::NEG_Z));
        assert!(close(cam.right(), Vec3::X));
        assert!(close(cam.up(), Vec3::Y));
    }

    #[test]
    fn positive_yaw_turns_right_toward_positive_x() {
        let cam = update(&camera(), &look(Vec2::new(std::f32::consts::FRAC_PI_2, 0.0)));
        assert!(close(cam.forward(), Vec3::X), "forward was {:?}", cam.forward());
    }

    #[test]
    fn positive_look_y_pitches_up() {
        let cam = update(&camera(), &look(Vec2::new(0.0, 0.5)));
        assert!(cam.forward().y > 0.0);
    }

    #[test]
    fn pitch_clamps_short_of_the_poles() {
        let cam = update(&camera(), &look(Vec2::new(0.0, 10.0)));
        assert_eq!(cam.pitch, PITCH_LIMIT);
        let cam = update(&cam, &look(Vec2::new(0.0, -20.0)));
        assert_eq!(cam.pitch, -PITCH_LIMIT);
    }

    #[test]
    fn the_basis_stays_orthonormal_when_pitched_and_yawed() {
        // up is unit length and perpendicular to forward and right at any orientation, so a pan
        // moves the camera without skew or scale.
        let cam = FlyCamera { yaw: 1.1, pitch: -0.7, ..camera() };
        assert!((cam.forward().length() - 1.0).abs() < EPS);
        assert!((cam.right().length() - 1.0).abs() < EPS);
        assert!((cam.up().length() - 1.0).abs() < EPS);
        assert!(cam.forward().dot(cam.right()).abs() < EPS);
        assert!(cam.forward().dot(cam.up()).abs() < EPS);
        assert!(cam.right().dot(cam.up()).abs() < EPS);
    }

    // ---- dolly ----

    #[test]
    fn scroll_dollies_along_the_look_direction() {
        // A level camera dollies straight along -Z; the dolly magnitude is the input distance.
        let moved = update(&camera(), &CameraInput { dolly: 5.0, ..Default::default() });
        assert!(close(moved.position, Vec3::NEG_Z * 5.0), "dollied to {:?}", moved.position);
    }

    #[test]
    fn dolly_follows_pitch_into_the_view() {
        // Dolly is along the full look direction, not the horizontal: a downward-pitched view dollies
        // downward (negative Y), so scrolling drives into whatever the camera is looking at.
        let cam = FlyCamera { pitch: -0.6, ..camera() };
        let moved = update(&cam, &CameraInput { dolly: 3.0, ..Default::default() });
        assert!(close(moved.position, cam.forward() * 3.0), "dolly should track forward");
        assert!(moved.position.y < 0.0, "a downward look dollies downward");
    }

    // ---- pan ----

    #[test]
    fn pan_x_slides_along_right_and_pan_y_along_up() {
        // pan.x moves along the camera's right, pan.y along its up. The input carries the sign that
        // makes the scene track the drag (set in crate::input), so here the axes are at face value.
        let cam = camera();
        let moved = update(&cam, &CameraInput { pan: Vec2::new(2.0, 0.0), ..Default::default() });
        assert!(close(moved.position, cam.right() * 2.0));
        let moved = update(&cam, &CameraInput { pan: Vec2::new(0.0, 2.0), ..Default::default() });
        assert!(close(moved.position, cam.up() * 2.0));
    }

    #[test]
    fn pan_uses_the_tilted_view_plane_not_world_axes() {
        // Pan-up on a pitched view moves along the camera's up (which tilts with pitch), not world up,
        // so the drag stays in the screen plane and gains a horizontal lean.
        let cam = FlyCamera { pitch: -0.6, ..camera() };
        let moved = update(&cam, &CameraInput { pan: Vec2::new(0.0, 1.0), ..Default::default() });
        assert!(close(moved.position, cam.up()));
        assert!(moved.position.z < 0.0, "a downward-tilted up leans forward, so pan-up gains -Z");
    }

    // ---- composition ----

    #[test]
    fn look_is_applied_before_the_move() {
        // A single frame that turns 90 degrees right and dollies must dolly along the new forward
        // (+X), not the pre-turn one (-Z): look first, then move.
        let input = CameraInput {
            look_delta: Vec2::new(std::f32::consts::FRAC_PI_2, 0.0),
            dolly: 4.0,
            ..Default::default()
        };
        let moved = update(&camera(), &input);
        assert!(close(moved.position, Vec3::X * 4.0), "dolly used the post-look forward: {:?}", moved.position);
    }

    #[test]
    fn no_input_holds_the_camera_still() {
        assert_eq!(update(&camera(), &CameraInput::default()), camera());
    }

    // ---- framing ----

    #[test]
    fn framing_looks_straight_at_the_bounds_center() {
        let cam = FlyCamera { position: Vec3::new(50.0, 3.0, -20.0), yaw: 1.2, pitch: 0.4 };
        let (min, max) = (Vec3::new(10.0, 2.0, 10.0), Vec3::new(12.0, 4.0, 13.0));
        let framed = frame(&cam, min, max);
        let center = (min + max) * 0.5;
        let to_center = (center - framed.position).normalize();
        assert!(close(to_center, framed.forward()), "forward {:?} vs {to_center:?}", framed.forward());
    }

    #[test]
    fn framing_backs_off_far_enough_for_the_bounds_to_fit_the_fov() {
        let cam = FlyCamera { position: Vec3::ZERO, yaw: 0.3, pitch: -0.4 };
        let (min, max) = (Vec3::new(0.0, 0.0, 0.0), Vec3::new(8.0, 4.0, 6.0));
        let framed = frame(&cam, min, max);
        let center = (min + max) * 0.5;
        let radius = (max - min).length() * 0.5;
        let fits_at = radius / (FOV_Y_RADIANS * 0.5).sin();
        assert!((center - framed.position).length() >= fits_at, "the sphere must fit the vertical fov");
    }

    #[test]
    fn framing_keeps_yaw_and_clamps_pitch_into_the_downward_band() {
        let cam = FlyCamera { position: Vec3::ZERO, yaw: 2.1, pitch: 0.8 };
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
        let cam = FlyCamera { position: Vec3::ZERO, yaw: 0.0, pitch: -0.3 };
        let at = Vec3::new(5.0, 2.0, 5.0);
        let framed = frame(&cam, at, at);
        let distance = (at - framed.position).length();
        assert!(distance >= FRAME_MIN_RADIUS, "distance {distance} too close for a readable view");
        assert!(framed.position.is_finite());
    }

    // ---- matrices ----

    #[test]
    fn view_proj_centers_what_the_camera_looks_at() {
        let cam = FlyCamera { position: Vec3::new(3.0, 4.0, 5.0), yaw: 0.7, pitch: -0.2 };
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
        let cam = FlyCamera { position: Vec3::new(2.0, 5.0, -3.0), yaw: 1.1, pitch: -0.4 };
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
