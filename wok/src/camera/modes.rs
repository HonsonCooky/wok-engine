//! The editor's get-around god camera: the [`Camera`] the frame loop holds, the viewport drives, and the
//! renderer reads.
//!
//! `super` (`crate::camera`) holds the view-math primitive ([`FlyCamera`]); this module wraps one at a
//! spawn pose and adds the drive verbs the viewport feeds (`crate::viewport`): [`look`](Camera::look)
//! (right-drag mouse-look), [`fly`](Camera::fly) (WASD + raise/lower, dt-based), and
//! [`dolly`](Camera::dolly) (scroll). This is the get-around camera (the first rebuild bite); the rest of
//! the interaction grammar - selection-aware Move, the Frame verb, angle presets (designs/editor-design.md,
//! the Input section) - is still to come. Pure like the primitive: no egui, no input, no
//! window (the viewport reads the input and calls these).

use glam::{Mat4, Vec2, Vec3};

use super::FlyCamera;

/// Distance the spawn camera sits back from the scene focus, in metres - far enough to read an
/// object-placement working view, near enough to stay inside the scene fog. Camera feel is tunable.
const SPAWN_DISTANCE: f32 = 40.0;
/// Spawn pitch (radians): a gentle look-down over the scene (form and height read without being
/// top-down). Yaw spawns at `0` (facing world `-Z`, a map read).
const SPAWN_PITCH: f32 = -0.6;

/// Free-fly speed in metres per second (the get-around camera's base pace; a held boost multiplies it by
/// [`BOOST_MULT`]). Camera feel is tunable.
const FLY_SPEED: f32 = 16.0;
/// Multiplier on [`FLY_SPEED`] while the boost key (Shift) is held - the "cover ground fast" gear. Camera
/// feel is tunable.
const BOOST_MULT: f32 = 5.0;
/// Metres the camera dollies along its look per mouse-wheel notch. Camera feel is tunable.
const DOLLY_M: f32 = 2.0;

/// Mouse-look sensitivity, radians per pixel of raw motion (the tuned value from the prior free-fly
/// camera). The look reads raw `DeviceEvent::MouseMotion`, so the cursor lock does not change the feel.
const LOOK_SENSITIVITY: f32 = 0.0035;
/// Pitch clamp for the look, just shy of straight up/down (about 88.8 degrees), so the view never flips
/// through the pole where the look matrix degenerates.
const PITCH_LIMIT: f32 = 1.55;

/// The editor's camera: one [`FlyCamera`] the viewport flies and looks. The frame loop holds one
/// (frame-loop residency, not model state); the viewport drives it from the input each frame
/// (`crate::viewport`), the renderer reads its matrices, and the click-to-select casts its cursor ray.
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

    /// Turn the camera by a frame of raw mouse motion (pixels), at [`LOOK_SENSITIVITY`]: rightward motion
    /// yaws right (`+yaw`), downward motion pitches down (the view follows the mouse), with pitch clamped
    /// off the poles ([`PITCH_LIMIT`]) so the view never flips through vertical. The viewport feeds this
    /// only while a right-drag look is held and the cursor is locked, reading raw `DeviceEvent::MouseMotion`
    /// so the lock changes the feel of nothing.
    pub fn look(&mut self, motion: Vec2) {
        self.fly.yaw += motion.x * LOOK_SENSITIVITY;
        self.fly.pitch = (self.fly.pitch - motion.y * LOOK_SENSITIVITY).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    }

    /// Fly the camera for one frame at [`FLY_SPEED`] (times [`BOOST_MULT`] when `boost` is held), scaled
    /// by `dt` so the pace is metres per second regardless of frame rate. `fwd`/`strafe`/`lift` are the
    /// -1/0/+1 key intents: `fwd` steps along the look direction ([`FlyCamera::forward`], so W/S follow
    /// where the camera points, pitch included), `strafe` along the horizontal right ([`FlyCamera::right`],
    /// so A/D stay level), and `lift` along world `+Y` (E/Q, look-independent). The combined direction is
    /// normalized, so a diagonal is no faster than a cardinal; an all-zero intent is a no-op.
    pub fn fly(&mut self, fwd: f32, strafe: f32, lift: f32, boost: bool, dt: f32) {
        let direction = self.fly.forward() * fwd + self.fly.right() * strafe + Vec3::Y * lift;
        let speed = if boost { FLY_SPEED * BOOST_MULT } else { FLY_SPEED };
        self.fly.position += direction.normalize_or_zero() * speed * dt;
    }

    /// Dolly the camera along its look direction by `notches` mouse-wheel notches, [`DOLLY_M`] metres each
    /// (positive notches - wheel up - move forward, into the scene). The viewport feeds this from the
    /// scroll delta whenever the pointer is over the well.
    pub fn dolly(&mut self, notches: f32) {
        self.fly.position += self.fly.forward() * notches * DOLLY_M;
    }

    /// The perspective view-projection (far supplied per frame - the scene's render distance).
    pub fn view_proj(&self, aspect: f32, far: f32) -> Mat4 {
        self.fly.view_proj(aspect, far)
    }

    /// The eye position the renderer reads (fog distances from it).
    pub fn eye(&self) -> Vec3 {
        self.fly.position
    }

    /// The world-space cursor ray for picking ([`FlyCamera::cursor_ray`]). `far` is the same far the view
    /// projects with, so the unprojection inverts the exact matrix the renderer drew with. The viewport
    /// click-to-select (`crate::viewport`) casts this on a left press over the well.
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
        // the focus - the spawn-over-a-scene vantage the renderer draws from.
        let focus = Vec3::new(64.0, 2.0, 64.0);
        let cam = Camera::over(focus);
        assert!(((focus - cam.eye()).length() - SPAWN_DISTANCE).abs() < EPS, "eye sits SPAWN_DISTANCE out");
        let to_focus = (focus - cam.eye()).normalize();
        assert!((to_focus - cam.fly.forward()).length() < EPS, "forward points at the focus");
        assert!(cam.eye().y > focus.y, "the spawn eye floats above the focus (a downward look)");
    }

    // ---- look ----

    #[test]
    fn look_yaws_right_on_rightward_motion_and_pitches_down_on_downward_motion() {
        let mut cam = Camera::over(Vec3::ZERO);
        let (yaw0, pitch0) = (cam.fly.yaw, cam.fly.pitch);
        cam.look(Vec2::new(10.0, 0.0));
        assert!(cam.fly.yaw > yaw0, "rightward motion yaws right (+yaw)");
        cam.look(Vec2::new(0.0, 10.0));
        assert!(cam.fly.pitch < pitch0, "downward motion pitches down (the view follows the mouse)");
    }

    #[test]
    fn look_clamps_pitch_off_the_poles() {
        let mut cam = Camera::over(Vec3::ZERO);
        cam.look(Vec2::new(0.0, -100_000.0)); // hard up
        assert!((cam.fly.pitch - PITCH_LIMIT).abs() < EPS, "pitch clamps off the up pole: {}", cam.fly.pitch);
        cam.look(Vec2::new(0.0, 100_000.0)); // hard down
        assert!((cam.fly.pitch + PITCH_LIMIT).abs() < EPS, "pitch clamps off the down pole: {}", cam.fly.pitch);
    }

    // ---- fly (dt-based free flight) ----

    #[test]
    fn fly_forward_moves_along_the_look_and_strafe_along_horizontal_right() {
        // Forward flies along the look (pitch included); strafe flies along the horizontal right and stays
        // level. One second at FLY_SPEED covers FLY_SPEED metres.
        let mut cam = Camera::over(Vec3::ZERO);
        let forward = cam.fly.forward();
        let before = cam.eye();
        cam.fly(1.0, 0.0, 0.0, false, 1.0);
        let step = cam.eye() - before;
        assert!((step.normalize() - forward).length() < EPS, "forward intent flies along the look");
        assert!((step.length() - FLY_SPEED).abs() < EPS, "one second covers FLY_SPEED metres");

        let mut cam = Camera::over(Vec3::ZERO);
        let right = cam.fly.right();
        let before = cam.eye();
        cam.fly(0.0, 1.0, 0.0, false, 1.0);
        let step = cam.eye() - before;
        assert!((step.normalize() - right).length() < EPS, "strafe flies along the horizontal right");
        assert!(step.y.abs() < EPS, "the strafe stays level");
    }

    #[test]
    fn fly_lift_is_world_up_independent_of_the_look() {
        let mut cam = Camera::over(Vec3::ZERO);
        let before = cam.eye();
        cam.fly(0.0, 0.0, 1.0, false, 1.0);
        let step = cam.eye() - before;
        assert!((step.normalize() - Vec3::Y).length() < EPS, "lift flies straight up in world Y");
        assert!((step.length() - FLY_SPEED).abs() < EPS);
    }

    #[test]
    fn fly_scales_with_dt_and_boost_multiplies() {
        // Half the dt covers half the distance; the boost multiplies the speed by BOOST_MULT.
        let mut slow = Camera::over(Vec3::ZERO);
        let s0 = slow.eye();
        slow.fly(1.0, 0.0, 0.0, false, 0.5);
        assert!(((slow.eye() - s0).length() - FLY_SPEED * 0.5).abs() < EPS, "distance scales with dt");

        let mut boosted = Camera::over(Vec3::ZERO);
        let b0 = boosted.eye();
        boosted.fly(1.0, 0.0, 0.0, true, 0.5);
        assert!(((boosted.eye() - b0).length() - FLY_SPEED * BOOST_MULT * 0.5).abs() < EPS, "boost multiplies");
    }

    #[test]
    fn fly_normalizes_so_a_diagonal_is_not_faster() {
        // Forward + strafe together cover the same distance as either alone (the combined direction is
        // normalized), so a held diagonal is not faster than a cardinal. An all-zero intent is a no-op.
        let mut diag = Camera::over(Vec3::ZERO);
        let d0 = diag.eye();
        diag.fly(1.0, 1.0, 0.0, false, 1.0);
        assert!(((diag.eye() - d0).length() - FLY_SPEED).abs() < EPS, "a diagonal covers FLY_SPEED, not more");
        let at_rest = diag.eye();
        diag.fly(0.0, 0.0, 0.0, false, 1.0);
        assert_eq!(diag.eye(), at_rest, "an all-zero intent does not move the eye");
    }

    // ---- dolly ----

    #[test]
    fn dolly_moves_along_forward_by_notches_times_dolly_m() {
        let mut cam = Camera::over(Vec3::ZERO);
        let forward = cam.fly.forward();
        let before = cam.eye();
        cam.dolly(3.0);
        let step = cam.eye() - before;
        assert!((step - forward * 3.0 * DOLLY_M).length() < EPS, "dolly steps along forward by notches * DOLLY_M");
    }
}
