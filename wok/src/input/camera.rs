//! Camera input: the frame's raw input snapshot mapped to the fly camera's movement and look.
//!
//! The vim home row drives planar movement, Ctrl reroutes it to a world-vertical elevator, and the
//! right mouse button turns raw motion into look - each gated by egui's focus claims so the same
//! physical input never drives the UI and the camera at once (`crate::input` derives those flags).

use glam::Vec2;
use wok_platform::input::InputState;
use wok_platform::winit::event::MouseButton;
use wok_platform::winit::keyboard::{Key, NamedKey};

use crate::camera::CameraInput;

/// Map the frame's raw input snapshot to the camera's input. Movement is the vim home row: with
/// Ctrl up, f/d drive forward/back and g/s strafe right/left; with Ctrl down the same f/d become
/// a world-vertical elevator (f up, d down) and planar movement is suppressed, so a command chord
/// like Ctrl+S never also drives the camera. Holding the right mouse button turns raw mouse
/// motion into look, scroll adjusts speed - except for whatever egui claimed: pointer input
/// (look, scroll) stops when the cursor is over the UI, movement keys stop when a field has
/// keyboard focus.
pub fn camera_input(input: &InputState, pointer_free: bool, keys_free: bool) -> CameraInput {
    /// Mouse-look sensitivity, radians per pixel of raw motion.
    const LOOK_SENSITIVITY: f32 = 0.0035;

    let axis = |pos: char, neg: char| {
        if !keys_free {
            return 0.0;
        }
        f32::from(char_held(input, pos)) - f32::from(char_held(input, neg))
    };
    let look_delta = if pointer_free && input.mouse_held(MouseButton::Right) {
        Vec2::new(input.mouse_motion.0 as f32, -input.mouse_motion.1 as f32) * LOOK_SENSITIVITY
    } else {
        Vec2::ZERO
    };
    // Ctrl reroutes the row to a world-vertical elevator and suppresses planar movement, so a
    // command chord (Ctrl+S, Ctrl+Z) never also flies the camera.
    let ctrl = input.key_held(NamedKey::Control);
    CameraInput {
        move_forward: if ctrl { 0.0 } else { axis('f', 'd') },
        move_right: if ctrl { 0.0 } else { axis('g', 's') },
        move_up: if ctrl { axis('f', 'd') } else { 0.0 },
        look_delta,
        speed_steps: if pointer_free { input.scroll_delta.1 } else { 0.0 },
    }
}

/// Is a printable character key held, compared case-insensitively so shift state does not stick
/// a movement key (the held-key analogue of `InputState::char_pressed`).
fn char_held(input: &InputState, ch: char) -> bool {
    input.keys_held.iter().any(|k| match k {
        Key::Character(s) => s.chars().any(|c| c.eq_ignore_ascii_case(&ch)),
        _ => false,
    })
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn input_with(keys: &[&str]) -> InputState {
        InputState {
            keys_held: keys.iter().map(|s| Key::Character((*s).into())).collect(),
            keys_pressed: HashSet::new(),
            keys_released: HashSet::new(),
            mouse_pos: (0.0, 0.0),
            mouse_delta: (0.0, 0.0),
            mouse_motion: (10.0, 4.0),
            mouse_buttons_held: HashSet::new(),
            mouse_buttons_pressed: HashSet::new(),
            mouse_buttons_released: HashSet::new(),
            scroll_delta: (0.0, 2.0),
            gamepads: vec![],
        }
    }

    #[test]
    fn home_row_drives_planar_movement_with_no_vertical() {
        // f/d forward/back, g/s right/left; the bare row never rises or sinks, and scroll still
        // sets fly speed.
        let fwd_right = camera_input(&input_with(&["f", "g"]), true, true);
        assert_eq!(fwd_right.move_forward, 1.0);
        assert_eq!(fwd_right.move_right, 1.0);
        assert_eq!(fwd_right.move_up, 0.0);
        assert_eq!(fwd_right.speed_steps, 2.0);

        let back_left = camera_input(&input_with(&["d", "s"]), true, true);
        assert_eq!(back_left.move_forward, -1.0);
        assert_eq!(back_left.move_right, -1.0);
        assert_eq!(back_left.move_up, 0.0);
    }

    #[test]
    fn ctrl_turns_the_row_into_a_vertical_elevator_and_suppresses_planar() {
        // Ctrl+f ascends; the planar keys held alongside it (here g) are suppressed, so a command
        // chord never also drives the camera.
        let mut up = input_with(&["f", "g"]);
        up.keys_held.insert(Key::Named(NamedKey::Control));
        let up = camera_input(&up, true, true);
        assert_eq!(up.move_up, 1.0, "Ctrl+f ascends");
        assert_eq!(up.move_forward, 0.0);
        assert_eq!(up.move_right, 0.0, "Ctrl suppresses planar even with g held");

        let mut down = input_with(&["d"]);
        down.keys_held.insert(Key::Named(NamedKey::Control));
        assert_eq!(camera_input(&down, true, true).move_up, -1.0, "Ctrl+d descends");
    }

    #[test]
    fn opposed_keys_cancel_and_shifted_keys_still_count() {
        // f and d are the forward/back pair; held together they cancel, and a shifted F still
        // counts (char matching is case-insensitive).
        let input = input_with(&["F", "d"]);
        assert_eq!(camera_input(&input, true, true).move_forward, 0.0);
    }

    #[test]
    fn mouse_motion_is_look_only_while_right_button_is_held() {
        let mut input = input_with(&[]);
        assert_eq!(camera_input(&input, true, true).look_delta, Vec2::ZERO);

        input.mouse_buttons_held.insert(MouseButton::Right);
        let look = camera_input(&input, true, true).look_delta;
        assert!(look.x > 0.0, "rightward motion should turn right: {look:?}");
        assert!(look.y < 0.0, "downward motion should pitch down: {look:?}");
    }

    #[test]
    fn egui_focus_suppresses_exactly_its_share_of_the_input() {
        let mut input = input_with(&["f", "g"]);
        input.mouse_buttons_held.insert(MouseButton::Right);

        // Pointer over the UI: no look, no speed scroll; movement keys still work.
        let over_ui = camera_input(&input, false, true);
        assert_eq!(over_ui.look_delta, Vec2::ZERO);
        assert_eq!(over_ui.speed_steps, 0.0);
        assert_eq!(over_ui.move_forward, 1.0);
        assert_eq!(over_ui.move_right, 1.0);

        // A text field has focus: nothing fires on any axis; pointer look still works.
        let typing = camera_input(&input, true, false);
        assert_eq!(typing.move_forward, 0.0);
        assert_eq!(typing.move_right, 0.0);
        assert_eq!(typing.move_up, 0.0);
        assert!(typing.look_delta != Vec2::ZERO);
    }
}
