//! Undo and redo as authored-form snapshots, riding the single writer.
//!
//! `crate::app`'s `apply_action` is the one place the model mutates, so a checkpoint taken there
//! before each mutating action is a sufficient record. A snapshot is the authored state - the
//! scene (including its `next_instance_id` counter), the chunks, and the selection - exactly the
//! clone `EditorModel::retransform` already makes each edit, never the runtime arrays it derives.
//! Heightmaps and lighting are left out by design: no action mutates them yet, so nothing would
//! restore. Undo swaps the authored state back and re-runs the same authored -> runtime transform,
//! so it costs what the edits it reverses cost and the viewport tracks it for free.
//!
//! Coalescing keeps one gesture to one undo step: a run of like mutations - the inspector's edits
//! to one placement, or a viewport group-drag's per-frame moves - checkpoints only once, before
//! the run's first action. Any other applied action closes the run. Because undo rides the action
//! layer (the design's premise), this module reads the `Action` vocabulary directly to classify
//! what mutates; it owns the stacks and the run state, and the writer owns when to call it.

use std::collections::BTreeMap;

use wok_content::StoreError;
use wok_scene::{Chunk, ChunkCoord, Scene};

use crate::model::{EditorModel, Selection};
use crate::panels::Action;
use crate::selection::SelectionSet;

/// One authored-state snapshot: what a mutating action is checkpointed against, and what undo or
/// redo swaps back. Private to the history mechanism - only the model methods below build and
/// consume it.
struct Snapshot {
    scene: Scene,
    chunks: BTreeMap<ChunkCoord, Chunk>,
    selection: SelectionSet,
}

/// The open coalescing run, if any. A gesture made of many small mutations - the inspector's edits
/// to one placement, or a viewport drag's per-frame moves - records one checkpoint, before its
/// first action, and stays open while like actions continue; any other applied action closes it.
/// An `Edit` run is keyed on its selection (a further edit to the same one coalesces); a `Move` run
/// coalesces any consecutive `MoveSelection` (the moved group has no single key).
#[derive(Default, PartialEq, Eq)]
enum OpenRun {
    #[default]
    None,
    Edit(Selection),
    Move,
}

/// The undo and redo stacks plus the open-run marker that drives coalescing. Owned by
/// [`EditorModel`] and mutated only through the model methods below; starts empty, so the initial
/// loaded state is not itself an undo target.
#[derive(Default)]
pub struct History {
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    open: OpenRun,
}

impl History {
    /// Advance the run state for the action about to apply and report whether a pre-action
    /// checkpoint should be taken now. Exhaustive over [`Action`] on purpose: a new variant must
    /// declare its history effect here rather than silently going unrecorded.
    fn should_checkpoint(&mut self, action: &Action) -> bool {
        match action {
            // A run of edits to one selection is one gesture: only its first edit checkpoints.
            Action::Edit { sel, .. } if self.open == OpenRun::Edit(*sel) => false,
            Action::Edit { sel, .. } => {
                self.open = OpenRun::Edit(*sel);
                true
            }
            // A group-move drag is one gesture too: only its first frame checkpoints, the rest
            // coalesce. Keyed on "a move run is open", not a selection - the group has no one key.
            Action::MoveSelection { .. } if self.open == OpenRun::Move => false,
            Action::MoveSelection { .. } => {
                self.open = OpenRun::Move;
                true
            }
            // Every other mutating action closes any run and takes its own checkpoint. The set
            // delete/duplicate are one action and so one checkpoint, exactly like their single
            // forms were - the whole group undoes in a single step.
            Action::Place { .. } | Action::Delete | Action::Duplicate | Action::Rename { .. } => {
                self.open = OpenRun::None;
                true
            }
            // Non-mutating actions, plus Save (it writes disk; undoing it would desync memory from
            // files) and Undo/Redo themselves, record nothing but still close any open run.
            // ToggleSelect and SelectMany (the marquee) change only the selection, so they record
            // nothing, like Select.
            Action::Select(_) | Action::ToggleSelect(_) | Action::SelectMany { .. }
            | Action::ArmPlace(_) | Action::DisarmPlace | Action::Frame(_) | Action::Save
            | Action::Undo | Action::Redo => {
                self.open = OpenRun::None;
                false
            }
        }
    }
}

impl EditorModel {
    /// Take the pre-action checkpoint a mutating action needs, honoring edit-run coalescing. Called
    /// at the single writer before the action applies; inert actions and coalesced edits push
    /// nothing, so the clone only happens when a snapshot is actually kept. Clearing the redo stack
    /// here is what makes a new edit after an undo abandon the redone-away future - and is why
    /// rewinding `next_instance_id` on undo never collides with a live redo.
    pub fn checkpoint(&mut self, action: &Action) {
        if self.history.should_checkpoint(action) {
            let snapshot = self.snapshot();
            self.history.undo.push(snapshot);
            self.history.redo.clear();
        }
    }

    /// Undo the last checkpointed change: restore the top undo snapshot, banking the current state
    /// for redo. Returns whether anything was undone (`false` on an empty stack, touching nothing).
    pub fn undo(&mut self) -> Result<bool, StoreError> {
        let Some(snapshot) = self.history.undo.pop() else { return Ok(false) };
        let current = self.snapshot();
        self.history.redo.push(current);
        self.history.open = OpenRun::None;
        self.restore(snapshot)?;
        Ok(true)
    }

    /// Redo the last undone change: the mirror of [`Self::undo`], banking the current state for
    /// undo. Returns whether anything was redone.
    pub fn redo(&mut self) -> Result<bool, StoreError> {
        let Some(snapshot) = self.history.redo.pop() else { return Ok(false) };
        let current = self.snapshot();
        self.history.undo.push(current);
        self.history.open = OpenRun::None;
        self.restore(snapshot)?;
        Ok(true)
    }

    /// Capture the authored state into a snapshot. The clone is the same authored-form clone
    /// `retransform` makes each edit, so a checkpoint costs an edit's worth of work.
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            scene: self.scene.clone(),
            chunks: self.chunks.clone(),
            selection: self.selection.clone(),
        }
    }

    /// Restore a snapshot: swap the authored state back, re-transform every loaded chunk so the
    /// runtime arrays match, drop a now-dangling selection, and mark everything dirty (the authored
    /// form differs from disk again; over-marking is safe, under-marking would lose work). It
    /// re-transforms all loaded chunks rather than a computed diff - whole-form snapshots at
    /// single-chunk scale, per the brief; revisit alongside per-chunk snapshots.
    fn restore(&mut self, snapshot: Snapshot) -> Result<(), StoreError> {
        self.scene = snapshot.scene;
        self.chunks = snapshot.chunks;
        self.selection = snapshot.selection;
        let coords: Vec<ChunkCoord> = self.chunks.keys().copied().collect();
        for coord in coords {
            self.retransform(coord)?;
            self.dirty_chunks.insert(coord);
        }
        self.scene_dirty = true;
        self.validate_selection();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::chunk_origin;
    use crate::sample;
    use glam::Vec3;
    use wok_scene::{ChunkCoord, InstanceId, PrefabRef, Transform};

    fn sample_model() -> EditorModel {
        let content = sample::build();
        EditorModel::new(
            content.scene,
            content.prefabs.into_iter().collect(),
            vec![(content.chunk, Some(content.heightmap))],
        )
        .expect("sample content loads")
    }

    /// A terrain point well inside the loaded chunk, where a place lands.
    fn interior(x: f32, z: f32) -> Vec3 {
        chunk_origin(ChunkCoord::new(0, 0)) + Vec3::new(x, 0.0, z)
    }

    #[test]
    fn place_then_undo_removes_it_and_restores_selection_and_count_then_redo_replaces() {
        let mut model = sample_model();
        let coord = ChunkCoord::new(0, 0);
        let count = model.placement_count();
        // A prior selection the undo must restore.
        let before = Selection { coord, id: InstanceId(3) };
        model.selection.replace(before);

        let place = Action::Place { prefab: PrefabRef::new("crate"), point: interior(40.0, 40.0) };
        model.checkpoint(&place);
        let placed = model.place(&PrefabRef::new("crate"), interior(40.0, 40.0)).unwrap().expect("placed");
        assert_eq!(model.placement_count(), count + 1);
        assert_eq!(model.selection.primary(), Some(placed));

        assert!(model.undo().unwrap(), "undo reports it acted");
        assert_eq!(model.placement_count(), count, "the placement is gone");
        assert!(model.placement(placed).is_none(), "gone by id, too");
        assert_eq!(model.selection.primary(), Some(before), "the pre-place selection is restored");

        assert!(model.redo().unwrap(), "redo reports it acted");
        assert_eq!(model.placement_count(), count + 1, "the placement is back");
        assert_eq!(model.selection.primary(), Some(placed), "and selected again, same id");
    }

    #[test]
    fn delete_then_undo_restores_and_reselects_then_redo_redeletes() {
        let mut model = sample_model();
        let coord = ChunkCoord::new(0, 0);
        let sel = Selection { coord, id: InstanceId(2) };
        model.selection.replace(sel);
        let count = model.placement_count();
        let original = model.placement(sel).unwrap().clone();

        model.checkpoint(&Action::Delete);
        assert!(model.delete(sel).unwrap());
        assert_eq!(model.placement_count(), count - 1);
        assert!(model.selection.is_empty(), "delete clears the dangling selection");

        assert!(model.undo().unwrap());
        assert_eq!(model.placement_count(), count, "the placement is back");
        assert_eq!(model.placement(sel), Some(&original), "and identical to before");
        assert_eq!(model.selection.primary(), Some(sel), "the snapshot held the selection, so it reselects");

        assert!(model.redo().unwrap());
        assert_eq!(model.placement_count(), count - 1, "deleted again");
        assert!(model.placement(sel).is_none());
    }

    #[test]
    fn set_delete_removes_every_selected_placement_and_one_undo_restores_them() {
        let mut model = sample_model();
        let coord = ChunkCoord::new(0, 0);
        let group = [
            Selection { coord, id: InstanceId(0) },
            Selection { coord, id: InstanceId(2) },
            Selection { coord, id: InstanceId(5) },
        ];
        for sel in group {
            model.selection.toggle(sel);
        }
        let count = model.placement_count();

        // One checkpoint for the whole group (the writer takes it once per action), then delete all.
        model.checkpoint(&Action::Delete);
        model.delete_selection().unwrap();
        assert_eq!(model.placement_count(), count - 3, "all three gone");
        assert!(model.selection.is_empty(), "nothing left selected");
        assert!(group.iter().all(|s| model.placement(*s).is_none()));

        // One undo brings the whole group and the selection back.
        assert!(model.undo().unwrap());
        assert_eq!(model.placement_count(), count, "all three restored in one step");
        assert!(group.iter().all(|s| model.placement(*s).is_some()));
        assert!(group.iter().all(|s| model.selection.contains(*s)), "the group is selected again");
        assert!(!model.undo().unwrap(), "the group delete was a single undo step");
    }

    #[test]
    fn set_duplicate_copies_the_group_selects_the_copies_and_undoes_in_one_step() {
        let mut model = sample_model();
        let coord = ChunkCoord::new(0, 0);
        let a = Selection { coord, id: InstanceId(0) };
        let b = Selection { coord, id: InstanceId(3) };
        model.selection.toggle(a);
        model.selection.toggle(b);
        let count = model.placement_count();

        model.checkpoint(&Action::Duplicate);
        model.duplicate_selection().unwrap();
        assert_eq!(model.placement_count(), count + 2, "both members copied");
        assert_eq!(model.selection.len(), 2, "the copies are selected");
        assert!(!model.selection.contains(a) && !model.selection.contains(b), "the originals are not");
        assert!(model.selection.iter().all(|c| model.placement(c).is_some()), "each copy resolves");

        assert!(model.undo().unwrap());
        assert_eq!(model.placement_count(), count, "both copies removed in one step");
        assert!(model.selection.contains(a) && model.selection.contains(b), "the originals reselect");
        assert!(!model.undo().unwrap(), "the group duplicate was a single undo step");
    }

    #[test]
    fn a_single_edit_undoes_and_redoes_the_transform() {
        let mut model = sample_model();
        let sel = Selection { coord: ChunkCoord::new(0, 0), id: InstanceId(0) };
        let before = model.placement(sel).unwrap().transform;
        let after = Transform { translation: Vec3::new(9.0, 3.0, 7.0), ..before };

        model.checkpoint(&Action::Edit { sel, transform: after, state: None });
        model.edit_placement(sel, after, None).unwrap();
        assert_eq!(model.placement(sel).unwrap().transform, after);

        assert!(model.undo().unwrap());
        assert_eq!(model.placement(sel).unwrap().transform, before, "undo restores the prior transform");
        // The runtime arrays re-transformed, not just the authored form: the store re-slices to the
        // restored chunk, so the viewport tracks the undo.
        let runtime = model.store.get(sel.coord).expect("chunk still loaded after undo");
        let direct = wok_scene::slice_chunk(&model.chunks[&sel.coord], &model.prefabs).unwrap();
        assert_eq!(runtime.visible, direct.visible, "undo re-sliced the chunk into the store");

        assert!(model.redo().unwrap());
        assert_eq!(model.placement(sel).unwrap().transform, after, "redo reapplies the edit");
    }

    #[test]
    fn a_run_of_edits_to_one_selection_collapses_to_one_undo_entry() {
        let mut model = sample_model();
        let sel = Selection { coord: ChunkCoord::new(0, 0), id: InstanceId(0) };
        let before = model.placement(sel).unwrap().transform;

        // Five edits to the same selection, each checkpointed at the writer exactly as a drag's
        // frames are: the run coalesces, so only the first keeps a snapshot.
        for i in 1..=5 {
            let t = Transform { translation: Vec3::new(i as f32, 0.0, 0.0), ..before };
            model.checkpoint(&Action::Edit { sel, transform: t, state: None });
            model.edit_placement(sel, t, None).unwrap();
        }
        assert_eq!(model.placement(sel).unwrap().transform.translation, Vec3::new(5.0, 0.0, 0.0));

        assert!(model.undo().unwrap());
        assert_eq!(model.placement(sel).unwrap().transform, before, "one undo rewinds the whole run");
        assert!(!model.undo().unwrap(), "the run was a single recorded step");
    }

    #[test]
    fn an_action_between_edits_breaks_the_run_into_separate_steps() {
        let mut model = sample_model();
        let sel = Selection { coord: ChunkCoord::new(0, 0), id: InstanceId(0) };
        let t0 = model.placement(sel).unwrap().transform;
        let t1 = Transform { translation: Vec3::new(1.0, 0.0, 0.0), ..t0 };
        let t2 = Transform { translation: Vec3::new(2.0, 0.0, 0.0), ..t0 };

        model.checkpoint(&Action::Edit { sel, transform: t1, state: None });
        model.edit_placement(sel, t1, None).unwrap();
        // A non-mutating Select between the edits closes the run; the second edit is its own step.
        model.checkpoint(&Action::Select(Some(sel)));
        model.checkpoint(&Action::Edit { sel, transform: t2, state: None });
        model.edit_placement(sel, t2, None).unwrap();

        assert!(model.undo().unwrap());
        assert_eq!(model.placement(sel).unwrap().transform, t1, "first undo: back to edit one");
        assert!(model.undo().unwrap());
        assert_eq!(model.placement(sel).unwrap().transform, t0, "second undo: back to the start");
    }

    #[test]
    fn a_new_mutation_after_an_undo_clears_the_redo_stack() {
        let mut model = sample_model();

        model.checkpoint(&Action::Place { prefab: PrefabRef::new("crate"), point: interior(20.0, 20.0) });
        model.place(&PrefabRef::new("crate"), interior(20.0, 20.0)).unwrap().expect("placed");
        assert!(model.undo().unwrap());

        // A fresh mutation drops the redone-away future.
        model.checkpoint(&Action::Place { prefab: PrefabRef::new("crate"), point: interior(50.0, 50.0) });
        model.place(&PrefabRef::new("crate"), interior(50.0, 50.0)).unwrap().expect("placed");

        assert!(!model.redo().unwrap(), "the cleared redo stack has nothing to replay");
        assert!(model.undo().unwrap(), "but the new mutation is itself undoable");
    }

    #[test]
    fn undo_and_redo_on_an_empty_history_are_no_ops() {
        let mut model = sample_model();
        assert!(!model.undo().unwrap(), "nothing to undo");
        assert!(!model.redo().unwrap(), "nothing to redo");
        assert!(!model.is_dirty(), "a no-op leaves the model clean");
    }

    #[test]
    fn save_records_no_checkpoint() {
        let mut model = sample_model();
        model.checkpoint(&Action::Save);
        assert!(!model.undo().unwrap(), "Save is deliberately not an undoable step");
    }
}
