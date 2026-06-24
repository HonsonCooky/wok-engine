//! Viewport input: the frame's raw input snapshot mapped to the mouse-only editor camera.
//!
//! egui sees every raw window event first (via `App::on_window_event`); [`camera_input`] then
//! consults `pointer_free` - the cursor is over the editor viewport and egui is not using the pointer
//! for its own UI (computed in `crate::main`, the 911a258 gate) - so the same motion never drives a
//! panel and the camera at once. The camera is mouse-only and always live (designs/editor-design.md,
//! Input): hold the right button and move to look, scroll to dolly along the look, hold the middle
//! button and move to pan the view plane. There is no keyboard movement and no camera mode, so the
//! left hand is left wholly to operators and precision (which return with picking and place).
//! Everything here is unit testable with no window.

use glam::Vec2;
use wok_platform::input::InputState;
use wok_platform::winit::event::MouseButton;

use crate::camera::CameraInput;

/// Mouse-look sensitivity, radians per pixel of raw motion. Unchanged from the prior fly camera.
const LOOK_SENSITIVITY: f32 = 0.0035;

/// Dolly distance per scroll notch, in metres. Generous on purpose: the starter scene is one ~128 m
/// chunk, so crossing it should be a few flicks of the wheel rather than a long grind. Fixed for now;
/// scaling it by the distance to what the camera looks at (so close work dollies finer) is the
/// parked refinement, not a second way to translate.
const DOLLY_PER_NOTCH: f32 = 6.0;

/// Pan distance per pixel of raw motion, in metres. Tuned for a near one-to-one grab-the-world feel
/// at a typical working distance (tens of metres); far out it reads slow, which the same
/// view-distance scale as the dolly would fix. Fixed for now.
const PAN_SENSITIVITY: f32 = 0.06;

/// Map the frame's raw input snapshot to the camera's input, all from the mouse: hold the right
/// button and move to look, scroll to dolly along the look, hold the middle button and move to pan
/// the view plane. When the cursor is not free for the viewport (`pointer_free` is false - over the
/// chrome, an open menu, or an in-progress egui drag) nothing drives the camera, so the UI keeps its
/// own pointer input. Raw motion (not the cursor delta) is used so a future cursor lock would not
/// change the feel.
pub fn camera_input(input: &InputState, pointer_free: bool) -> CameraInput {
    if !pointer_free {
        return CameraInput::default();
    }
    let motion = Vec2::new(input.mouse_motion.0 as f32, input.mouse_motion.1 as f32);
    let look_delta = if input.mouse_held(MouseButton::Right) {
        // Rightward motion turns right (+yaw); downward motion pitches down (the view follows the
        // mouse), so screen y is negated against the pitch-up convention.
        Vec2::new(motion.x, -motion.y) * LOOK_SENSITIVITY
    } else {
        Vec2::ZERO
    };
    let pan = if input.mouse_held(MouseButton::Middle) {
        // Scene tracks the drag: dragging right moves the camera left (-right) and dragging down moves
        // it up (+up), so the grabbed point follows the cursor. crate::camera applies pan.x along
        // right and pan.y along up, so the signs live here.
        Vec2::new(-motion.x, motion.y) * PAN_SENSITIVITY
    } else {
        Vec2::ZERO
    };
    CameraInput { look_delta, dolly: input.scroll_delta.1 * DOLLY_PER_NOTCH, pan }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// An input snapshot with the given buttons held, raw motion, and vertical scroll - all the camera
    /// reads. Everything else is empty (the camera is mouse-only).
    fn mouse(buttons: &[MouseButton], motion: (f64, f64), scroll: f32) -> InputState {
        InputState {
            keys_held: HashSet::new(),
            keys_pressed: HashSet::new(),
            keys_released: HashSet::new(),
            mouse_pos: (0.0, 0.0),
            mouse_delta: (0.0, 0.0),
            mouse_motion: motion,
            mouse_buttons_held: buttons.iter().copied().collect(),
            mouse_buttons_pressed: HashSet::new(),
            mouse_buttons_released: HashSet::new(),
            scroll_delta: (0.0, scroll),
            gamepads: vec![],
        }
    }

    #[test]
    fn right_drag_looks_and_leaves_dolly_and_pan_idle() {
        // Hold the right button and move: rightward motion turns right (+yaw), downward motion pitches
        // down (-pitch). Nothing else fires.
        let look = camera_input(&mouse(&[MouseButton::Right], (10.0, 4.0), 0.0), true);
        assert!(look.look_delta.x > 0.0, "rightward motion turns right: {:?}", look.look_delta);
        assert!(look.look_delta.y < 0.0, "downward motion pitches down: {:?}", look.look_delta);
        assert_eq!(look.dolly, 0.0);
        assert_eq!(look.pan, Vec2::ZERO);
    }

    #[test]
    fn scroll_dollies_along_the_look_and_scales_with_the_notches() {
        // Scroll dollies with no button held; the amount scales linearly with the notch count.
        let one = camera_input(&mouse(&[], (0.0, 0.0), 1.0), true);
        assert_eq!(one.dolly, DOLLY_PER_NOTCH, "one notch dollies one step");
        assert_eq!(one.look_delta, Vec2::ZERO);
        assert_eq!(one.pan, Vec2::ZERO);
        let three = camera_input(&mouse(&[], (0.0, 0.0), 3.0), true);
        assert_eq!(three.dolly, 3.0 * DOLLY_PER_NOTCH, "dolly scales with the notch count");
    }

    #[test]
    fn middle_drag_pans_so_the_scene_tracks_the_drag() {
        // Dragging right (+x motion) moves the camera left (pan.x < 0, applied along +right) and
        // dragging down (+y motion) moves it up (pan.y > 0, applied along +up), so the grabbed point
        // follows the cursor.
        let input = camera_input(&mouse(&[MouseButton::Middle], (10.0, 4.0), 0.0), true);
        assert!(input.pan.x < 0.0, "drag right moves the camera left: {:?}", input.pan);
        assert!(input.pan.y > 0.0, "drag down moves the camera up: {:?}", input.pan);
        assert_eq!(input.look_delta, Vec2::ZERO, "a middle-drag does not look");
        assert_eq!(input.dolly, 0.0);
    }

    #[test]
    fn the_chrome_and_menus_take_all_camera_input_when_the_pointer_is_not_free() {
        // pointer_free is false over the chrome, an open menu, or an in-progress egui drag (the
        // 911a258 gate): no look, no dolly, no pan, even with both buttons held and the wheel turning.
        let busy = mouse(&[MouseButton::Right, MouseButton::Middle], (10.0, 4.0), 2.0);
        assert_eq!(camera_input(&busy, false), CameraInput::default());
    }
}
