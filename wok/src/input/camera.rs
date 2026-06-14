//! Camera input: the frame's raw input snapshot mapped to the fly camera's movement and look.
//!
//! WASD drives planar movement, Q/E a world-vertical elevator, and the right mouse button turns raw
//! motion into look - each gated by egui's focus claims so the same physical input never drives the
//! UI and the camera at once (`crate::input` derives those flags). Holding Ctrl suppresses movement,
//! so a command chord like Ctrl+S never also flies the camera.

use glam::Vec2;
use wok_platform::input::InputState;
use wok_platform::winit::event::MouseButton;
use wok_platform::winit::keyboard::{Key, NamedKey};

use crate::camera::CameraInput;

/// Map the frame's raw input snapshot to the camera's input. Movement is WASD plus a Q/E vertical
/// elevator (E up, Q down); holding Ctrl suppresses all movement, so a command chord like Ctrl+S
/// saves and Ctrl+Z undoes without also flying the camera. Holding the right mouse button turns raw
/// mouse motion into look, scroll adjusts speed - except for whatever egui claimed: pointer input
/// (look, scroll) stops when the cursor is over the UI, movement keys stop when a field has keyboard
/// focus.
pub fn camera_input(input: &InputState, pointer_free: bool, keys_free: bool) -> CameraInput {
    /// Mouse-look sensitivity, radians per pixel of raw motion.
    const LOOK_SENSITIVITY: f32 = 0.0035;

    // Movement is suppressed when a field has focus (`keys_free`) or a command chord is held (Ctrl),
    // so a chord like Ctrl+S saves and Ctrl+Z undoes without also flying the camera.
    let move_free = keys_free && !input.key_held(NamedKey::Control);
    let axis = |pos: char, neg: char| {
        if !move_free {
            return 0.0;
        }
        f32::from(char_held(input, pos)) - f32::from(char_held(input, neg))
    };
    let look_delta = if pointer_free && input.mouse_held(MouseButton::Right) {
        Vec2::new(input.mouse_motion.0 as f32, -input.mouse_motion.1 as f32) * LOOK_SENSITIVITY
    } else {
        Vec2::ZERO
    };
    CameraInput {
        move_forward: axis('w', 's'),
        move_right: axis('d', 'a'),
        move_up: axis('e', 'q'),
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
    fn wasd_drives_planar_movement_with_no_vertical() {
        // W/S forward/back, A/D left/right; the bare cluster never rises or sinks, and scroll still
        // sets fly speed.
        let fwd_right = camera_input(&input_with(&["w", "d"]), true, true);
        assert_eq!(fwd_right.move_forward, 1.0);
        assert_eq!(fwd_right.move_right, 1.0);
        assert_eq!(fwd_right.move_up, 0.0);
        assert_eq!(fwd_right.speed_steps, 2.0);

        let back_left = camera_input(&input_with(&["s", "a"]), true, true);
        assert_eq!(back_left.move_forward, -1.0);
        assert_eq!(back_left.move_right, -1.0);
        assert_eq!(back_left.move_up, 0.0);
    }

    #[test]
    fn q_and_e_drive_the_vertical_elevator() {
        assert_eq!(camera_input(&input_with(&["e"]), true, true).move_up, 1.0, "E ascends");
        assert_eq!(camera_input(&input_with(&["q"]), true, true).move_up, -1.0, "Q descends");
    }

    #[test]
    fn ctrl_suppresses_movement_so_a_command_chord_never_flies() {
        // A command chord (Ctrl+S save, Ctrl+Z undo) must not also drive the camera, so every
        // movement key is suppressed while Ctrl is held - planar and vertical alike.
        let mut held = input_with(&["w", "d", "e"]);
        held.keys_held.insert(Key::Named(NamedKey::Control));
        let c = camera_input(&held, true, true);
        assert_eq!(c.move_forward, 0.0);
        assert_eq!(c.move_right, 0.0);
        assert_eq!(c.move_up, 0.0, "Ctrl suppresses the vertical keys too");
    }

    #[test]
    fn opposed_keys_cancel_and_shifted_keys_still_count() {
        // W and S are the forward/back pair; held together they cancel, and a shifted W still
        // counts (char matching is case-insensitive).
        let input = input_with(&["W", "s"]);
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
        let mut input = input_with(&["w", "d"]);
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
