//! Object-mode keyboard movement: the home row nudges the whole selection by grid steps.
//!
//! The modal manipulator's left hand (see designs/editor-design.md, Input): in object mode the
//! mouse selects and the keyboard moves, so a left press never repositions (that gesture is
//! retired) - instead each home-row tap shifts the whole selection one grid step along a world
//! axis. It emits the same [`Action::MoveSelection`] the inspector's multi-position edit and the
//! retired drag used, so a burst of taps coalesces into one undo step through the existing
//! transform run (`crate::history`).
//!
//! Modal by construction: free-fly flies the camera on these same letters (`super::camera`), so
//! `super::viewport` calls here only in object mode and only when the keys are free (a focused
//! field types them instead). World-axis steps for now; camera-relative nudges and the count
//! multiplier are a later slice.

use glam::Vec3;
use wok_platform::input::InputState;

use crate::model::EditorModel;
use crate::panels::Action;

/// One world-axis grid step, in metres: how far a single home-row tap nudges the selection. The
/// unit step the count multiplier (a later slice) will scale.
const GRID_STEP_M: f32 = 1.0;

/// The home-row nudge bindings, paired `(positive, negative)` per world axis. Tunable: remap by
/// changing these six letters. The defaults keep the hand on the home row - f/s push +X/-X, e/d
/// push +Z/-Z, r/v push +Y/-Y - and reuse the WASD letters that are inert as camera keys in object
/// mode, so the same physical key means "fly" in free-fly and "nudge" here.
const NUDGE_X: (char, char) = ('f', 's');
const NUDGE_Y: (char, char) = ('r', 'v');
const NUDGE_Z: (char, char) = ('e', 'd');

/// Object-mode home-row input: each axis whose positive or negative key was tapped this frame
/// contributes one grid step, summed into a single [`Action::MoveSelection`] - so opposed taps
/// cancel and a multi-key frame is still one move. One tap is one step: the bindings read
/// `InputState::char_pressed`, which is edge-triggered, so holding a key does not repeat. Nothing
/// is emitted with an empty selection or no tapped key. The caller gates mode and focus; this only
/// decides what the keys do.
pub(crate) fn handle(input: &InputState, model: &EditorModel, actions: &mut Vec<Action>) {
    if model.selection.is_empty() {
        return;
    }
    let tapped =
        |(pos, neg): (char, char)| f32::from(input.char_pressed(pos)) - f32::from(input.char_pressed(neg));
    let delta = Vec3::new(tapped(NUDGE_X), tapped(NUDGE_Y), tapped(NUDGE_Z)) * GRID_STEP_M;
    if delta != Vec3::ZERO {
        actions.push(Action::MoveSelection { delta });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wok_platform::winit::keyboard::Key;
    use wok_scene::{ChunkCoord, InstanceId};

    use crate::input::test_support::{any_camera, blank_input, emitted, sample_model};
    use crate::mode::Mode;
    use crate::model::Selection;
    use crate::panels::UiState;

    /// A one-frame input with `ch` tapped (pressed this frame), the edge `char_pressed` reads.
    fn tap(ch: &str) -> InputState {
        let mut input = blank_input();
        input.keys_pressed.insert(Key::Character(ch.into()));
        input
    }

    /// The sample's first placement, selected: the group the nudge acts on.
    fn select_one(model: &mut EditorModel) -> Selection {
        let sel = Selection { coord: ChunkCoord::new(0, 0), id: InstanceId(0) };
        model.selection.replace(sel);
        sel
    }

    #[test]
    fn a_home_row_key_in_object_mode_nudges_the_selection_one_grid_step() {
        let mut model = sample_model();
        select_one(&mut model);
        let mut ui = UiState::default(); // Object is the default mode
        // 'f' is the +X binding: one tap is one grid step along +X, no rotation or scale.
        let actions = emitted(&tap("f"), &any_camera(), &model, &mut ui);
        assert_eq!(actions, vec![Action::MoveSelection { delta: Vec3::X * GRID_STEP_M }]);
    }

    #[test]
    fn each_binding_pushes_its_world_axis_by_one_step() {
        let mut model = sample_model();
        select_one(&mut model);
        let mut ui = UiState::default();
        let cases = [
            ("f", Vec3::X), ("s", Vec3::NEG_X),
            ("r", Vec3::Y), ("v", Vec3::NEG_Y),
            ("e", Vec3::Z), ("d", Vec3::NEG_Z),
        ];
        for (key, dir) in cases {
            let actions = emitted(&tap(key), &any_camera(), &model, &mut ui);
            assert_eq!(
                actions,
                vec![Action::MoveSelection { delta: dir * GRID_STEP_M }],
                "{key} nudges by {dir:?}",
            );
        }
    }

    #[test]
    fn the_home_row_does_nothing_in_free_fly() {
        // In free-fly these letters fly the camera (in `advance_camera`, not the input routing);
        // through the viewport routing they emit no action - the nudge is object-mode only.
        let mut model = sample_model();
        select_one(&mut model);
        let mut ui = UiState { mode: Mode::FreeFly, ..UiState::default() };
        for key in ["f", "s", "e", "d", "r", "v"] {
            assert!(emitted(&tap(key), &any_camera(), &model, &mut ui).is_empty(), "{key} is inert in free-fly");
        }
    }

    #[test]
    fn a_nudge_with_no_selection_emits_nothing() {
        // Nothing is the cursor, so the verb has no target: the key is inert, not a no-op move.
        let model = sample_model(); // empty selection
        let mut ui = UiState::default();
        assert!(emitted(&tap("f"), &any_camera(), &model, &mut ui).is_empty(), "no selection, no nudge");
    }

    #[test]
    fn opposed_nudge_keys_on_one_frame_cancel_to_no_move() {
        let mut model = sample_model();
        select_one(&mut model);
        let mut ui = UiState::default();
        // +X and -X tapped the same frame sum to zero on the axis: no MoveSelection at all (the
        // delta is one summed vector, not two separate moves).
        let mut input = blank_input();
        input.keys_pressed.insert(Key::Character("f".into()));
        input.keys_pressed.insert(Key::Character("s".into()));
        assert!(emitted(&input, &any_camera(), &model, &mut ui).is_empty(), "opposed taps cancel");
    }

    #[test]
    fn a_burst_of_nudges_coalesces_into_one_undo_step() {
        let mut model = sample_model();
        let sel = select_one(&mut model);
        let before = model.placement(sel).unwrap().transform.translation;
        let mut ui = UiState::default();

        // Five frames, each a fresh +X tap: the routing emits one MoveSelection per frame, applied
        // through the writer's checkpoint exactly as the frame loop does. Consecutive moves share
        // one transform run (`crate::history`), so the burst is a single undo step.
        for _ in 0..5 {
            let delta = match emitted(&tap("f"), &any_camera(), &model, &mut ui).as_slice() {
                [Action::MoveSelection { delta }] => *delta,
                other => panic!("each frame nudges once: {other:?}"),
            };
            model.checkpoint(&Action::MoveSelection { delta });
            model.move_selection(delta).unwrap();
        }
        assert_eq!(
            model.placement(sel).unwrap().transform.translation,
            before + Vec3::X * GRID_STEP_M * 5.0,
            "five steps moved five grid units",
        );

        assert!(model.undo().unwrap());
        assert_eq!(model.placement(sel).unwrap().transform.translation, before, "one undo rewinds the burst");
        assert!(!model.undo().unwrap(), "the burst was a single recorded step");
    }
}
