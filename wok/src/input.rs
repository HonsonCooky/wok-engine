//! Input routing: what the camera, hotkeys, and viewport clicks get to see after egui has
//! claimed its share.
//!
//! egui sees every raw window event first (via `App::on_window_event`); these functions then
//! consult the two focus flags the frame loop reads from egui - `pointer_free` (the cursor is
//! not over a panel and no widget is being dragged) and `keys_free` (no field has keyboard
//! focus) - so the same physical input never acts in the UI and the viewport at once. The fly
//! camera keeps right-mouse-hold to look, which leaves the cursor free for the UI by
//! construction. The left button picks, places, and (on the already-selected placement) drags to
//! move, with `crate::drag` owning the drag math. Every authored-model change is emitted as an
//! [`Action`] for the frame loop to apply, never written here, so the loop stays the model's
//! single writer; only presentation state (place mode, the context menu, the drag) is touched in
//! place.

use glam::{Vec2, Vec3};
use wok_platform::input::InputState;
use wok_platform::winit::event::MouseButton;
use wok_platform::winit::keyboard::{Key, NamedKey};
use wok_scene::Transform;

use crate::camera::{CameraInput, FlyCamera};
use crate::drag::{DragMode, PlacementDrag, drag_offset, dragged_translation};
use crate::model::{EditorModel, chunk_origin};
use crate::panels::{Action, UiState};
use crate::pick;

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

/// Motion (pixels) under which a press-and-release still reads as a click rather than a drag:
/// enough for a twitchy hand, far under any deliberate drag. Shared by the right button (camera
/// look vs context click) and the left button (placement drag vs pick).
const CLICK_SLOP_PX: f32 = 4.0;

/// Hotkeys and viewport clicks, read against the current model and emitted as [`Action`]s for the
/// frame loop to apply, so this routing reads model state and never writes it. Ctrl+S emits `Save`,
/// Ctrl+Z and Ctrl+Shift+Z (or Ctrl+Y) emit `Undo` / `Redo`, Delete emits `Delete`, Esc cancels
/// place mode, then an open context menu, then emits
/// `Select(None)` to deselect. A left click emits `Place` (in place mode) or `Select` on what it
/// picks; a left press on the already-selected placement arms a drag that emits `Edit` once it
/// crosses the slop (Shift moves vertically); a right click (a clean press-release - dragging is
/// camera look) selects and opens the context menu on what it hit. Presentation state - place mode,
/// the context menu, the right-drag accumulator, the scroll-to flag - is mutated here in place;
/// only authored model changes go through actions. Ctrl+S deliberately ignores `keys_free`: saving
/// must work mid-edit in a details field; undo and redo, by contrast, honor it, leaving a focused
/// field egui's own in-field undo.
pub fn handle(
    input: &InputState,
    pointer_free: bool,
    keys_free: bool,
    camera: &FlyCamera,
    size: (u32, u32),
    far: f32,
    model: &EditorModel,
    ui: &mut UiState,
    actions: &mut Vec<Action>,
) {
    if input.key_held(NamedKey::Control) && input.char_pressed('s') {
        actions.push(Action::Save);
    }
    // Undo/redo, gated on keys_free so a focused text field keeps egui's own in-field undo (Ctrl+S
    // above is the deliberate exception). char_pressed is case-insensitive, so Shift only picks the
    // direction: Ctrl+Z undoes, Ctrl+Shift+Z and Ctrl+Y both redo.
    if keys_free && input.key_held(NamedKey::Control) {
        if input.char_pressed('z') {
            actions.push(if input.key_held(NamedKey::Shift) { Action::Redo } else { Action::Undo });
        }
        if input.char_pressed('y') {
            actions.push(Action::Redo);
        }
    }
    if keys_free && input.key_pressed(NamedKey::Delete)
        && let Some(sel) = model.selection
    {
        actions.push(Action::Delete(sel));
    }
    if keys_free && input.key_pressed(NamedKey::Escape) {
        if ui.placing.is_some() {
            ui.placing = None;
        } else if ui.context_menu.is_some() {
            ui.context_menu = None;
        } else {
            actions.push(Action::Select(None));
        }
    }

    // Telling a look-drag from a context click: accumulate raw motion across the whole right
    // hold; a release that never really moved is the click.
    if input.mouse_pressed(MouseButton::Right) {
        ui.right_drag_px = 0.0;
    }
    if input.mouse_held(MouseButton::Right) {
        ui.right_drag_px += Vec2::new(input.mouse_motion.0 as f32, input.mouse_motion.1 as f32).length();
    }

    let viewport = Vec2::new(size.0 as f32, size.1.max(1) as f32);
    let cursor = Vec2::new(input.mouse_pos.0 as f32, input.mouse_pos.1 as f32);
    let ray = |camera: &FlyCamera| {
        let view_proj = camera.view_proj(viewport.x / viewport.y, far);
        pick::cursor_ray(view_proj, camera.position, cursor, viewport)
    };

    if pointer_free && input.mouse_pressed(MouseButton::Left) {
        ui.context_menu = None;
        let Some(dir) = ray(camera) else { return };
        if let Some(prefab) = ui.placing.take() {
            match pick::terrain_hit(&model.heightmaps, camera.position, dir, far) {
                Some(hit) => actions.push(Action::Place { prefab, point: hit.point }),
                // No terrain under the click: stay armed instead of silently dropping the mode.
                None => ui.placing = Some(prefab),
            }
        } else {
            let picked =
                pick::pick(&model.chunks, &model.prefabs, &model.heightmaps, camera.position, dir, far);
            if picked.is_some() && picked == model.selection {
                // A press on the already-selected placement arms a drag instead of re-picking:
                // past the slop it moves the placement; released under it, it was a click on
                // what is already selected, which changes nothing.
                ui.drag =
                    picked.map(|sel| PlacementDrag { sel, press_px: cursor, active: false, anchor: None });
            } else {
                actions.push(Action::Select(picked));
                // A viewport selection brings its tree row into view.
                ui.scroll_to_selection = picked.is_some();
            }
        }
    }

    // An armed placement drag advances while the left button stays down and ends with it. The
    // selection can vanish mid-drag (Esc, Delete, an external reload); the drag dies with it.
    // Once started, the drag deliberately ignores `pointer_free`: it owns the pointer even when
    // the cursor crosses a panel, exactly as a panel widget's own drag would over the viewport.
    if let Some(mut drag) = ui.drag.take()
        && input.mouse_held(MouseButton::Left)
        && model.selection == Some(drag.sel)
    {
        if let Some(dir) = ray(camera) {
            drag_selected(input, camera.position, dir, far, model, &mut drag, cursor, actions);
        }
        ui.drag = Some(drag);
    }

    if pointer_free
        && input.mouse_buttons_released.contains(&MouseButton::Right)
        && ui.right_drag_px < CLICK_SLOP_PX
    {
        let Some(dir) = ray(camera) else { return };
        let picked = pick::pick(&model.chunks, &model.prefabs, &model.heightmaps, camera.position, dir, far);
        if picked.is_some() {
            actions.push(Action::Select(picked));
            ui.scroll_to_selection = true;
            ui.context_menu = Some((cursor.x, cursor.y));
        } else {
            ui.context_menu = None;
        }
    }
}

/// One held frame of a placement drag: enforce the slop, pick the mode by Shift (re-anchoring
/// whenever the mode is entered, so the placement never jumps to the cursor), and emit the frame's
/// translation as an [`Action::Edit`] - the same authored-form edit the details panel makes, so
/// dirty tracking and the unsaved indicator follow for free. A frame whose cursor gives nothing to
/// track (no terrain under it, a degenerate ray) holds the placement still.
fn drag_selected(
    input: &InputState,
    eye: Vec3,
    dir: Vec3,
    far: f32,
    model: &EditorModel,
    drag: &mut PlacementDrag,
    cursor: Vec2,
    actions: &mut Vec<Action>,
) {
    if !drag.active {
        if (cursor - drag.press_px).length() < CLICK_SLOP_PX {
            return;
        }
        drag.active = true;
    }
    let Some(placement) = model.placement(drag.sel) else { return };
    let current = placement.transform;
    let state = placement.state.clone();
    let Some(prefab) = model.prefabs.get(&placement.prefab) else { return };
    let origin = chunk_origin(drag.sel.coord);

    let mode = if input.key_held(NamedKey::Shift) { DragMode::Vertical } else { DragMode::Terrain };
    if drag.anchor.map(|(m, _)| m) != Some(mode) {
        drag.anchor = drag_offset(mode, &current, origin, &model.heightmaps, eye, dir, far)
            .map(|offset| (mode, offset));
    }
    let Some((_, offset)) = drag.anchor else { return };
    let Some(translation) =
        dragged_translation(mode, offset, &current, origin, prefab, &model.heightmaps, eye, dir, far)
    else {
        return;
    };
    if translation != current.translation {
        actions.push(Action::Edit { sel: drag.sel, transform: Transform { translation, ..current }, state });
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

    use crate::model::Selection;
    use crate::sample;
    use wok_scene::{ChunkCoord, InstanceId, PrefabRef};

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

    // ---- viewport input emits actions (the single-writer pin) ----

    fn blank_input() -> InputState {
        InputState {
            keys_held: HashSet::new(),
            keys_pressed: HashSet::new(),
            keys_released: HashSet::new(),
            mouse_pos: (0.0, 0.0),
            mouse_delta: (0.0, 0.0),
            mouse_motion: (0.0, 0.0),
            mouse_buttons_held: HashSet::new(),
            mouse_buttons_pressed: HashSet::new(),
            mouse_buttons_released: HashSet::new(),
            scroll_delta: (0.0, 0.0),
            gamepads: vec![],
        }
    }

    fn sample_model() -> EditorModel {
        let content = sample::build();
        EditorModel::new(
            content.scene,
            content.prefabs.into_iter().collect(),
            vec![(content.chunk, Some(content.heightmap))],
        )
        .expect("sample content loads")
    }

    /// A camera at `eye` looking straight at `target`, so a screen-centre cursor rays exactly along
    /// that line (the centre-ray-is-forward property `crate::pick` pins). Keep `target` off the
    /// camera's vertical line: looking along world up is the look-matrix singularity.
    fn looking_from_at(eye: Vec3, target: Vec3) -> FlyCamera {
        let d = (target - eye).normalize();
        FlyCamera { position: eye, yaw: d.x.atan2(-d.z), pitch: d.y.asin(), speed: 16.0 }
    }

    /// A camera for the keyboard-only cases, where no ray is cast.
    fn any_camera() -> FlyCamera {
        FlyCamera { position: Vec3::new(64.0, 40.0, 100.0), yaw: 0.0, pitch: -0.6, speed: 16.0 }
    }

    /// Run `handle` with the pointer and keys free (egui claims nothing) over an 800x600 viewport,
    /// returning the actions it emitted. A centred `mouse_pos` then rays along the camera forward.
    fn emitted(input: &InputState, camera: &FlyCamera, model: &EditorModel, ui: &mut UiState) -> Vec<Action> {
        let mut actions = Vec::new();
        handle(input, true, true, camera, (800, 600), 500.0, model, ui, &mut actions);
        actions
    }

    /// Screen centre for the viewport `emitted` uses; a centred cursor rays along forward.
    const CENTER_CURSOR: (f64, f64) = (400.0, 300.0);

    #[test]
    fn ctrl_s_emits_save() {
        let model = sample_model();
        let mut ui = UiState::default();
        let mut input = blank_input();
        input.keys_held.insert(Key::Named(NamedKey::Control));
        input.keys_pressed.insert(Key::Character("s".into()));
        assert_eq!(emitted(&input, &any_camera(), &model, &mut ui), vec![Action::Save]);
    }

    #[test]
    fn delete_emits_delete_of_the_current_selection() {
        let mut model = sample_model();
        let sel = Selection { coord: ChunkCoord::new(0, 0), id: InstanceId(2) };
        model.selection = Some(sel);
        let mut ui = UiState::default();
        let mut input = blank_input();
        input.keys_pressed.insert(Key::Named(NamedKey::Delete));
        assert_eq!(emitted(&input, &any_camera(), &model, &mut ui), vec![Action::Delete(sel)]);
    }

    #[test]
    fn escape_with_nothing_to_cancel_emits_deselect() {
        let mut model = sample_model();
        model.selection = Some(Selection { coord: ChunkCoord::new(0, 0), id: InstanceId(0) });
        let mut ui = UiState::default();
        let mut input = blank_input();
        input.keys_pressed.insert(Key::Named(NamedKey::Escape));
        assert_eq!(emitted(&input, &any_camera(), &model, &mut ui), vec![Action::Select(None)]);
    }

    #[test]
    fn escape_in_place_mode_cancels_in_place_without_an_action() {
        // Place-mode cancel is presentation, not an authored change: it clears UiState and emits
        // nothing, exactly as before.
        let model = sample_model();
        let mut ui = UiState { placing: Some(PrefabRef::new("crate")), ..UiState::default() };
        let mut input = blank_input();
        input.keys_pressed.insert(Key::Named(NamedKey::Escape));
        assert!(emitted(&input, &any_camera(), &model, &mut ui).is_empty());
        assert!(ui.placing.is_none(), "Esc disarmed place mode");
    }

    #[test]
    fn left_click_in_place_mode_over_terrain_emits_place_and_consumes_the_mode() {
        let model = sample_model();
        let mut ui = UiState { placing: Some(PrefabRef::new("crate")), ..UiState::default() };
        // Aim down into the chunk interior: the centre ray meets terrain.
        let cam = looking_from_at(Vec3::new(64.0, 40.0, 100.0), Vec3::new(64.0, 0.0, 64.0));
        let mut input = blank_input();
        input.mouse_pos = CENTER_CURSOR;
        input.mouse_buttons_pressed.insert(MouseButton::Left);

        let actions = emitted(&input, &cam, &model, &mut ui);
        assert_eq!(actions.len(), 1, "one click, one place");
        match &actions[0] {
            Action::Place { prefab, point } => {
                assert_eq!(*prefab, PrefabRef::new("crate"));
                assert!(point.is_finite(), "placed at a real terrain point: {point:?}");
            }
            other => panic!("expected Place, got {other:?}"),
        }
        assert!(ui.placing.is_none(), "a hit consumes place mode");
    }

    #[test]
    fn left_click_in_place_mode_off_terrain_stays_armed_and_emits_nothing() {
        let model = sample_model();
        let mut ui = UiState { placing: Some(PrefabRef::new("crate")), ..UiState::default() };
        // Aim up and away from the loaded chunk: the ray never meets terrain.
        let cam = looking_from_at(Vec3::new(64.0, 30.0, 64.0), Vec3::new(64.0, 60.0, 30.0));
        let mut input = blank_input();
        input.mouse_pos = CENTER_CURSOR;
        input.mouse_buttons_pressed.insert(MouseButton::Left);

        assert!(emitted(&input, &cam, &model, &mut ui).is_empty(), "no terrain, nothing placed");
        assert_eq!(ui.placing, Some(PrefabRef::new("crate")), "place mode stays armed on a miss");
    }

    #[test]
    fn left_click_on_a_placement_emits_select() {
        let model = sample_model();
        // The first pillar (chunk-local 60, 70): aim at its rested world centre from above and to
        // the south, so the ray meets it before any terrain and no other placement is in the way.
        let coord = ChunkCoord::new(0, 0);
        let pillar = model.chunks[&coord]
            .placements
            .iter()
            .find(|p| p.prefab.as_str() == "pillar")
            .expect("a pillar in the sample");
        let sel = Selection { coord, id: pillar.instance_id };
        let target = chunk_origin(coord) + pillar.transform.translation;
        let cam = looking_from_at(target + Vec3::new(0.0, 30.0, 30.0), target);

        let mut ui = UiState::default();
        let mut input = blank_input();
        input.mouse_pos = CENTER_CURSOR;
        input.mouse_buttons_pressed.insert(MouseButton::Left);

        assert_eq!(emitted(&input, &cam, &model, &mut ui), vec![Action::Select(Some(sel))]);
        assert!(ui.scroll_to_selection, "a viewport pick brings its tree row into view");
    }

    #[test]
    fn an_active_drag_emits_edit_when_the_cursor_moves_the_placement() {
        let mut model = sample_model();
        let coord = ChunkCoord::new(0, 0);
        let sel = {
            let boulder = model.chunks[&coord]
                .placements
                .iter()
                .find(|p| p.prefab.as_str() == "boulder")
                .expect("a boulder in the sample");
            Selection { coord, id: boulder.instance_id }
        };
        model.selection = Some(sel);
        // An already-active drag anchored at a zero grab offset: the dragged spot is the cursor's
        // terrain hit itself. Aiming at the chunk centre puts that hit well away from the boulder's
        // authored position, so the frame moves it and must emit an Edit.
        let drag = PlacementDrag {
            sel,
            press_px: Vec2::ZERO,
            active: true,
            anchor: Some((DragMode::Terrain, Vec3::ZERO)),
        };
        let mut ui = UiState { drag: Some(drag), ..UiState::default() };
        let cam = looking_from_at(Vec3::new(64.0, 40.0, 100.0), Vec3::new(64.0, 0.0, 64.0));
        let mut input = blank_input();
        input.mouse_pos = CENTER_CURSOR;
        // Held, not pressed: the drag branch advances, the press-only re-pick branch does not.
        input.mouse_buttons_held.insert(MouseButton::Left);

        let actions = emitted(&input, &cam, &model, &mut ui);
        assert_eq!(actions.len(), 1, "one moved frame, one edit");
        match &actions[0] {
            Action::Edit { sel: edited, .. } => assert_eq!(*edited, sel),
            other => panic!("expected Edit, got {other:?}"),
        }
    }

    // ---- undo/redo hotkeys ----

    #[test]
    fn ctrl_z_undoes_and_ctrl_shift_z_or_ctrl_y_redo_when_keys_are_free() {
        let model = sample_model();
        let mut ui = UiState::default();
        let chord = |held: &[NamedKey], ch: &str| {
            let mut input = blank_input();
            for &k in held {
                input.keys_held.insert(Key::Named(k));
            }
            input.keys_pressed.insert(Key::Character(ch.into()));
            input
        };

        let ctrl_z = chord(&[NamedKey::Control], "z");
        assert_eq!(emitted(&ctrl_z, &any_camera(), &model, &mut ui), vec![Action::Undo]);

        let ctrl_shift_z = chord(&[NamedKey::Control, NamedKey::Shift], "z");
        assert_eq!(emitted(&ctrl_shift_z, &any_camera(), &model, &mut ui), vec![Action::Redo]);

        let ctrl_y = chord(&[NamedKey::Control], "y");
        assert_eq!(emitted(&ctrl_y, &any_camera(), &model, &mut ui), vec![Action::Redo]);
    }

    #[test]
    fn undo_redo_keys_are_suppressed_while_a_field_has_focus() {
        // keys_free = false stands in for a focused text field: Ctrl+Z must stay egui's in-field
        // undo and never reach the editor.
        let model = sample_model();
        let mut ui = UiState::default();
        let mut input = blank_input();
        input.keys_held.insert(Key::Named(NamedKey::Control));
        input.keys_pressed.insert(Key::Character("z".into()));
        let mut actions = Vec::new();
        handle(&input, true, false, &any_camera(), (800, 600), 500.0, &model, &mut ui, &mut actions);
        assert!(actions.is_empty(), "Ctrl+Z is suppressed while a field has focus");
    }
}
