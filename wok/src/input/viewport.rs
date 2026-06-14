//! Viewport input routing: hotkeys, left-click pick/place, the placement drag, and the area
//! marquee, read against the model and emitted as [`Action`]s for the frame loop to apply (never
//! written here, so the loop stays the model's single writer). `crate::input` covers the egui
//! focus-gating these honor; `super::reposition` and `super::marquee` own the two left-drag gestures.

use glam::Vec2;
use wok_platform::input::InputState;
use wok_platform::winit::event::MouseButton;
use wok_platform::winit::keyboard::NamedKey;

use crate::camera::FlyCamera;
use crate::drag::PlacementDrag;
use crate::model::EditorModel;
use crate::panels::{Action, UiState};
use crate::pick;

use super::{CLICK_SLOP_PX, marquee, reposition};

/// Hotkeys and viewport clicks, read against the current model and emitted as [`Action`]s for the
/// frame loop to apply, so this routing reads model state and never writes it. Ctrl+S emits `Save`,
/// Ctrl+Z and Ctrl+Shift+Z (or Ctrl+Y) emit `Undo` / `Redo`, Delete emits `Delete`, Esc cancels
/// place mode, then an open context menu, then emits
/// `Select(None)` to deselect. In place mode a left click emits `Place`. Otherwise a left press on
/// any selected placement arms a reposition drag (the whole selection moves once it crosses the
/// slop, Shift vertically); a left press anywhere else arms a marquee that resolves on release -
/// past the slop a box `SelectMany` (Ctrl extends, else replaces), under it the click it always
/// was (`Select` on the pick or empty, `ToggleSelect` under Ctrl). A right click (a clean
/// press-release - dragging is camera look) selects and opens the context menu on what it hit.
/// Presentation state - place mode,
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
    if keys_free && input.key_pressed(NamedKey::Delete) && !model.selection.is_empty() {
        actions.push(Action::Delete);
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
            let ctrl = input.key_held(NamedKey::Control);
            if let Some(sel) = picked.filter(|&sel| !ctrl && model.selection.contains(sel)) {
                // A plain press on any selected placement arms a reposition drag instead of
                // re-picking: past the slop the whole selection moves rigidly with the grabbed
                // member; released under it, nothing changes.
                ui.drag = Some(PlacementDrag { sel, press_px: cursor, active: false, anchor: None });
            } else {
                // A press anywhere else - empty space, or an unselected placement it passes over
                // rather than grabs - arms a marquee. The click-vs-box and plain-vs-Ctrl decisions
                // are deferred to release (`marquee::step`): at press time we cannot yet tell a
                // click from the start of a box.
                ui.marquee = Some(marquee::Marquee::new(cursor));
            }
        }
    }

    // An armed placement drag advances while the left button stays down and ends with it. The
    // selection can vanish mid-drag (Esc, Delete, an external reload); the drag dies with it.
    // Once started, the drag deliberately ignores `pointer_free`: it owns the pointer even when
    // the cursor crosses a panel, exactly as a panel widget's own drag would over the viewport.
    if let Some(mut drag) = ui.drag.take()
        && input.mouse_held(MouseButton::Left)
        && model.selection.contains(drag.sel)
    {
        if let Some(dir) = ray(camera) {
            reposition::step(input, camera.position, dir, far, model, &mut drag, cursor, actions);
        }
        ui.drag = Some(drag);
    }

    // An armed marquee advances while the left button stays down and resolves on release - a box
    // `SelectMany` past the slop, or the deferred single click under it. Like the reposition drag
    // it owns the pointer once armed (ignores `pointer_free`), so dragging the box over a panel
    // does not break it. A no-op on frames with no marquee.
    marquee::step(input, camera, size, far, model, ui, cursor, actions);

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

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use wok_platform::winit::keyboard::Key;
    use wok_scene::{ChunkCoord, InstanceId, PrefabRef};

    use crate::drag::DragMode;
    use crate::input::test_support::{
        CENTER_CURSOR, aim_at_pillar, any_camera, blank_input, clicked, emitted, looking_from_at,
        sample_model,
    };
    use crate::model::Selection;

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
    fn delete_emits_a_set_delete_when_something_is_selected() {
        let mut model = sample_model();
        model.selection.replace(Selection { coord: ChunkCoord::new(0, 0), id: InstanceId(2) });
        let mut ui = UiState::default();
        let mut input = blank_input();
        input.keys_pressed.insert(Key::Named(NamedKey::Delete));
        assert_eq!(emitted(&input, &any_camera(), &model, &mut ui), vec![Action::Delete]);
    }

    #[test]
    fn delete_emits_nothing_on_an_empty_selection() {
        // No selection, no checkpoint-worthy action: the key is inert rather than a no-op undo step.
        let model = sample_model();
        let mut ui = UiState::default();
        let mut input = blank_input();
        input.keys_pressed.insert(Key::Named(NamedKey::Delete));
        assert!(emitted(&input, &any_camera(), &model, &mut ui).is_empty());
    }

    #[test]
    fn escape_with_nothing_to_cancel_emits_deselect() {
        let mut model = sample_model();
        model.selection.replace(Selection { coord: ChunkCoord::new(0, 0), id: InstanceId(0) });
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
    fn a_click_under_the_slop_replace_selects_the_pick() {
        let model = sample_model();
        let (sel, cam) = aim_at_pillar(&model);
        let mut ui = UiState::default();
        let mut input = blank_input();
        input.mouse_pos = CENTER_CURSOR;
        // A click is now a press (which arms a marquee) then a release under the slop, where the
        // click resolves: the plain click still replace-selects the pick, exactly as before.
        assert_eq!(clicked(&input, &cam, &model, &mut ui), vec![Action::Select(Some(sel))]);
        assert!(ui.scroll_to_selection, "a viewport pick brings its tree row into view");
    }

    #[test]
    fn a_ctrl_click_under_the_slop_toggles_the_pick() {
        let model = sample_model();
        let (sel, cam) = aim_at_pillar(&model);
        let mut ui = UiState::default();
        let mut input = blank_input();
        input.mouse_pos = CENTER_CURSOR;
        input.keys_held.insert(Key::Named(NamedKey::Control));

        // Ctrl held the whole click: the under-slop release toggles the pick in or out of the set,
        // still a toggle and not a replace-select.
        assert_eq!(clicked(&input, &cam, &model, &mut ui), vec![Action::ToggleSelect(sel)]);
    }

    #[test]
    fn an_active_drag_emits_a_move_when_the_cursor_moves_the_placement() {
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
        model.selection.replace(sel);
        // An already-active drag anchored at a zero grab offset: the dragged spot is the cursor's
        // terrain hit itself. Aiming at the chunk centre puts that hit well away from the boulder's
        // authored position, so the frame moves it and must emit a group move.
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
        assert_eq!(actions.len(), 1, "one moved frame, one move action");
        match &actions[0] {
            Action::MoveSelection { delta } => assert!(*delta != Vec3::ZERO, "the drag moved the group"),
            other => panic!("expected MoveSelection, got {other:?}"),
        }
    }

    #[test]
    fn a_press_on_any_selected_member_arms_the_group_drag() {
        let mut model = sample_model();
        let (grabbed, cam) = aim_at_pillar(&model);
        // A two-member selection; the grabbed pillar is one of several, not the sole selection.
        model.selection.replace(grabbed);
        model.selection.toggle(Selection { coord: ChunkCoord::new(0, 0), id: InstanceId(0) });
        let mut ui = UiState::default();
        let mut input = blank_input();
        input.mouse_pos = CENTER_CURSOR;
        // A real press frame holds the button it just pressed; both flags so the arm survives the
        // same frame's drag-advance pass (which would otherwise take-and-not-restore it).
        input.mouse_buttons_pressed.insert(MouseButton::Left);
        input.mouse_buttons_held.insert(MouseButton::Left);

        // Pressing any selected member arms the group drag (no action yet) rather than re-selecting;
        // the older sole-selection gate would have replace-selected here instead.
        assert!(emitted(&input, &cam, &model, &mut ui).is_empty(), "arming emits no action");
        assert_eq!(ui.drag.as_ref().map(|d| d.sel), Some(grabbed), "the pressed member is grabbed");
    }

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
