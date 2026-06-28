//! The editor's static god camera: the [`Camera`] the frame loop holds and the renderer reads.
//!
//! `super` (`crate::camera`) holds the view-math primitive ([`FlyCamera`]); this module wraps one at a
//! fixed spawn pose. After the interaction demolition (the render-only baseline) nothing drives it - it
//! sits over the open scene at the spawn vantage so the renderer has a viewpoint. The get-around-camera
//! workflow (the first rebuild bite) re-adds the fly / look input that moves it; the rest of the
//! movement-camera grammar (designs/movement-camera-design.md) is ON HOLD. Pure like the primitive: no
//! egui, no input, no window.

use glam::{Mat4, Vec2, Vec3};

use super::FlyCamera;

/// Distance the spawn camera sits back from the scene focus, in metres - far enough to read an
/// object-placement working view, near enough to stay inside the scene fog. Camera feel is tunable.
const SPAWN_DISTANCE: f32 = 40.0;
/// Spawn pitch (radians): a gentle look-down over the scene (form and height read without being
/// top-down). Yaw spawns at `0` (facing world `-Z`, a map read).
const SPAWN_PITCH: f32 = -0.6;

/// The editor's camera: one [`FlyCamera`] at a fixed spawn pose. The frame loop holds one (frame-loop
/// residency, not model state); the renderer reads its matrices, and the parked picking casts its cursor
/// ray. Nothing moves it in the render-only baseline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// The live perspective camera the renderer and picking read.
    fly: FlyCamera,
}

impl Camera {
    /// A camera looking at `focus` from the spawn vantage - back along a gentle downward look at the
    /// default distance, the spawn-over-a-scene and pre-scene default.
    /// [`RenderScene::spawn_camera`](crate::render_scene::RenderScene::spawn_camera) builds this over a
    /// freshly loaded scene.
    pub fn over(focus: Vec3) -> Camera {
        let oriented = FlyCamera { position: Vec3::ZERO, yaw: 0.0, pitch: SPAWN_PITCH };
        let position = focus - oriented.forward() * SPAWN_DISTANCE;
        Camera { fly: FlyCamera { position, ..oriented } }
    }

    /// The perspective view-projection (far supplied per frame - the scene's render distance).
    pub fn view_proj(&self, aspect: f32, far: f32) -> Mat4 {
        self.fly.view_proj(aspect, far)
    }

    /// The eye position the renderer reads (fog distances from it).
    pub fn eye(&self) -> Vec3 {
        self.fly.position
    }

    /// The world-space cursor ray for picking ([`FlyCamera::cursor_ray`]). `far` is the same far the
    /// view projects with. Parked with the picking: the render-only baseline casts no ray yet, and the
    /// click-to-select workflow (a rebuild bite) reads this.
    #[allow(dead_code)]
    pub fn cursor_ray(&self, pos_in_rect: Vec2, rect_size: Vec2, far: f32) -> (Vec3, Vec3) {
        self.fly.cursor_ray(pos_in_rect, rect_size, far)
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-4;

    #[test]
    fn spawn_sits_back_from_the_focus_and_looks_at_it() {
        // over(focus) places the eye SPAWN_DISTANCE out along a gentle downward look and aims straight at
        // the focus - the static spawn-over-a-scene vantage the renderer draws from.
        let focus = Vec3::new(64.0, 2.0, 64.0);
        let cam = Camera::over(focus);
        assert!(((focus - cam.eye()).length() - SPAWN_DISTANCE).abs() < EPS, "eye sits SPAWN_DISTANCE out");
        let to_focus = (focus - cam.eye()).normalize();
        assert!((to_focus - cam.fly.forward()).length() < EPS, "forward points at the focus");
        assert!(cam.eye().y > focus.y, "the spawn eye floats above the focus (a downward look)");
    }
}
