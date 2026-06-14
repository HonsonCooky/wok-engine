//! Selection edits: applying one uniform change to every placement in the selection set.
//!
//! A third `impl EditorModel` block beside `crate::model` (construction, place, delete, single
//! edit) and `crate::edit_ops` (duplicate, rename, bounds). These four verbs - move, rotate, scale,
//! and set-state - share one body, `edit_selection`: snapshot the selected placements, mutate each
//! authored form, mark its chunk dirty, and re-transform each affected chunk once. They are the
//! primitive layer the inspector's multi-field editing and the upcoming object-mode verbs both
//! call; structural set ops (delete, duplicate) live with their single forms, not here.
//!
//! Each is a pure authored-form mutation in the HLD's authored -> runtime direction, exactly as the
//! group-drag's `move_selection` already was: a rigid per-member change - translate, premultiply a
//! rotation, multiply a scale, assign a state - never a group-around-centroid transform, so the
//! selection keeps its layout and each member turns or grows about its own origin. The coalescing
//! that makes a transform drag one undo step lives in `crate::history` (the shared transform run);
//! this module only performs the mutations.

use std::collections::BTreeSet;

use glam::{Quat, Vec3};
use wok_content::StoreError;
use wok_scene::Placement;

use crate::model::{EditorModel, Selection};

impl EditorModel {
    /// Apply `edit` to every selected placement's authored form, marking each touched chunk dirty
    /// and re-transforming each affected chunk once (coords deduped). The shared body of the four
    /// selection edits below; a member that no longer resolves is skipped.
    fn edit_selection(&mut self, mut edit: impl FnMut(&mut Placement)) -> Result<(), StoreError> {
        let targets: Vec<Selection> = self.selection.iter().collect();
        let mut affected = BTreeSet::new();
        for sel in targets {
            let Some(chunk) = self.chunks.get_mut(&sel.coord) else { continue };
            let Some(placement) = chunk.placements.iter_mut().find(|p| p.instance_id == sel.id) else {
                continue;
            };
            edit(placement);
            self.dirty_chunks.insert(sel.coord);
            affected.insert(sel.coord);
        }
        for coord in affected {
            self.retransform(coord)?;
        }
        Ok(())
    }

    /// Move every selected placement rigidly by a uniform delta. The viewport group-drag's per-frame
    /// step: the grabbed placement resolves the delta and the whole set follows, keeping its
    /// relative layout. A rigid translate only - rotation, scale, and the group's shape untouched.
    pub fn move_selection(&mut self, delta: Vec3) -> Result<(), StoreError> {
        self.edit_selection(|p| p.transform.translation += delta)
    }

    /// Rotate every selected placement in place by `delta` (rotation = delta * rotation), each about
    /// its own origin - not a shared centroid, so the set keeps its layout while every member turns.
    /// The inspector's multi-selection rotation edit; `delta` is the change from the primary's shown
    /// orientation, so the primary lands on exactly the displayed angles and the rest turn alike.
    pub fn rotate_selection(&mut self, delta: Quat) -> Result<(), StoreError> {
        self.edit_selection(|p| p.transform.rotation = delta * p.transform.rotation)
    }

    /// Scale every selected placement by `factor` (scale *= factor), each about its own origin. The
    /// inspector's multi-selection scale edit; `factor` is the ratio the primary's shown scale
    /// changed by, so members keep their own proportions while the whole set grows or shrinks.
    pub fn scale_selection(&mut self, factor: f32) -> Result<(), StoreError> {
        self.edit_selection(|p| p.transform.scale *= factor)
    }

    /// Set every selected placement's resolved state (the inspector's multi-selection state combo).
    /// `None` is the implicit default, exactly as the slicer resolves it. A discrete edit, not a
    /// drag, so it is one undo step on its own (`crate::history`). Borrows the name and owns a copy
    /// per member (each `Placement` holds its own `String`).
    pub fn set_state_selection(&mut self, state: Option<&str>) -> Result<(), StoreError> {
        self.edit_selection(|p| p.state = state.map(str::to_owned))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panels::Action;
    use crate::sample;
    use wok_scene::{ChunkCoord, InstanceId, PrefabState, Transform};

    fn sample_model() -> EditorModel {
        let content = sample::build();
        EditorModel::new(
            content.scene,
            content.prefabs.into_iter().collect(),
            vec![(content.chunk, Some(content.heightmap))],
        )
        .expect("sample content loads")
    }

    /// The sample's prefabs are single-state; give each an extra "open" state (a clone of default)
    /// so a multi-selection state set has a valid target for every member, whatever its prefab.
    fn sample_model_with_open_state() -> EditorModel {
        let mut model = sample_model();
        for prefab in model.prefabs.values_mut() {
            let open = PrefabState { name: "open".to_string(), ..prefab.states[0].clone() };
            prefab.states.push(open);
        }
        model
    }

    /// Select three members of the sample's one chunk; the group the multi-edits act on.
    fn select_group(model: &mut EditorModel) -> [Selection; 3] {
        let coord = ChunkCoord::new(0, 0);
        let group = [
            Selection { coord, id: InstanceId(0) },
            Selection { coord, id: InstanceId(2) },
            Selection { coord, id: InstanceId(5) },
        ];
        for sel in group {
            model.selection.toggle(sel);
        }
        group
    }

    // ---- each verb applies to every member ----

    #[test]
    fn move_selection_shifts_every_member_by_delta_and_undoes_in_one_step() {
        let mut model = sample_model();
        let group = select_group(&mut model);
        let before: Vec<Vec3> =
            group.iter().map(|s| model.placement(*s).unwrap().transform.translation).collect();
        let delta = Vec3::new(3.0, 1.5, -2.0);

        model.checkpoint(&Action::MoveSelection { delta });
        model.move_selection(delta).unwrap();
        for (sel, &was) in group.iter().zip(&before) {
            assert_eq!(model.placement(*sel).unwrap().transform.translation, was + delta, "moved by delta");
        }

        assert!(model.undo().unwrap());
        for (sel, &was) in group.iter().zip(&before) {
            assert_eq!(model.placement(*sel).unwrap().transform.translation, was, "the whole group restored");
        }
        assert!(!model.undo().unwrap(), "the group move was a single undo step");
    }

    #[test]
    fn a_single_selection_move_still_shifts_and_undoes() {
        let mut model = sample_model();
        let sel = Selection { coord: ChunkCoord::new(0, 0), id: InstanceId(0) };
        model.selection.replace(sel);
        let before = model.placement(sel).unwrap().transform.translation;
        let delta = Vec3::new(2.0, 0.0, 1.0);

        model.checkpoint(&Action::MoveSelection { delta });
        model.move_selection(delta).unwrap();
        assert_eq!(model.placement(sel).unwrap().transform.translation, before + delta);

        assert!(model.undo().unwrap());
        assert_eq!(model.placement(sel).unwrap().transform.translation, before, "a one-member move undoes too");
    }

    #[test]
    fn rotate_selection_turns_every_member_in_place() {
        let mut model = sample_model();
        let group = select_group(&mut model);
        let before: Vec<Transform> =
            group.iter().map(|s| model.placement(*s).unwrap().transform).collect();
        let delta = Quat::from_rotation_y(0.5);

        model.rotate_selection(delta).unwrap();
        for (sel, was) in group.iter().zip(&before) {
            let now = model.placement(*sel).unwrap().transform;
            assert_eq!(now.translation, was.translation, "in place: position unchanged");
            assert_eq!(now.scale, was.scale, "scale unchanged");
            assert!(now.rotation.dot(delta * was.rotation).abs() > 1.0 - 1e-6, "each rotated by delta");
        }
    }

    #[test]
    fn scale_selection_multiplies_every_member() {
        let mut model = sample_model();
        let group = select_group(&mut model);
        let before: Vec<Vec3> = group.iter().map(|s| model.placement(*s).unwrap().transform.scale).collect();

        model.scale_selection(2.0).unwrap();
        for (sel, &was) in group.iter().zip(&before) {
            assert_eq!(model.placement(*sel).unwrap().transform.scale, was * 2.0, "scaled by the factor");
        }
    }

    #[test]
    fn set_state_selection_sets_every_member_then_clears_to_default() {
        let mut model = sample_model_with_open_state();
        let group = select_group(&mut model);

        model.set_state_selection(Some("open")).unwrap();
        for sel in group {
            assert_eq!(model.placement(sel).unwrap().state.as_deref(), Some("open"), "state set on all");
        }
        // None is the implicit default; setting it clears every member back.
        model.set_state_selection(None).unwrap();
        assert!(group.iter().all(|s| model.placement(*s).unwrap().state.is_none()), "None clears all states");
    }

    // ---- coalescing: a transform drag is one undo step; a state set is its own ----

    #[test]
    fn a_move_run_coalesces_across_frames_into_one_undo_step() {
        let mut model = sample_model();
        let sel = Selection { coord: ChunkCoord::new(0, 0), id: InstanceId(0) };
        model.selection.replace(sel);
        let before = model.placement(sel).unwrap().transform.translation;

        // A drag is many MoveSelection frames, each checkpointed at the writer; the run coalesces,
        // so only the first keeps a snapshot.
        for _ in 0..5 {
            let delta = Vec3::new(1.0, 0.0, 0.0);
            model.checkpoint(&Action::MoveSelection { delta });
            model.move_selection(delta).unwrap();
        }
        assert_eq!(model.placement(sel).unwrap().transform.translation, before + Vec3::new(5.0, 0.0, 0.0));

        assert!(model.undo().unwrap());
        assert_eq!(model.placement(sel).unwrap().transform.translation, before, "one undo rewinds the drag");
        assert!(!model.undo().unwrap(), "the drag was a single recorded step");
    }

    #[test]
    fn move_rotate_and_scale_coalesce_into_one_transform_undo_step() {
        let mut model = sample_model();
        let group = select_group(&mut model);
        let before: Vec<Transform> =
            group.iter().map(|s| model.placement(*s).unwrap().transform).collect();

        // Position, then rotation, then scale within one gesture: all three open or continue ONE
        // transform run, so the whole sequence is a single undo step.
        let steps: [Action; 3] = [
            Action::MoveSelection { delta: Vec3::new(1.0, 0.0, 0.0) },
            Action::RotateSelection { delta: Quat::from_rotation_y(0.2) },
            Action::ScaleSelection { factor: 1.5 },
        ];
        for action in steps {
            model.checkpoint(&action);
            match action {
                Action::MoveSelection { delta } => model.move_selection(delta).unwrap(),
                Action::RotateSelection { delta } => model.rotate_selection(delta).unwrap(),
                Action::ScaleSelection { factor } => model.scale_selection(factor).unwrap(),
                _ => unreachable!(),
            }
        }

        assert!(model.undo().unwrap());
        for (sel, was) in group.iter().zip(&before) {
            assert_eq!(model.placement(*sel).unwrap().transform, *was, "the whole run rewinds at once");
        }
        assert!(!model.undo().unwrap(), "move + rotate + scale were one undo step");
    }

    #[test]
    fn a_state_set_is_its_own_undo_step_and_breaks_the_transform_run() {
        let mut model = sample_model_with_open_state();
        let group = select_group(&mut model);

        model.checkpoint(&Action::MoveSelection { delta: Vec3::new(1.0, 0.0, 0.0) });
        model.move_selection(Vec3::new(1.0, 0.0, 0.0)).unwrap();
        // A discrete state set closes the transform run and takes its own checkpoint.
        model.checkpoint(&Action::SetStateSelection { state: Some("open".to_string()) });
        model.set_state_selection(Some("open")).unwrap();

        assert!(model.undo().unwrap());
        assert!(group.iter().all(|s| model.placement(*s).unwrap().state.is_none()), "first undo: state back");
        assert!(model.undo().unwrap(), "second undo: the move");
        assert!(!model.undo().unwrap(), "the move and the state set were two separate steps");
    }
}
