//! The action layer's apply point: the one place the authored model mutates, split from
//! `crate::app` so the frame loop stays at its own altitude.
//!
//! Both the UI surfaces and the viewport input routing emit [`Action`]s; the frame loop applies
//! them here, so the model has a single writer and undo/redo can ride it (`crate::history`
//! checkpoints before each mutating action). Inert actions (selection, place mode, frame, save)
//! record no checkpoint. Presentation-only state (place mode, the context menu, the drag, the
//! marquee, the interaction mode) is mutated where it is read, never through an action.

use crate::camera;
use crate::app::EditorApp;
use crate::panels::Action;
use crate::sync;

impl EditorApp {
    pub(crate) fn apply_action(&mut self, action: Action) {
        // The single writer checkpoints before every mutating action; a run of edits to one
        // selection coalesces into one undo step (`crate::history`), and inert actions record
        // nothing. Doing it here covers both the UI and the input-routing apply passes.
        self.model.checkpoint(&action);
        match action {
            Action::Select(Some(sel)) => self.model.selection.replace(sel),
            Action::Select(None) => self.model.selection.clear(),
            Action::ToggleSelect(sel) => self.model.selection.toggle(sel),
            Action::SelectMany { items, add } => self.model.selection.extend(items, add),
            Action::Edit { sel, transform, state } => {
                if let Err(err) = self.model.edit_placement(sel, transform, state) {
                    eprintln!("wok: edit failed: {err}");
                }
            }
            Action::MoveSelection { delta } => {
                if let Err(err) = self.model.move_selection(delta) {
                    eprintln!("wok: move failed: {err}");
                }
            }
            Action::RotateSelection { delta } => {
                if let Err(err) = self.model.rotate_selection(delta) {
                    eprintln!("wok: rotate failed: {err}");
                }
            }
            Action::ScaleSelection { factor } => {
                if let Err(err) = self.model.scale_selection(factor) {
                    eprintln!("wok: scale failed: {err}");
                }
            }
            Action::SetStateSelection { state } => {
                if let Err(err) = self.model.set_state_selection(state.as_deref()) {
                    eprintln!("wok: set state failed: {err}");
                }
            }
            Action::ArmPlace(prefab) => self.ui.placing = Some(prefab),
            Action::DisarmPlace => self.ui.placing = None,
            Action::Place { prefab, point } => {
                if let Err(err) = self.model.place(&prefab, point) {
                    eprintln!("wok: place failed: {err}");
                }
            }
            Action::Duplicate => match self.model.duplicate_selection() {
                // The copies are selected by the model; bring the primary's tree row into view.
                Ok(()) => self.ui.scroll_to_selection = true,
                Err(err) => eprintln!("wok: duplicate failed: {err}"),
            },
            Action::Rename { sel, name } => {
                self.model.rename(sel, &name);
            }
            Action::Delete => {
                if let Err(err) = self.model.delete_selection() {
                    eprintln!("wok: delete failed: {err}");
                }
            }
            Action::Frame(sel) => {
                // Frames the free-fly camera (the double-click "take me to it"). In object mode the
                // camera is locked to the selection and re-derived from the orbit each frame, so this
                // set is harmless there - the orbit step keeps the camera on the selection.
                if let Some(bounds) = self.model.world_bounds(sel) {
                    self.camera = camera::frame(&self.camera, bounds.min, bounds.max);
                }
            }
            Action::Save => match sync::save(&mut self.model, &self.paths) {
                Ok(()) => println!("wok: saved"),
                Err(err) => eprintln!("wok: save failed: {err}"),
            },
            Action::Undo => {
                if let Err(err) = self.model.undo() {
                    eprintln!("wok: undo failed: {err}");
                }
            }
            Action::Redo => {
                if let Err(err) = self.model.redo() {
                    eprintln!("wok: redo failed: {err}");
                }
            }
        }
    }
}
