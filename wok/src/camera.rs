//! The editor's camera state and the pure matrix and framing math the renderer and picking read.
//!
//! [`FlyCamera`] is the one camera state the renderer reads: a position and a yaw/pitch orientation,
//! with [`forward`](FlyCamera::forward) / [`right`](FlyCamera::right) / [`up`](FlyCamera::up) the
//! orthonormal basis the view is built from. [`view_proj`](FlyCamera::view_proj) is the
//! view-projection the renderer draws with (far plane supplied per frame - the scene's render
//! distance), and [`cursor_ray`](FlyCamera::cursor_ray) inverts it to turn a viewport click into a
//! world ray for picking (sharp-edges 2: one shared cursor-to-ray source). [`look_at`] and [`frame`]
//! are the framing math - the angles that aim a camera at a target, and a vantage of an axis-aligned
//! bounds. Everything here is unit testable with no window.
//!
//! Pure math, no input: the interaction that drove the camera (the mouse-only look / dolly / pan) was
//! removed in the interaction demolition (designs/movement-camera-design.md, "What survives, what is
//! thrown out"), and the rebuilt camera grammar (brief 2: the Layout / Orbit cluster nav and the
//! framing ladder) is built on this same math from the frame loop. Nothing advances the camera yet, so
//! the editor renders a static view until then.
//!
//! Conventions: right-handed, `+Y` up. `yaw` is radians about `+Y` with `0` facing `-Z`, positive
//! turning right (toward `+X`); `pitch` is radians with positive looking up. `forward` follows the
//! yaw/pitch; `right` is always horizontal (no roll); `up` is their cross, equal to `+Y` when level.

use glam::{Mat4, Vec2, Vec3};

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

impl FlyCamera {
    /// The unit vector the camera looks along.
    pub fn forward(&self) -> Vec3 {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        Vec3::new(sin_yaw * cos_pitch, sin_pitch, -cos_yaw * cos_pitch)
    }

    /// The unit vector to the camera's right, always horizontal (roll is never introduced). Parked for
    /// the rebuilt camera (brief 2) like the framing math: only the tests exercise it until then.
    #[allow(dead_code)]
    pub fn right(&self) -> Vec3 {
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        Vec3::new(cos_yaw, 0.0, sin_yaw)
    }

    /// The unit vector for the camera's up across the view plane: perpendicular to forward and right,
    /// so it tilts with pitch (unlike world up). Equals `+Y` when the view is level. The basis
    /// (`right`, `up`, `-forward`) is orthonormal at any orientation. Parked for the rebuilt camera
    /// (brief 2) like [`right`](Self::right): only the tests exercise it until then.
    #[allow(dead_code)]
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

    /// The world-space ray for a cursor click in the viewport: the inverse of [`view_proj`], used by
    /// the 3D picking (3b). `pos_in_rect` is the click in egui points relative to the well's top-left,
    /// and `rect_size` is that same well's size - the rect the 3D rendered into, so the ray matches
    /// what the user clicked (sharp-edges 2: one shared cursor-to-ray source). `far` is the scene's far
    /// plane, the same one the render projects with, so the unprojection inverts the exact matrix.
    ///
    /// The click maps to normalized device coordinates (egui's y runs down, NDC's up, so y flips),
    /// then two clip points - on the near plane (`z = 0`) and the far plane (`z = 1`, the depth range
    /// `perspective_rh` maps to, wgpu's convention) - are unprojected through the inverse
    /// view-projection (`project_point3` is the divide by `w`). The ray runs from the eye along their
    /// difference, normalized so a pick `t` reads as world distance. The ndc is a ratio of point
    /// coordinates, so the result is identical whether measured in points or pixels.
    ///
    /// [`view_proj`]: Self::view_proj
    pub fn cursor_ray(&self, pos_in_rect: Vec2, rect_size: Vec2, far: f32) -> (Vec3, Vec3) {
        let ndc_x = 2.0 * pos_in_rect.x / rect_size.x - 1.0;
        let ndc_y = 1.0 - 2.0 * pos_in_rect.y / rect_size.y;
        let inv = self.view_proj(rect_size.x / rect_size.y, far).inverse();
        let near = inv.project_point3(Vec3::new(ndc_x, ndc_y, 0.0));
        let far_point = inv.project_point3(Vec3::new(ndc_x, ndc_y, 1.0));
        (self.position, (far_point - near).normalize())
    }
}

// ---- framing ----
// Exercised by the unit tests below and built on by the rebuilt camera (brief 2: the framing ladder
// and an explicit Go action); nothing calls them yet, so they are parked under `#[allow(dead_code)]`.

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

    // ---- orientation ----

    #[test]
    fn yaw_zero_faces_negative_z_with_a_level_basis() {
        let cam = camera();
        assert!(close(cam.forward(), Vec3::NEG_Z));
        assert!(close(cam.right(), Vec3::X));
        assert!(close(cam.up(), Vec3::Y));
    }

    #[test]
    fn the_basis_stays_orthonormal_when_pitched_and_yawed() {
        // up is unit length and perpendicular to forward and right at any orientation - the orthonormal
        // basis the view matrix and the rebuilt camera (brief 2) are built on.
        let cam = FlyCamera { yaw: 1.1, pitch: -0.7, ..camera() };
        assert!((cam.forward().length() - 1.0).abs() < EPS);
        assert!((cam.right().length() - 1.0).abs() < EPS);
        assert!((cam.up().length() - 1.0).abs() < EPS);
        assert!(cam.forward().dot(cam.right()).abs() < EPS);
        assert!(cam.forward().dot(cam.up()).abs() < EPS);
        assert!(cam.right().dot(cam.up()).abs() < EPS);
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

    // ---- cursor_ray ----

    #[test]
    fn cursor_ray_through_the_center_points_along_forward() {
        // A click at the centre of the well unprojects to the camera's central axis, so the ray runs
        // from the eye straight along forward.
        let cam = FlyCamera { position: Vec3::new(3.0, 4.0, 5.0), yaw: 0.7, pitch: -0.2 };
        let size = Vec2::new(800.0, 600.0);
        let (origin, dir) = cam.cursor_ray(size * 0.5, size, 400.0);
        assert!(close(origin, cam.position), "the ray starts at the eye");
        assert!(close(dir, cam.forward()), "a centred click looks along forward: {dir:?}");
    }

    #[test]
    fn cursor_ray_inverts_the_projection_for_an_off_center_pixel() {
        // A world point in front projects to some pixel; the ray cast back through that pixel must aim
        // straight at it. This pins the ndc mapping, including egui's y-down -> NDC y-up flip.
        let cam = FlyCamera { position: Vec3::new(1.0, 2.0, -3.0), yaw: 0.4, pitch: -0.3 };
        let size = Vec2::new(1024.0, 768.0);
        let far = 500.0;
        let target = cam.position + cam.forward() * 40.0 + cam.right() * 6.0 + cam.up() * 4.0;
        let clip = cam.view_proj(size.x / size.y, far).project_point3(target);
        // Invert cursor_ray's ndc mapping to recover the egui-point pixel (y-down) the target lands on.
        let pixel = Vec2::new((clip.x + 1.0) * 0.5 * size.x, (1.0 - clip.y) * 0.5 * size.y);
        let (origin, dir) = cam.cursor_ray(pixel, size, far);
        assert!(close(dir, (target - origin).normalize()), "the ray aims at the target: {dir:?}");
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
