//! Input-to-intent mapping: one frame's raw device snapshot becomes what the player meant.
//!
//! The [`Intent`] is the seam between the platform and the simulation: everything downstream (the
//! fixed-step loop, the camera) reads intent, never raw input, so the bindings live in exactly one
//! place and the simulation can be driven by scripted intents in the headless replay test. Pure
//! (snapshot and dt in, intent out), so the mapping is unit testable without a window.
//!
//! Bindings: WASD or the left stick for movement (analog magnitude flows through), space or the
//! south button to jump, the mouse or the right stick to orbit, Esc or Select/Back to quit. Mouse
//! look is always live - the game owns the pointer (the app locks and hides the OS cursor), so
//! there is no held-button gate. Keyboard and gamepad coexist by summation: an idle device
//! contributes zero, so whichever moved last is what the player feels, with no device-switching
//! state. The two look devices differ in kind - the mouse reports a displacement, the stick a held
//! rate - so the stick is integrated by the frame dt; and feel is judged per device - each
//! play-test verdict lands on one device alone - so each carries its own inversion pair
//! (`MOUSE_INVERT_*`, `STICK_INVERT_*`), applied per device before the sum.

use glam::Vec2;
use wok_platform::gilrs::Button;
use wok_platform::input::InputState;
use wok_platform::winit::keyboard::{Key, NamedKey};

use crate::constants::{MOUSE_INVERT_X, MOUSE_INVERT_Y, STICK_DEADZONE, STICK_INVERT_X, STICK_INVERT_Y};
use crate::tuning::Tuning;

/// What the player asked for this frame, in the simulation's terms: movement axes relative to the
/// camera (forward and right, resolved against the camera yaw at step time), a jump edge, and the
/// orbit delta in radians.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Intent {
    /// Forward minus backward (W minus S, or the left stick's forward deflection). Analog: the
    /// stick contributes its deadzoned magnitude, not a thresholded digital step.
    pub move_forward: f32,
    /// Right minus left (D minus A, or the left stick's rightward deflection).
    pub move_right: f32,
    /// Space or the south button was pressed this frame (an edge, not a hold: holding does not
    /// bounce). The edge is the whole jump signal: every jump flies the full authored arc, so
    /// nothing downstream reads how long the control stays down.
    pub jump: bool,
    /// Radians of orbit to add this frame: `x` to yaw, `y` to pitch. The base mapping turns the
    /// view with the drag or deflection - the boom convention makes view-right a negative yaw
    /// delta - and raises the camera as the input pushes forward; the per-device `MOUSE_INVERT_*`
    /// and `STICK_INVERT_*` constants flip an axis for that device alone.
    pub look_delta: Vec2,
    /// Esc or the gamepad's Select/Back was pressed this frame: the player asked to quit. The app
    /// turns this into the platform's clean-shutdown request; the simulation never sees it.
    pub quit: bool,
}

/// Map one frame's input snapshot to the player's intent. `dt` integrates the right stick's turn
/// rate; the mouse contribution is already a per-frame displacement and ignores it. The look
/// sensitivities (mouse and stick) come from the live tuning; the deadzone and inversion stay
/// build-fixed device policy in `crate::constants`.
pub fn map_input(input: &InputState, dt: f32, tuning: &Tuning) -> Intent {
    let axis = |pos: char, neg: char| f32::from(char_held(input, pos)) - f32::from(char_held(input, neg));
    let pad = input.gamepad(0);

    // wok-platform's collector flips stick Y to the screen convention (Y+ is down), the same sign
    // raw mouse motion uses, so both look devices and the movement stick share one sign treatment:
    // "forward/up" is negative Y until the inversion convention is applied.
    let move_stick =
        deadzone(pad.map_or(Vec2::ZERO, |p| Vec2::new(p.left_stick.0, p.left_stick.1)), STICK_DEADZONE);
    let look_stick =
        deadzone(pad.map_or(Vec2::ZERO, |p| Vec2::new(p.right_stick.0, p.right_stick.1)), STICK_DEADZONE);

    // Each device resolves to its own orbit delta - sensitivity or rate, then its own inversion
    // pair - before the sum, so flipping the mouse cannot drag the stick's feel with it. Raw mouse
    // motion always drives look: the pointer is the game's (locked and hidden), not a desktop
    // cursor that might be on its way somewhere else.
    let mouse_raw =
        Vec2::new(input.mouse_motion.0 as f32, input.mouse_motion.1 as f32) * tuning.mouse_look_sensitivity;
    let mouse = Vec2::new(
        mouse_raw.x * axis_sign(MOUSE_INVERT_X),
        mouse_raw.y * axis_sign(MOUSE_INVERT_Y),
    );
    let stick_raw = look_stick * tuning.stick_look_rate * dt;
    let stick = Vec2::new(
        stick_raw.x * axis_sign(STICK_INVERT_X),
        stick_raw.y * axis_sign(STICK_INVERT_Y),
    );

    Intent {
        move_forward: axis('w', 's') - move_stick.y,
        move_right: axis('d', 'a') + move_stick.x,
        jump: input.key_pressed(NamedKey::Space)
            || pad.is_some_and(|p| p.buttons_pressed.contains(&Button::South)),
        look_delta: mouse + stick,
        quit: input.key_pressed(NamedKey::Escape)
            || pad.is_some_and(|p| p.buttons_pressed.contains(&Button::Select)),
    }
}

/// Radial deadzone with magnitude rescale: deflections inside `deadzone` read zero, and the live
/// range is stretched so output magnitude runs continuously from 0 at the deadzone's edge to 1 at
/// full deflection (clamped there - a square gate's diagonal can exceed unit length). Radial, not
/// per-axis, so the dead region is a circle and diagonal deflections keep their direction.
fn deadzone(stick: Vec2, deadzone: f32) -> Vec2 {
    let len = stick.length();
    if len <= deadzone {
        return Vec2::ZERO;
    }
    let live = ((len - deadzone) / (1.0 - deadzone)).min(1.0);
    stick * (live / len)
}

/// The sign an axis's raw motion contributes to the orbit. Non-inverted is negative on both axes:
/// rightward motion turns the view right by swinging the boom the other way (a negative yaw delta),
/// and downward motion lowers the camera (a negative pitch delta).
fn axis_sign(invert: bool) -> f32 {
    if invert { 1.0 } else { -1.0 }
}

/// Is a printable character key held, compared case-insensitively so shift state does not stick a
/// movement key (the same held-key reading the editor uses).
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
    use wok_platform::input::GamepadState;

    const DT: f32 = 1.0 / 60.0;

    /// Map under the shipped defaults: every binding test reads the default look sensitivities, the
    /// same values play ships with. Tests that pin a sensitivity scale read it back off
    /// `Tuning::default()` so they move with the default, not a hard-coded copy.
    fn mapped(input: &InputState, dt: f32) -> Intent {
        map_input(input, dt, &Tuning::default())
    }

    fn input_with(held: &[&str], pressed_space: bool) -> InputState {
        let mut keys_pressed = HashSet::new();
        if pressed_space {
            keys_pressed.insert(Key::Named(NamedKey::Space));
        }
        InputState {
            keys_held: held.iter().map(|s| Key::Character((*s).into())).collect(),
            keys_pressed,
            keys_repeating: HashSet::new(),
            keys_released: HashSet::new(),
            mouse_pos: (0.0, 0.0),
            mouse_delta: (0.0, 0.0),
            // Still by default: mouse look is ungated now, so a moving fixture mouse would leak
            // into every non-mouse test. The mouse tests set their own motion.
            mouse_motion: (0.0, 0.0),
            mouse_buttons_held: HashSet::new(),
            mouse_buttons_pressed: HashSet::new(),
            mouse_buttons_released: HashSet::new(),
            scroll_delta: (0.0, 0.0),
            gamepads: vec![],
        }
    }

    /// The fixture with a moved mouse: raw motion (10, 4), rightward and downward.
    fn input_with_mouse_motion() -> InputState {
        let mut input = input_with(&[], false);
        input.mouse_motion = (10.0, 4.0);
        input
    }

    fn pad(left: (f32, f32), right: (f32, f32), south: bool) -> GamepadState {
        let mut buttons_pressed = HashSet::new();
        if south {
            buttons_pressed.insert(Button::South);
        }
        GamepadState {
            left_stick: left,
            right_stick: right,
            left_trigger: 0.0,
            right_trigger: 0.0,
            buttons_held: HashSet::new(),
            buttons_pressed,
        }
    }

    fn input_with_pad(gamepad: GamepadState) -> InputState {
        let mut input = input_with(&[], false);
        input.gamepads = vec![gamepad];
        input
    }

    // ---- keyboard and mouse ----

    #[test]
    fn wasd_maps_to_the_movement_axes() {
        let intent = mapped(&input_with(&["w", "d"], false), DT);
        assert_eq!(intent.move_forward, 1.0);
        assert_eq!(intent.move_right, 1.0);
        let intent = mapped(&input_with(&["s", "a"], false), DT);
        assert_eq!(intent.move_forward, -1.0);
        assert_eq!(intent.move_right, -1.0);
    }

    #[test]
    fn opposed_keys_cancel_and_shifted_keys_still_count() {
        let intent = mapped(&input_with(&["W", "s"], false), DT);
        assert_eq!(intent.move_forward, 0.0);
    }

    #[test]
    fn space_pressed_is_a_jump_edge() {
        assert!(mapped(&input_with(&[], true), DT).jump);
        assert!(!mapped(&input_with(&[], false), DT).jump);
    }

    #[test]
    fn holding_space_does_not_re_raise_the_jump_edge() {
        // The edge is the whole jump signal: held space (no fresh press) asks for no jump, so
        // holding can never bounce a second jump out of one press.
        let mut input = input_with(&[], false);
        input.keys_held.insert(Key::Named(NamedKey::Space));
        assert!(!mapped(&input, DT).jump);
    }

    #[test]
    fn mouse_motion_always_drives_look_and_a_still_mouse_is_silent() {
        // No held-button gate: the pointer belongs to the game (locked and hidden), so raw motion
        // is look with no button held, and a motionless mouse contributes exactly zero.
        assert_ne!(mapped(&input_with_mouse_motion(), DT).look_delta, Vec2::ZERO);
        assert_eq!(mapped(&input_with(&[], false), DT).look_delta, Vec2::ZERO);
    }

    #[test]
    fn mouse_horizontal_turns_the_view_and_vertical_is_flipped() {
        // The settled mouse verdict: horizontal keeps the base view-turn mapping (moving right
        // turns the view right, a negative yaw delta - the both-axis flip overcorrected), while
        // vertical stays flipped (moving down raises the camera, a positive pitch delta).
        let look = mapped(&input_with_mouse_motion(), DT).look_delta;
        assert!(look.x < 0.0, "a rightward motion should turn the view right (negative yaw): {look:?}");
        assert!(look.y > 0.0, "a downward motion should raise the camera (positive pitch): {look:?}");
    }

    #[test]
    fn mouse_sensitivity_scales_the_motion_exactly() {
        // The flip is a sign, never a magnitude: the delta is the raw motion times the mouse
        // sensitivity, axis for axis.
        let sens = Tuning::default().mouse_look_sensitivity;
        let look = mapped(&input_with_mouse_motion(), DT).look_delta;
        assert!((look.x.abs() - 10.0 * sens).abs() < 1e-6, "got {look:?}");
        assert!((look.y.abs() - 4.0 * sens).abs() < 1e-6, "got {look:?}");
    }

    // ---- quitting ----

    #[test]
    fn escape_or_select_is_a_quit_edge() {
        let mut input = input_with(&[], false);
        assert!(!mapped(&input, DT).quit, "no quit without a press");
        input.keys_pressed.insert(Key::Named(NamedKey::Escape));
        assert!(mapped(&input, DT).quit, "Esc quits");

        let mut gamepad = pad((0.0, 0.0), (0.0, 0.0), false);
        gamepad.buttons_pressed.insert(Button::Select);
        assert!(mapped(&input_with_pad(gamepad), DT).quit, "Select/Back quits");
    }

    // ---- deadzone math ----

    #[test]
    fn deflection_inside_the_deadzone_reads_zero() {
        assert_eq!(deadzone(Vec2::new(0.1, 0.05), 0.15), Vec2::ZERO);
        assert_eq!(deadzone(Vec2::ZERO, 0.15), Vec2::ZERO);
        // Exactly at the edge is still dead: the live range starts strictly past it.
        assert_eq!(deadzone(Vec2::new(0.15, 0.0), 0.15), Vec2::ZERO);
    }

    #[test]
    fn the_live_range_rescales_continuously_from_zero_to_one() {
        // Halfway through the live range (deadzone 0.15: deflection 0.575) is magnitude 0.5, and
        // full deflection is exactly 1: no jump at the deadzone edge, full speed at full tilt.
        let half = deadzone(Vec2::new(0.575, 0.0), 0.15);
        assert!((half.x - 0.5).abs() < 1e-6, "got {half:?}");
        assert_eq!(deadzone(Vec2::new(1.0, 0.0), 0.15), Vec2::new(1.0, 0.0));
    }

    #[test]
    fn the_deadzone_is_radial_and_preserves_direction() {
        // A diagonal deflection keeps its direction, and a square gate's corner (length > 1)
        // clamps to unit magnitude instead of overshooting.
        let out = deadzone(Vec2::new(1.0, 1.0), 0.15);
        assert!((out.length() - 1.0).abs() < 1e-6, "corner should clamp to unit: {out:?}");
        assert!((out.x - out.y).abs() < 1e-6, "direction should be preserved: {out:?}");
    }

    // ---- stick-to-intent mapping ----

    #[test]
    fn the_left_stick_moves_with_analog_magnitude() {
        // Stick pushed up (negative y in the platform's screen convention) at full tilt is full
        // forward; a partial deflection flows through as a partial magnitude, not a digital 1.
        let full = mapped(&input_with_pad(pad((0.0, -1.0), (0.0, 0.0), false)), DT);
        assert_eq!(full.move_forward, 1.0);
        assert_eq!(full.move_right, 0.0);

        let partial = mapped(&input_with_pad(pad((0.575, 0.0), (0.0, 0.0), false)), DT);
        assert!((partial.move_right - 0.5).abs() < 1e-6, "got {}", partial.move_right);
    }

    #[test]
    fn a_resting_stick_moves_nothing() {
        let intent = mapped(&input_with_pad(pad((0.05, -0.08), (0.0, 0.0), false)), DT);
        assert_eq!(intent.move_forward, 0.0);
        assert_eq!(intent.move_right, 0.0);
    }

    #[test]
    fn the_south_button_is_a_jump_edge() {
        assert!(mapped(&input_with_pad(pad((0.0, 0.0), (0.0, 0.0), true)), DT).jump);
        assert!(!mapped(&input_with_pad(pad((0.0, 0.0), (0.0, 0.0), false)), DT).jump);
    }

    #[test]
    fn the_held_south_button_does_not_re_raise_the_jump_edge() {
        // The pad's side of the same rule: buttons_held carries the hold, and a hold is not a
        // press.
        let mut gamepad = pad((0.0, 0.0), (0.0, 0.0), false);
        gamepad.buttons_held.insert(Button::South);
        assert!(!mapped(&input_with_pad(gamepad), DT).jump);
    }

    #[test]
    fn the_right_stick_orbits_as_a_rate_integrated_by_dt() {
        // The stick's horizontal verdict: the base view-turn mapping. Rightward deflection turns
        // the view right (negative yaw delta), and the contribution is proportional to dt because
        // deflection is a held rate. This must stay pinned while the stick's vertical is flipped:
        // the inversion is per axis as well as per device.
        let rate = Tuning::default().stick_look_rate;
        let one = mapped(&input_with_pad(pad((0.0, 0.0), (1.0, 0.0), false)), DT);
        assert!((one.look_delta.x - -(rate * DT)).abs() < 1e-6, "got {:?}", one.look_delta);

        let double = mapped(&input_with_pad(pad((0.0, 0.0), (1.0, 0.0), false)), DT * 2.0);
        assert!((double.look_delta.x - one.look_delta.x * 2.0).abs() < 1e-6);
    }

    #[test]
    fn a_forward_stick_push_lowers_the_camera() {
        // The stick's vertical pin, separate from the mouse's: the flipped verdict means pushing
        // the right stick forward (negative y in the platform's screen convention) lowers the
        // camera - a negative pitch delta, the inverse of the base mapping's sign.
        let look = mapped(&input_with_pad(pad((0.0, 0.0), (0.0, -1.0), false)), DT).look_delta;
        assert!(look.y < 0.0, "a forward push should lower the camera (negative pitch): {look:?}");
    }

    #[test]
    fn keyboard_and_stick_sum_so_the_idle_device_is_silent() {
        // The coexistence rule: an idle stick adds zero to a held key, and vice versa, so
        // whichever device moved last is what the player feels with no switching state.
        let mut input = input_with(&["w"], false);
        input.gamepads = vec![pad((0.0, 0.0), (0.0, 0.0), false)];
        assert_eq!(mapped(&input, DT).move_forward, 1.0);

        let stick_only = mapped(&input_with_pad(pad((0.0, -1.0), (0.0, 0.0), false)), DT);
        assert_eq!(stick_only.move_forward, 1.0);
    }
}
