//! Input routing: what the camera, hotkeys, and viewport clicks get to see after egui has
//! claimed its share.
//!
//! egui sees every raw window event first (via `App::on_window_event`); these functions then
//! consult the two focus flags the frame loop reads from egui - `pointer_free` (the cursor is
//! not over a panel and no widget is being dragged) and `keys_free` (no field has keyboard
//! focus) - so the same physical input never acts in the UI and the viewport at once. The fly
//! camera keeps right-mouse-hold to look, which leaves the cursor free for the UI by
//! construction.

use glam::Vec2;
use wok_platform::input::InputState;
use wok_platform::winit::event::MouseButton;
use wok_platform::winit::keyboard::{Key, NamedKey};

use crate::camera::{CameraInput, FlyCamera};
use crate::content::ContentPaths;
use crate::model::EditorModel;
use crate::panels::UiState;
use crate::pick;
use crate::sync;

/// Map the frame's raw input snapshot to the camera's input: WASD moves, Q/E sink and rise,
/// holding the right mouse button turns raw mouse motion into look, scroll adjusts speed -
/// except for whatever egui claimed: pointer input (look, scroll) stops when the cursor is over
/// the UI, movement keys stop when a field has keyboard focus.
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
    CameraInput {
        move_forward: axis('w', 's'),
        move_right: axis('d', 'a'),
        move_up: axis('e', 'q'),
        look_delta,
        speed_steps: if pointer_free { input.scroll_delta.1 } else { 0.0 },
    }
}

/// Hotkeys and viewport clicks: Ctrl+S saves, Delete removes the selection, Esc cancels place
/// mode then deselects, a left click places (in place mode) or picks. Ctrl+S deliberately
/// ignores `keys_free`: saving must work mid-edit in an inspector field.
pub fn handle(
    input: &InputState,
    pointer_free: bool,
    keys_free: bool,
    camera: &FlyCamera,
    size: (u32, u32),
    far: f32,
    model: &mut EditorModel,
    ui: &mut UiState,
    paths: &ContentPaths,
) {
    if input.key_held(NamedKey::Control) && input.char_pressed('s') {
        match sync::save(model, paths) {
            Ok(()) => println!("wok: saved"),
            Err(err) => eprintln!("wok: save failed: {err}"),
        }
    }
    if keys_free && input.key_pressed(NamedKey::Delete)
        && let Some(sel) = model.selection
        && let Err(err) = model.delete(sel)
    {
        eprintln!("wok: delete failed: {err}");
    }
    if keys_free && input.key_pressed(NamedKey::Escape) {
        if ui.placing.is_some() {
            ui.placing = None;
        } else {
            model.selection = None;
        }
    }

    if pointer_free && input.mouse_pressed(MouseButton::Left) {
        let size = Vec2::new(size.0 as f32, size.1.max(1) as f32);
        let view_proj = camera.view_proj(size.x / size.y, far);
        let cursor = Vec2::new(input.mouse_pos.0 as f32, input.mouse_pos.1 as f32);
        let Some(dir) = pick::cursor_ray(view_proj, camera.position, cursor, size) else { return };
        if let Some(prefab) = ui.placing.take() {
            match pick::terrain_hit(&model.heightmaps, camera.position, dir, far) {
                Some(hit) => {
                    if let Err(err) = model.place(&prefab, hit.point) {
                        eprintln!("wok: place failed: {err}");
                    }
                }
                // No terrain under the click: stay armed instead of silently dropping the mode.
                None => ui.placing = Some(prefab),
            }
        } else {
            model.selection =
                pick::pick(&model.chunks, &model.prefabs, &model.heightmaps, camera.position, dir, far);
        }
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
    fn wasd_and_qe_map_to_movement_axes() {
        let input = input_with(&["w", "d", "q"]);
        let mapped = camera_input(&input, true, true);
        assert_eq!(mapped.move_forward, 1.0);
        assert_eq!(mapped.move_right, 1.0);
        assert_eq!(mapped.move_up, -1.0);
        assert_eq!(mapped.speed_steps, 2.0);
    }

    #[test]
    fn opposed_keys_cancel_and_shifted_keys_still_count() {
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
        let mut input = input_with(&["w"]);
        input.mouse_buttons_held.insert(MouseButton::Right);

        // Pointer over the UI: no look, no speed scroll; movement keys still work.
        let over_ui = camera_input(&input, false, true);
        assert_eq!(over_ui.look_delta, Vec2::ZERO);
        assert_eq!(over_ui.speed_steps, 0.0);
        assert_eq!(over_ui.move_forward, 1.0);

        // A text field has focus: no movement; pointer look still works.
        let typing = camera_input(&input, true, false);
        assert_eq!(typing.move_forward, 0.0);
        assert!(typing.look_delta != Vec2::ZERO);
    }
}
