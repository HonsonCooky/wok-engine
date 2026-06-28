//! The editor's static camera math: the pure matrix and cursor-ray primitives the renderer and the
//! (parked) picking read.
//!
//! The interaction layer that flew the camera was demolished to a render-only baseline (the editor
//! interaction is being rebuilt incrementally, one workflow at a time - designs/orchestrator-state.md;
//! the detailed grammar in designs/movement-camera-design.md is ON HOLD). What survives here is the
//! minimum the render path and picking need: [`FlyCamera`], a perspective camera pose
//! ([`forward`](FlyCamera::forward) / [`view_proj`](FlyCamera::view_proj) /
//! [`cursor_ray`](FlyCamera::cursor_ray)), wrapped by the static [`Camera`] in [`modes`] (re-exported as
//! [`Camera`]). Nothing drives it; it sits at a fixed spawn vantage over the open scene. The
//! get-around-camera workflow (the first rebuild bite) re-adds the fly basis and the input that moves it.
//!
//! Everything here is pure - no egui, no input, no window - and unit tested below.
//!
//! Conventions: right-handed, `+Y` up. `yaw` is radians about `+Y` with `0` facing `-Z`, positive
//! turning right (toward `+X`); `pitch` is radians with positive looking up. `forward` follows the
//! yaw/pitch.

use glam::{Mat4, Vec2, Vec3};

/// The static editor camera (one [`FlyCamera`] at a fixed spawn pose).
mod modes;
pub use modes::Camera;

/// Vertical field of view and near plane for the projection. The far plane is per-frame data (the
/// scene's render distance - its streaming extent, per the HLD), so it is a [`view_proj`](FlyCamera::view_proj)
/// parameter.
const FOV_Y_RADIANS: f32 = std::f32::consts::FRAC_PI_3;
const NEAR_PLANE: f32 = 0.1;

/// A perspective camera pose - the basis the static [`Camera`] ([`modes`]) holds. The renderer reads its
/// [`view_proj`](Self::view_proj); the parked picking casts its [`cursor_ray`](Self::cursor_ray).
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

    /// The combined view-projection matrix for a target with the given aspect ratio, with the far
    /// plane supplied per frame (the editor derives it from the scene's render distance).
    /// `perspective_rh` maps depth to `0..=1`, which is wgpu's clip-space convention.
    pub fn view_proj(&self, aspect: f32, far: f32) -> Mat4 {
        let projection = Mat4::perspective_rh(FOV_Y_RADIANS, aspect, NEAR_PLANE, far);
        let view = Mat4::look_to_rh(self.position, self.forward(), Vec3::Y);
        projection * view
    }

    /// The world-space ray for a cursor click in the viewport: the inverse of [`view_proj`].
    /// `pos_in_rect` is the click in egui points relative to the well's top-left, and `rect_size` is that
    /// same well's size - the rect the 3D rendered into, so the ray matches what the user clicked
    /// (sharp-edges 2: one shared cursor-to-ray source). `far` is the scene's far plane, the same one the
    /// render projects with, so the unprojection inverts the exact matrix.
    ///
    /// The click maps to normalized device coordinates (egui's y runs down, NDC's up, so y flips),
    /// then two clip points - on the near plane (`z = 0`) and the far plane (`z = 1`, the depth range
    /// `perspective_rh` maps to, wgpu's convention) - are unprojected through the inverse
    /// view-projection (`project_point3` is the divide by `w`). The ray runs from the eye along their
    /// difference, normalized so a pick `t` reads as world distance. The ndc is a ratio of point
    /// coordinates, so the result is identical whether measured in points or pixels.
    ///
    /// Parked with the picking (the render-only baseline drives no picking yet): the renderer reads
    /// [`view_proj`], and the click-to-select workflow (a rebuild bite) reads this.
    ///
    /// [`view_proj`]: Self::view_proj
    #[allow(dead_code)]
    pub fn cursor_ray(&self, pos_in_rect: Vec2, rect_size: Vec2, far: f32) -> (Vec3, Vec3) {
        let ndc_x = 2.0 * pos_in_rect.x / rect_size.x - 1.0;
        let ndc_y = 1.0 - 2.0 * pos_in_rect.y / rect_size.y;
        let inv = self.view_proj(rect_size.x / rect_size.y, far).inverse();
        let near = inv.project_point3(Vec3::new(ndc_x, ndc_y, 0.0));
        let far_point = inv.project_point3(Vec3::new(ndc_x, ndc_y, 1.0));
        (self.position, (far_point - near).normalize())
    }
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
    fn yaw_zero_faces_negative_z() {
        // The render-only baseline keeps the forward basis the view matrix and the parked cursor ray are
        // built on; at yaw 0 / pitch 0 the camera looks straight down world -Z.
        assert!(close(camera().forward(), Vec3::NEG_Z));
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

    // ---- cursor_ray (parked, but still correct) ----

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
        let target = cam.position + cam.forward() * 40.0 + Vec3::new(6.0, 4.0, 2.0);
        let clip = cam.view_proj(size.x / size.y, far).project_point3(target);
        // Invert cursor_ray's ndc mapping to recover the egui-point pixel (y-down) the target lands on.
        let pixel = Vec2::new((clip.x + 1.0) * 0.5 * size.x, (1.0 - clip.y) * 0.5 * size.y);
        let (origin, dir) = cam.cursor_ray(pixel, size, far);
        assert!(close(dir, (target - origin).normalize()), "the ray aims at the target: {dir:?}");
    }
}
