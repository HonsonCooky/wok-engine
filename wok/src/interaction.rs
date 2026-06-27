//! The editor's keyboard-first spatial interaction: the directional cluster, the vertical pair, and the
//! target toggle, read from wok-platform [`InputState`] and turned into camera nav or selection edits.
//!
//! This is the rebuilt interaction layer (designs/movement-camera-design.md), the clean-slate
//! replacement for the demolished mouse-only camera and held-key gizmo. The whole scheme is one
//! directional cluster (4-way) plus a vertical pair (raise / lower), time-shared by the persistent
//! [`Target`](crate::model::Target) toggle: in `Look` the cluster pans and the vertical pair zooms the
//! [`LayoutCamera`], in `Move` they grid-step the selection. The verbs read wok-platform input (not
//! egui), so the same grammar maps onto a controller later (gilrs is in wok-platform); the chrome's egui
//! shortcuts (Ctrl+S, Esc) stay separate.
//!
//! Where it runs: the frame loop's interaction seam (`crate::main`), after the chrome's actions drain
//! and before the draw - the spot the old interaction plugged into. [`keyboard`] reads the cluster each
//! frame and either mutates the camera (Look) or returns the selection edits (Move) for the frame loop
//! to route through the single writer; it is focus-gated, so a focused text field types instead.
//!
//! Pure where it can be: the cluster-to-cardinal mapping ([`cluster_step`]) and the grid step
//! ([`grid_step`]) are pure and unit tested; the camera math lives in [`crate::camera`]. `keyboard`
//! itself reads an [`InputState`] snapshot, so it is testable by building one.
//!
//! Keybinding is PARKED: the keycaps below are temporary, sane placeholders so the verbs are testable
//! now, NOT the final layout. The binding settles as a rebindable table fitted to the ZSA Voyager
//! (movement-camera-design.md "Keybinding"); the prior scheme failed on left-hand modifier reach, which
//! that binding addresses, so do not read these letters as the design.

use wok_platform::input::InputState;
use wok_platform::winit::keyboard::NamedKey;

use crate::action::Action;
use crate::camera::LayoutCamera;
use crate::model::{Model, Target};

// ---- placeholder keybinds (PARKED - see the module doc) ----

/// The directional cluster (4-way), camera-relative. In Layout (top-down) the screen directions map to
/// fixed world cardinals (see [`cluster_step`]).
const CLUSTER_FORWARD: char = 'w'; // screen up    -> world -Z (north)
const CLUSTER_BACK: char = 's'; //    screen down  -> world +Z (south)
const CLUSTER_LEFT: char = 'a'; //    screen left  -> world -X (west)
const CLUSTER_RIGHT: char = 'd'; //   screen right -> world +X (east)

/// The vertical pair: world +Y / -Y in Move, zoom out / in in Look.
const RAISE: char = 'e';
const LOWER: char = 'q';

/// The target toggle (Move <-> Look) - a thumb-tap placeholder.
const TOGGLE: NamedKey = NamedKey::Space;

/// Read one frame of keyboard input and drive the editor: the toggle flips the cluster target, then the
/// cluster and vertical pair either pan and zoom the camera (Look) or step the selection (Move). Returns
/// the actions for the frame loop to route through the single writer (the toggle, and - in Move - the
/// selection's transform edits); the camera mutates in place, since it is frame-loop residency, not model
/// state.
///
/// Focus-gated: `typing` is true when a text field holds keyboard focus, and a held Ctrl is a chrome
/// chord (Ctrl+S and friends), so in either case the spatial verbs stay inert and the keys reach the
/// chrome instead. Hold-to-repeat rides the OS key-repeat (wok-platform's `keys_repeating`): a tap is one
/// step, a hold repeats at the OS rate. The toggle is the press edge only, so a held thumb does not
/// flip-flop.
pub fn keyboard(input: &InputState, typing: bool, model: &Model, camera: &mut LayoutCamera) -> Vec<Action> {
    let mut actions = Vec::new();
    if typing || input.key_held(NamedKey::Control) || input.key_held(NamedKey::Super) {
        return actions;
    }
    if input.key_pressed(TOGGLE) {
        actions.push(Action::ToggleTarget);
    }
    // The press edge OR an OS auto-repeat, so a tap steps once and a hold repeats.
    let on = |c: char| input.char_pressed(c) || input.char_repeating(c);
    let (dx, dz) = cluster_step(on(CLUSTER_FORWARD), on(CLUSTER_BACK), on(CLUSTER_LEFT), on(CLUSTER_RIGHT));
    let dy = on(RAISE) as i32 - on(LOWER) as i32;
    match model.shell.target() {
        Target::Look => {
            // The cluster pans the focus across the plane; the vertical pair zooms (raise out, lower in).
            // Camera-only - no model mutation, so nothing routes through the writer.
            if dx != 0 || dz != 0 {
                camera.pan(dx, dz);
            }
            if dy != 0 {
                camera.zoom(dy);
            }
        }
        Target::Move => {
            // The cluster grid-steps the selection and the vertical pair steps its world Y: the
            // grid-step Move, the next bite. Left inert here so Look lands first.
            let _ = (dx, dz, dy);
        }
    }
    actions
}

/// The world-cardinal grid step the directional cluster maps to in Layout (top-down): screen up is world
/// `-Z` (north), down `+Z`, left `-X`, right `+X` - exact, because the view looks straight down
/// (movement-camera-design.md "Move"). Returns `(dx, dz)` in grid cells (one cell per pressed direction;
/// opposite presses cancel). In Look this is the pan direction; in Move (next bite) the selection step.
///
/// Four bools, one per cluster direction: they are independent (forward + left is a held diagonal), so a
/// two-variant enum cannot stand in for them - the directions are the natural signature here.
#[allow(clippy::fn_params_excessive_bools)]
fn cluster_step(forward: bool, back: bool, left: bool, right: bool) -> (i32, i32) {
    let dx = right as i32 - left as i32;
    let dz = back as i32 - forward as i32;
    (dx, dz)
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use glam::Vec3;
    use std::collections::HashSet;
    use wok_platform::winit::keyboard::Key;

    /// Build an [`InputState`] snapshot with the given character keys pressed (and so held). Enough to
    /// drive [`keyboard`]; the mouse, scroll, and gamepad fields stay empty.
    fn input_with_chars(pressed: &[char]) -> InputState {
        let keys_pressed: HashSet<Key> = pressed.iter().map(|c| Key::Character(c.to_string().into())).collect();
        InputState {
            keys_held: keys_pressed.clone(),
            keys_pressed,
            keys_repeating: HashSet::new(),
            keys_released: HashSet::new(),
            mouse_pos: (0.0, 0.0),
            mouse_delta: (0.0, 0.0),
            mouse_motion: (0.0, 0.0),
            mouse_buttons_held: HashSet::new(),
            mouse_buttons_pressed: HashSet::new(),
            mouse_buttons_released: HashSet::new(),
            scroll_delta: (0.0, 0.0),
            gamepads: Vec::new(),
        }
    }

    #[test]
    fn cluster_step_maps_screen_directions_to_world_cardinals() {
        // The map orientation, exact under the straight-down view: forward is north (-Z), right is east.
        assert_eq!(cluster_step(true, false, false, false), (0, -1), "forward steps world -Z (north)");
        assert_eq!(cluster_step(false, true, false, false), (0, 1), "back steps +Z (south)");
        assert_eq!(cluster_step(false, false, true, false), (-1, 0), "left steps -X (west)");
        assert_eq!(cluster_step(false, false, false, true), (1, 0), "right steps +X (east)");
        assert_eq!(cluster_step(true, true, true, true), (0, 0), "opposite presses cancel");
        assert_eq!(cluster_step(true, false, false, true), (1, -1), "forward + right is a diagonal NE step");
    }

    #[test]
    fn keyboard_look_pans_and_zooms_the_camera_with_no_model_action() {
        let mut model = Model::default();
        model.shell.toggle_target(); // -> Look
        let mut cam = LayoutCamera::over(Vec3::new(5.0, 0.0, 5.0));
        let before = cam.focus;
        let actions = keyboard(&input_with_chars(&['w']), false, &model, &mut cam);
        assert!(actions.is_empty(), "Look is camera-only - no model edit to route");
        assert!(cam.focus.z < before.z, "forward pans the focus north (-Z): {:?}", cam.focus);
        // The vertical pair zooms: raise enlarges the half-height (zoom out).
        let zoomed_out = cam.half_height;
        let _ = keyboard(&input_with_chars(&['e']), false, &model, &mut cam);
        assert!(cam.half_height > zoomed_out, "raise zooms out in Look");
    }

    #[test]
    fn keyboard_toggle_routes_through_the_writer_and_a_focused_field_swallows_the_keys() {
        let model = Model::default();
        let mut cam = LayoutCamera::over(Vec3::ZERO);
        // The toggle key emits ToggleTarget (the frame loop applies it through the single writer).
        let actions = keyboard(&input_with_chars(&[]), false, &model, &mut cam);
        assert!(actions.is_empty(), "no keys, no actions");
        let toggled = {
            let mut input = input_with_chars(&[]);
            input.keys_pressed.insert(Key::Named(NamedKey::Space));
            input.keys_held.insert(Key::Named(NamedKey::Space));
            keyboard(&input, false, &model, &mut cam)
        };
        assert_eq!(toggled, vec![Action::ToggleTarget], "the toggle key emits ToggleTarget");
        // Focus-gated: with a text field focused, every verb key is inert and the camera does not move.
        let before = cam.focus;
        let gated = keyboard(&input_with_chars(&['w']), true, &model, &mut cam);
        assert!(gated.is_empty() && cam.focus == before, "a focused field swallows the verb keys");
    }
}
