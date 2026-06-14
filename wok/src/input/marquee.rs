//! The viewport marquee: a left-drag begun over empty space or an unselected placement draws a
//! selection box and selects every placement inside it on release.
//!
//! `crate::input::viewport` owns the press that arms the marquee (a press on a *selected* member
//! arms the reposition drag instead) and calls [`step`] each later frame. The box resolves on
//! release, not per frame: dragged past the click slop it is an area select that emits
//! [`Action::SelectMany`] (replacing the selection, or extending it while Ctrl is held); released
//! still under the slop it was a click, and keeps the single-click behavior - a plain click
//! replace-selects what it hit (clearing on empty), Ctrl+click toggles that placement (a no-op on
//! empty). The rect-vs-world test is `crate::pick::pick_rect`; this module owns only the
//! press/drag/release lifecycle and which action falls out of it.
//!
//! Like the reposition drag, the marquee owns the pointer once armed: [`step`] ignores
//! `pointer_free`, so dragging the box across a panel does not break it, exactly as a panel
//! widget's own drag would behave over the viewport.

use glam::Vec2;
use wok_platform::input::InputState;
use wok_platform::winit::event::MouseButton;
use wok_platform::winit::keyboard::NamedKey;

use crate::camera::FlyCamera;
use crate::model::EditorModel;
use crate::panels::{Action, UiState};
use crate::pick;

use super::CLICK_SLOP_PX;

/// A left-button area-select drag, from a press on empty or unselected space to release. Owned by
/// `UiState` (interaction state, like the reposition drag) and advanced by [`step`] each frame the
/// button stays down; the enclosed set is computed once, on release.
pub struct Marquee {
    /// Cursor at press, physical pixels: one corner of the box and the anchor the slop measures
    /// from.
    pub start_px: Vec2,
    /// Latest cursor, physical pixels: the opposite corner, refreshed every frame. The UI draws
    /// the box from `start_px` to here.
    pub current_px: Vec2,
    /// Past the slop and dragging a box; until then a release is a click, not a box. Latched, so a
    /// drag that wanders back under the slop stays a box rather than flipping to a click.
    pub active: bool,
}

impl Marquee {
    /// Arm a marquee at the press position. The opposite corner starts coincident and the box is
    /// inactive (a release here is a click) until the cursor crosses the slop.
    pub fn new(start_px: Vec2) -> Marquee {
        Marquee { start_px, current_px: start_px, active: false }
    }
}

/// Advance an armed marquee, or resolve it on release. Held: refresh the far corner and latch
/// `active` once the drag crosses the slop, leaving the marquee armed for the next frame. Released:
/// drop the marquee and emit the action it resolves to - [`Action::SelectMany`] for an active box
/// (extend when Ctrl is held, else replace), or the deferred single-click action under the slop
/// (re-picking at the release cursor, so a click behaves exactly as it did before the marquee). A
/// no-op when nothing is armed.
pub(crate) fn step(
    input: &InputState,
    camera: &FlyCamera,
    size: (u32, u32),
    far: f32,
    model: &EditorModel,
    ui: &mut UiState,
    cursor: Vec2,
    actions: &mut Vec<Action>,
) {
    let Some(mut marquee) = ui.marquee.take() else { return };
    marquee.current_px = cursor;

    if input.mouse_held(MouseButton::Left) {
        marquee.active |= (marquee.current_px - marquee.start_px).length() >= CLICK_SLOP_PX;
        ui.marquee = Some(marquee);
        return;
    }

    // Released. Ctrl at release picks extend vs replace for the box, and toggle vs the plain click.
    let ctrl = input.key_held(NamedKey::Control);
    let viewport = Vec2::new(size.0 as f32, size.1.max(1) as f32);
    let view_proj = camera.view_proj(viewport.x / viewport.y, far);

    if marquee.active {
        let items =
            pick::pick_rect(&model.chunks, view_proj, viewport, marquee.start_px, marquee.current_px);
        ui.scroll_to_selection = !items.is_empty();
        actions.push(Action::SelectMany { items, add: ctrl });
        return;
    }

    // Under the slop: this press-release was a click. Re-pick at the release cursor and keep the
    // pre-marquee behavior - plain click replace-selects (clears on empty), Ctrl+click toggles.
    let Some(dir) = pick::cursor_ray(view_proj, camera.position, cursor, viewport) else { return };
    let picked = pick::pick(&model.chunks, &model.prefabs, &model.heightmaps, camera.position, dir, far);
    if ctrl {
        if let Some(sel) = picked {
            actions.push(Action::ToggleSelect(sel));
            ui.scroll_to_selection = true;
        }
    } else {
        actions.push(Action::Select(picked));
        ui.scroll_to_selection = picked.is_some();
    }
}

#[cfg(test)]
mod tests {
    use glam::Vec3;
    use wok_platform::winit::event::MouseButton;
    use wok_scene::{ChunkCoord, InstanceId};

    use crate::input::test_support::{
        aim_at_pillar, blank_input, emitted, looking_from_at, marquee_dragged, sample_model,
    };
    use crate::model::Selection;
    use crate::panels::{Action, UiState};

    #[test]
    fn a_press_on_empty_space_arms_a_marquee_without_selecting() {
        // Empty selection, so the press cannot land on a selected member: it arms a marquee, and
        // arming emits no action (the box resolves on release, not press).
        let model = sample_model();
        let (_, cam) = aim_at_pillar(&model);
        let mut ui = UiState::default();
        let mut input = blank_input();
        input.mouse_pos = (5.0, 5.0); // a screen corner, off any placement
        input.mouse_buttons_pressed.insert(MouseButton::Left);
        input.mouse_buttons_held.insert(MouseButton::Left);

        assert!(emitted(&input, &cam, &model, &mut ui).is_empty(), "arming a marquee emits no action");
        let marquee = ui.marquee.as_ref().expect("the press armed a marquee");
        assert!(!marquee.active, "still under the slop, not yet a box");
    }

    #[test]
    fn a_marquee_past_the_slop_replace_selects_the_enclosed_placements() {
        let model = sample_model(); // empty selection: the press arms a marquee, never a reposition
        let (_, cam) = aim_at_pillar(&model);
        let mut ui = UiState::default();
        // A box over the whole viewport encloses the placements the camera sees.
        let actions = marquee_dragged(&cam, &model, &mut ui, (5.0, 5.0), (795.0, 595.0), false);
        match actions.as_slice() {
            // add = false is the replace verb (selection.rs proves extend(.., false) replaces).
            [Action::SelectMany { items, add: false }] => {
                assert!(!items.is_empty(), "the full-screen box caught the visible placements");
            }
            other => panic!("expected one replace SelectMany, got {other:?}"),
        }
        assert!(ui.scroll_to_selection, "a non-empty box brings the primary into view");
    }

    #[test]
    fn a_ctrl_marquee_past_the_slop_extends_with_the_enclosed_placements() {
        let model = sample_model();
        let (_, cam) = aim_at_pillar(&model);
        let mut ui = UiState::default();
        let actions = marquee_dragged(&cam, &model, &mut ui, (5.0, 5.0), (795.0, 595.0), true);
        match actions.as_slice() {
            // Ctrl at release sets add = true: the model extends rather than replaces (the extend
            // semantics themselves are covered by `crate::selection`).
            [Action::SelectMany { items, add: true }] => assert!(!items.is_empty()),
            other => panic!("expected one extend SelectMany, got {other:?}"),
        }
    }

    #[test]
    fn a_marquee_over_empty_space_clears_a_plain_set_and_leaves_a_ctrl_set() {
        let mut model = sample_model();
        let coord = ChunkCoord::new(0, 0);
        let (a, b) = (Selection { coord, id: InstanceId(0) }, Selection { coord, id: InstanceId(2) });
        model.selection.toggle(a);
        model.selection.toggle(b);
        let mut ui = UiState::default();
        // Looking away from the chunk: every placement sits behind the camera, so a full box over
        // the screen catches nothing.
        let away = looking_from_at(Vec3::new(64.0, 10.0, 300.0), Vec3::new(64.0, 10.0, 500.0));

        // Plain box over nothing: SelectMany with no items and add = false; applied, it clears.
        let plain = marquee_dragged(&away, &model, &mut ui, (10.0, 10.0), (790.0, 590.0), false);
        match plain.as_slice() {
            [Action::SelectMany { items, add: false }] => {
                assert!(items.is_empty(), "the box over empty space caught nothing");
                let mut set = model.selection.clone();
                set.extend(items.iter().copied(), false);
                assert!(set.is_empty(), "a plain empty box clears the selection");
            }
            other => panic!("expected an empty replace SelectMany, got {other:?}"),
        }

        // Ctrl box over nothing: add = true; applied, the set is left intact.
        let ctrl = marquee_dragged(&away, &model, &mut ui, (10.0, 10.0), (790.0, 590.0), true);
        match ctrl.as_slice() {
            [Action::SelectMany { items, add: true }] => {
                assert!(items.is_empty());
                let mut set = model.selection.clone();
                set.extend(items.iter().copied(), true);
                assert!(set.contains(a) && set.contains(b), "a Ctrl empty box leaves the set intact");
            }
            other => panic!("expected an empty extend SelectMany, got {other:?}"),
        }
    }
}
