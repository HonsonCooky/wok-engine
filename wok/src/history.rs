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
//! Coalescing keeps one gesture to one undo step: an unbroken run of `Edit`s to the same selection
//! (a drag is one such run, frame by frame) checkpoints only once, before the run's first edit.
//! Any other applied action closes the run. Because undo rides the action layer (the design's
//! premise), this module reads the `Action` vocabulary directly to classify what mutates; it owns
//! the stacks and the run state, and the writer owns when to call it.

use std::collections::BTreeMap;

use wok_content::StoreError;
use wok_scene::{Chunk, ChunkCoord, Scene};

use crate::model::{EditorModel, Selection};
use crate::panels::Action;

/// One authored-state snapshot: what a mutating action is checkpointed against, and what undo or
/// redo swaps back. Private to the history mechanism - only the model methods below build and
/// consume it.
struct Snapshot {
    scene: Scene,
    chunks: BTreeMap<ChunkCoord, Chunk>,
    selection: Option<Selection>,
}

/// The undo and redo stacks plus the open-edit-run marker that drives coalescing. Owned by
/// [`EditorModel`] and mutated only through the model methods below; starts empty, so the initial
/// loaded state is not itself an undo target.
#[derive(Default)]
pub struct History {
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// The selection of the open `Edit` run, set while the last checkpoint-relevant action was an
    /// `Edit`. A further `Edit` to this same selection coalesces into the checkpoint already taken;
    /// any other action clears it and so closes the run.
    open_edit: Option<Selection>,
}

impl History {
    /// Advance the run state for the action about to apply and report whether a pre-action
    /// checkpoint should be taken now. Exhaustive over [`Action`] on purpose: a new variant must
    /// declare its history effect here rather than silently going unrecorded.
    fn should_checkpoint(&mut self, action: &Action) -> bool {
        match action {
            // A run of edits to one selection is one gesture: only its first edit checkpoints.
            Action::Edit { sel, .. } if self.open_edit == Some(*sel) => false,
            Action::Edit { sel, .. } => {
                self.open_edit = Some(*sel);
                true
            }
            // Every other mutating action closes any run and takes its own checkpoint.
            Action::Place { .. } | Action::Delete(_) | Action::Duplicate(_) | Action::Rename { .. } => {
                self.open_edit = None;
                true
            }
            // Non-mutating actions, plus Save (it writes disk; undoing it would desync memory from
            // files) and Undo/Redo themselves, record nothing but still close any open run.
            Action::Select(_) | Action::ArmPlace(_) | Action::DisarmPlace | Action::Frame(_)
            | Action::Save | Action::Undo | Action::Redo => {
                self.open_edit = None;
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
        self.history.open_edit = None;
        self.restore(snapshot)?;
        Ok(true)
    }

    /// Redo the last undone change: the mirror of [`Self::undo`], banking the current state for
    /// undo. Returns whether anything was redone.
    pub fn redo(&mut self) -> Result<bool, StoreError> {
        let Some(snapshot) = self.history.redo.pop() else { return Ok(false) };
        let current = self.snapshot();
        self.history.undo.push(current);
        self.history.open_edit = None;
        self.restore(snapshot)?;
        Ok(true)
    }

    /// Capture the authored state into a snapshot. The clone is the same authored-form clone
    /// `retransform` makes each edit, so a checkpoint costs an edit's worth of work.
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            scene: self.scene.clone(),
            chunks: self.chunks.clone(),
            selection: self.selection,
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
        let before = Some(Selection { coord, id: InstanceId(3) });
        model.selection = before;

        let place = Action::Place { prefab: PrefabRef::new("crate"), point: interior(40.0, 40.0) };
        model.checkpoint(&place);
        let placed = model.place(&PrefabRef::new("crate"), interior(40.0, 40.0)).unwrap().expect("placed");
        assert_eq!(model.placement_count(), count + 1);
        assert_eq!(model.selection, Some(placed));

        assert!(model.undo().unwrap(), "undo reports it acted");
        assert_eq!(model.placement_count(), count, "the placement is gone");
        assert!(model.placement(placed).is_none(), "gone by id, too");
        assert_eq!(model.selection, before, "the pre-place selection is restored");

        assert!(model.redo().unwrap(), "redo reports it acted");
        assert_eq!(model.placement_count(), count + 1, "the placement is back");
        assert_eq!(model.selection, Some(placed), "and selected again, same id");
    }

    #[test]
    fn delete_then_undo_restores_and_reselects_then_redo_redeletes() {
        let mut model = sample_model();
        let coord = ChunkCoord::new(0, 0);
        let sel = Selection { coord, id: InstanceId(2) };
        model.selection = Some(sel);
        let count = model.placement_count();
        let original = model.placement(sel).unwrap().clone();

        model.checkpoint(&Action::Delete(sel));
        assert!(model.delete(sel).unwrap());
        assert_eq!(model.placement_count(), count - 1);
        assert_eq!(model.selection, None, "delete clears the dangling selection");

        assert!(model.undo().unwrap());
        assert_eq!(model.placement_count(), count, "the placement is back");
        assert_eq!(model.placement(sel), Some(&original), "and identical to before");
        assert_eq!(model.selection, Some(sel), "the snapshot held the selection, so it reselects");

        assert!(model.redo().unwrap());
        assert_eq!(model.placement_count(), count - 1, "deleted again");
        assert!(model.placement(sel).is_none());
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
