//! The editor's one free-fly god camera: the [`Camera`] the frame loop, the interaction layer, and the
//! renderer hold, and its eased Frame transition.
//!
//! `super` (`crate::camera`) holds the view-math primitives - the perspective [`FlyCamera`] basis, the
//! parked orthographic [`LayoutCamera`], and the [`frame`](crate::camera::frame) helper. This module is
//! the layer above them: the single free-roaming camera of the keyboard-first model
//! (designs/movement-camera-design.md "Camera", the 2026-06-29 revision). [`Camera`] wraps one
//! [`FlyCamera`] and is the whole editor camera - there are no modes to cycle. It flies the camera (the
//! Look target's cluster forward/back along the ground facing and strafe left/right, a right-drag mouse
//! look, and the vertical pair in world Y), dispatches the matrices and the cursor ray the renderer and
//! picking read, and eases to a [`frame`](crate::camera::frame) pose when the Frame verb asks. Pure like
//! the primitives: no egui, no input, no window.
//!
//! Framing is explicit, never automatic (the doc's fix for the auto-frame zoom-jump): a selection change
//! moves nothing on its own. The Frame verb ([`frame_to`](Camera::frame_to)) centres the camera on a
//! bounds, keeping the current view direction and easing the live pose to the fit over a few frames
//! ([`advance`](Camera::advance)); any direct fly or look ([`fly`](Camera::fly) / [`look`](Camera::look))
//! cancels an ease in flight, so a manual nav is immediate and never fights the glide.
//!
//! Angle presets (canonical top-down / elevation / oblique vantages) and a Walk mode are R2 and a later
//! tier; the parked [`LayoutCamera`] and [`frame`](crate::camera::frame) are the pieces those reuse.

use glam::{Mat4, Vec2, Vec3};
use wok_scene::Aabb;

use super::{FlyCamera, frame};

/// Distance the spawn camera sits back from the scene focus, in metres - far enough to read an
/// object-placement working view, near enough to stay inside the scene fog. Camera feel is tunable.
const SPAWN_DISTANCE: f32 = 40.0;
/// Spawn pitch (radians): a gentle look-down over the scene, the same vantage the first cut spawned at
/// (form and height read without being top-down). Yaw spawns at `0` (facing world `-Z`, a map read).
const SPAWN_PITCH: f32 = -0.6;

/// Metres the free-fly cluster steps the camera per input. A tap nudges once; a hold repeats at the OS
/// key-repeat rate (so crossing a ~128m chunk is a short hold). Tunable; camera feel is the parked tweak.
const FLY_STEP_M: f32 = 2.0;

/// Mouse-look sensitivity, radians per pixel of raw motion (the proven value from the first cut's
/// mouse-only camera). The right-drag look reads raw `DeviceEvent::MouseMotion`, so the cursor lock does
/// not change the feel.
const LOOK_SENSITIVITY: f32 = 0.0035;

/// Pitch clamp for the look, just shy of straight up/down (about 88.8 degrees), so the view never flips
/// through the pole where the look matrix degenerates.
const PITCH_LIMIT: f32 = 1.55;

/// Fraction of the remaining gap the Frame ease closes each frame - an exponential glide that eases in
/// hard and settles soft. At ~0.25 it reads as a smooth few-frame transition at the editor's vsync rate.
const FRAME_EASE: f32 = 0.25;
/// How close (metres / radians) the live pose must sit to the Frame target before it snaps the rest and
/// ends the ease, so the glide terminates cleanly rather than creeping forever.
const FRAME_EASE_EPS: f32 = 0.01;

/// The editor's camera: one free-fly [`FlyCamera`] the frame loop flies, plus an optional in-progress
/// Frame ease. The frame loop holds one of these (frame-loop residency, not model state, like the rest
/// of the camera); the interaction layer flies it (the Look target's cluster, the right-drag look, the
/// vertical pair) and asks it to frame the selection, the renderer reads its matrices, and picking casts
/// its cursor ray.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// The live perspective camera the renderer and picking read.
    fly: FlyCamera,
    /// An in-progress Frame ease: the pose [`advance`](Self::advance) glides the live camera toward, or
    /// `None` at rest. A direct fly or look clears it (a manual nav cancels the glide).
    transition: Option<FlyCamera>,
}

impl Camera {
    /// A free-fly camera looking at `focus` from the spawn vantage - back along a gentle downward look
    /// at the default distance, the spawn-over-a-scene and pre-scene default.
    /// [`RenderScene::spawn_camera`](crate::render_scene::RenderScene::spawn_camera) builds this over a
    /// freshly loaded scene.
    pub fn over(focus: Vec3) -> Camera {
        let oriented = FlyCamera { position: Vec3::ZERO, yaw: 0.0, pitch: SPAWN_PITCH };
        let position = focus - oriented.forward() * SPAWN_DISTANCE;
        Camera { fly: FlyCamera { position, ..oriented }, transition: None }
    }

    /// Fly the camera by `(dx, dz)` cluster cells and `dy` of the vertical pair, one [`FLY_STEP_M`] step
    /// per unit (a held key repeats at the OS rate). The cluster is camera-relative on the ground plane:
    /// forward/back (`dz`, `-1` is forward) steps along the camera's ground facing, strafe (`dx`, `+1` is
    /// right) along its right - so D moves toward `+X` at yaw 0 (screen-right) and A toward `-X`. The
    /// vertical pair (`dy`, `+1` raises) is world `+Y`, camera-independent. A direct fly cancels any Frame
    /// ease (a manual nav is immediate).
    pub fn fly(&mut self, dx: i32, dz: i32, dy: i32) {
        let forward = self.fly.ground_forward();
        let right = self.fly.right();
        let step = (forward * -(dz as f32) + right * dx as f32 + Vec3::Y * dy as f32) * FLY_STEP_M;
        self.fly.position += step;
        self.transition = None;
    }

    /// Turn the camera by a frame of raw mouse motion (pixels): rightward motion yaws right (`+yaw`),
    /// downward motion pitches down (the view follows the mouse), pitch clamped off the poles. The
    /// interaction layer feeds this only while a right-drag look is held (the cursor is locked then). A
    /// direct look cancels any Frame ease.
    pub fn look(&mut self, motion: Vec2) {
        self.fly.yaw += motion.x * LOOK_SENSITIVITY;
        self.fly.pitch = (self.fly.pitch - motion.y * LOOK_SENSITIVITY).clamp(-PITCH_LIMIT, PITCH_LIMIT);
        self.transition = None;
    }

    /// Frame an axis-aligned bounds - the explicit Frame verb (`f`). Centres the camera on the bounds,
    /// keeping the current view direction (yaw) and the [`frame`](crate::camera::frame) pose's gentle
    /// downward pitch, placed back along it at a fit distance, and eases the live pose there over a few
    /// frames ([`advance`](Self::advance)) rather than cutting. Selection never calls this on its own; it
    /// is a deliberate verb, so jumping between selections never yanks the view or changes the zoom.
    pub fn frame_to(&mut self, bounds: Aabb) {
        self.transition = Some(frame(&self.fly, bounds.min, bounds.max));
    }

    /// Advance an in-progress Frame ease one frame: glide the live pose a fixed fraction
    /// ([`FRAME_EASE`]) of the way to the target, snapping the rest and ending the ease once within
    /// [`FRAME_EASE_EPS`]. A no-op at rest. Called once per frame by the interaction seam, before the
    /// draw, so the eased view shows the same frame. Yaw needs no wrap handling: [`frame`] keeps the
    /// current yaw, so the target yaw equals the live yaw and the term is a no-op.
    pub fn advance(&mut self) {
        let Some(target) = self.transition else { return };
        let close = (target.position - self.fly.position).length() < FRAME_EASE_EPS
            && (target.yaw - self.fly.yaw).abs() < FRAME_EASE_EPS
            && (target.pitch - self.fly.pitch).abs() < FRAME_EASE_EPS;
        if close {
            self.fly = target;
            self.transition = None;
            return;
        }
        self.fly = FlyCamera {
            position: self.fly.position.lerp(target.position, FRAME_EASE),
            yaw: self.fly.yaw + (target.yaw - self.fly.yaw) * FRAME_EASE,
            pitch: self.fly.pitch + (target.pitch - self.fly.pitch) * FRAME_EASE,
        };
    }

    /// The camera forward projected to the ground plane, as `(x, z)`. Its sign and dominant axis (not its
    /// length) are what the camera-relative Move reads to pick the nearest grid axis, so it rides the
    /// yaw-only ground forward (`FlyCamera::ground_forward`), a unit horizontal vector at any pitch.
    pub fn ground_forward(&self) -> Vec2 {
        let forward = self.fly.ground_forward();
        Vec2::new(forward.x, forward.z)
    }

    /// The perspective view-projection (far supplied per frame - the scene's render distance).
    pub fn view_proj(&self, aspect: f32, far: f32) -> Mat4 {
        self.fly.view_proj(aspect, far)
    }

    /// The eye position the renderer reads (fog distances from it).
    pub fn eye(&self) -> Vec3 {
        self.fly.position
    }

    /// The world-space cursor ray for picking and drag-and-drop, through the perspective unprojection
    /// ([`FlyCamera::cursor_ray`]). `far` is the same far the view projects with.
    pub fn cursor_ray(&self, pos_in_rect: Vec2, rect_size: Vec2, far: f32) -> (Vec3, Vec3) {
        self.fly.cursor_ray(pos_in_rect, rect_size, far)
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-4;

    fn bounds(center: Vec3, half: f32) -> Aabb {
        Aabb::new(center - Vec3::splat(half), center + Vec3::splat(half))
    }

    // ---- spawn ----

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

    // ---- free-fly translate ----

    #[test]
    fn strafe_is_a_d_correct_and_forward_back_track_the_ground_facing() {
        // At yaw 0 the camera faces world -Z (north): D (dx +1) strafes to +X (east, screen-right), A
        // (dx -1) to -X (west); W (dz -1) steps north (-Z), S (dz +1) south. This is the A/D sign the
        // free-fly strafe must get right.
        let mut cam = Camera::over(Vec3::ZERO);
        cam.fly.pitch = 0.0; // level, so the ground facing is exactly -Z
        let base = cam.eye();
        cam.fly(1, 0, 0); // D
        assert!(cam.eye().x > base.x, "D strafes to +X (east): {} -> {}", base.x, cam.eye().x);
        let mut left = Camera::over(Vec3::ZERO);
        left.fly(-1, 0, 0); // A
        assert!(left.eye().x < 0.0, "A strafes to -X (west): {}", left.eye().x);
        let mut fwd = Camera::over(Vec3::ZERO);
        let z0 = fwd.eye().z;
        fwd.fly(0, -1, 0); // W
        assert!(fwd.eye().z < z0, "W steps north (-Z): {} -> {}", z0, fwd.eye().z);
        let mut back = Camera::over(Vec3::ZERO);
        let z1 = back.eye().z;
        back.fly(0, 1, 0); // S
        assert!(back.eye().z > z1, "S steps south (+Z): {} -> {}", z1, back.eye().z);
    }

    #[test]
    fn the_vertical_pair_raises_and_lowers_in_world_y() {
        // Q/E move the camera in world +Y, independent of the look direction.
        let mut cam = Camera::over(Vec3::ZERO);
        let y0 = cam.eye().y;
        cam.fly(0, 0, 1); // raise (E)
        assert!((cam.eye().y - (y0 + FLY_STEP_M)).abs() < EPS, "raise steps world +Y one cell");
        cam.fly(0, 0, -1); // lower (Q)
        assert!((cam.eye().y - y0).abs() < EPS, "lower returns it");
    }

    #[test]
    fn strafe_follows_the_yaw_after_a_look() {
        // After yawing 90 degrees to the right (now facing +X / east), W steps east and D strafes south
        // (+Z) - the cluster stays camera-relative.
        let mut cam = Camera::over(Vec3::ZERO);
        cam.fly.pitch = 0.0;
        cam.look(Vec2::new(std::f32::consts::FRAC_PI_2 / LOOK_SENSITIVITY, 0.0)); // yaw -> +pi/2
        let base = cam.eye();
        cam.fly(0, -1, 0); // W, now facing east
        assert!(cam.eye().x > base.x, "W steps east after the turn: {} -> {}", base.x, cam.eye().x);
        let mut strafe = Camera::over(Vec3::ZERO);
        strafe.fly.pitch = 0.0;
        strafe.look(Vec2::new(std::f32::consts::FRAC_PI_2 / LOOK_SENSITIVITY, 0.0));
        let z0 = strafe.eye().z;
        strafe.fly(1, 0, 0); // D, facing east -> strafes south
        assert!(strafe.eye().z > z0, "D strafes south facing east: {} -> {}", z0, strafe.eye().z);
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

    // ---- ground forward (the Move grid axis reads this) ----

    #[test]
    fn ground_forward_is_minus_z_at_yaw_zero_regardless_of_pitch() {
        // The Move nearest-grid-axis reads this; it must be the yaw-only facing, unaffected by a steep
        // pitch (looking near-straight-down still steps north at yaw 0).
        let mut cam = Camera::over(Vec3::ZERO);
        cam.fly.pitch = -1.4; // steep downward look
        let gf = cam.ground_forward();
        assert!((gf - Vec2::new(0.0, -1.0)).length() < EPS, "ground forward is -Z at yaw 0: {gf:?}");
    }

    // ---- frame verb (the eased transition) ----

    #[test]
    fn frame_to_then_easing_ends_looking_at_the_centre_from_the_current_direction_at_a_fit_distance() {
        // The Frame pose: set a target framing the bounds (keeping yaw, easing pitch into the gentle
        // downward band), then advance until it settles. It ends looking straight at the bounds centre,
        // from the kept yaw, backed off past the bounds radius - and the ease has terminated.
        let mut cam = Camera::over(Vec3::new(5.0, 0.0, 5.0));
        let yaw0 = cam.fly.yaw;
        let b = bounds(Vec3::new(40.0, 2.0, -15.0), 3.0);
        cam.frame_to(b);
        let target = cam.transition.expect("frame_to arms a transition");
        // Advance to convergence (the ease is exponential, so a generous cap settles it).
        for _ in 0..200 {
            cam.advance();
        }
        assert!(cam.transition.is_none(), "the ease terminates");
        assert_eq!(cam.fly, target, "and lands exactly on the framed pose");
        let center = (b.min + b.max) * 0.5;
        let to_center = (center - cam.eye()).normalize();
        assert!((to_center - cam.fly.forward()).length() < EPS, "ends looking at the bounds centre");
        assert_eq!(cam.fly.yaw, yaw0, "keeps the current view direction (yaw)");
        assert!((center - cam.eye()).length() > 3.0, "backs off past the bounds radius");
    }

    #[test]
    fn easing_moves_toward_the_target_without_overshooting() {
        // One advance closes part of the gap (it does not jump straight there, so the view glides), and
        // it never overshoots: the distance to the target only shrinks.
        let mut cam = Camera::over(Vec3::new(5.0, 0.0, 5.0));
        let b = bounds(Vec3::new(40.0, 2.0, -15.0), 3.0);
        cam.frame_to(b);
        let target = cam.transition.unwrap();
        let gap0 = (target.position - cam.eye()).length();
        cam.advance();
        let gap1 = (target.position - cam.eye()).length();
        assert!(gap1 < gap0, "the ease closes the gap: {gap0} -> {gap1}");
        assert!(gap1 > 0.0, "but does not snap there in one frame (it glides)");
    }

    #[test]
    fn a_direct_fly_or_look_cancels_an_in_progress_frame_ease() {
        // Framing is explicit and a manual nav wins: flying or looking mid-glide drops the transition, so
        // the camera does not fight the user.
        let mut cam = Camera::over(Vec3::ZERO);
        cam.frame_to(bounds(Vec3::new(20.0, 0.0, 0.0), 2.0));
        assert!(cam.transition.is_some());
        cam.fly(1, 0, 0);
        assert!(cam.transition.is_none(), "a fly cancels the ease");
        cam.frame_to(bounds(Vec3::new(20.0, 0.0, 0.0), 2.0));
        cam.look(Vec2::new(5.0, 0.0));
        assert!(cam.transition.is_none(), "a look cancels the ease");
    }

    #[test]
    fn selecting_alone_never_moves_the_camera() {
        // The doc's rule: there is no auto-frame. The camera only moves through fly / look / the Frame
        // verb, so nothing here can move it without an explicit call - asserted by the absence of any
        // selection hook on Camera (a compile-time guarantee) and that a fresh camera at rest does not
        // drift when advance runs with no transition.
        let mut cam = Camera::over(Vec3::new(3.0, 1.0, 2.0));
        let at_rest = cam.eye();
        cam.advance(); // no transition armed
        assert_eq!(cam.eye(), at_rest, "advance is inert at rest - selection cannot move the camera");
    }
}
