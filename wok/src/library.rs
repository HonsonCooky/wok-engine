//! The Prefabs page: every prefab on disk, read-only; clicking arms place mode.
//!
//! Carried over from the v1 left panel's library section, now a page of its own. Behavior is
//! unchanged: one armed prefab at a time, the next viewport click places it, Esc (or re-clicking
//! the armed entry) disarms.

use wok_scene::PrefabRef;

use crate::model::EditorModel;
use crate::panels::{Action, UiState};

/// Build the Prefabs page into the left panel.
pub fn page(ui: &mut egui::Ui, model: &EditorModel, ui_state: &UiState, actions: &mut Vec<Action>) {
    ui.add_space(2.0);
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        let mut names: Vec<&str> = model.prefabs.keys().map(PrefabRef::as_str).collect();
        names.sort_unstable();
        for name in names {
            let armed = ui_state.placing.as_ref().is_some_and(|p| p.as_str() == name);
            if ui.selectable_label(armed, name).clicked() {
                actions.push(if armed {
                    Action::DisarmPlace
                } else {
                    Action::ArmPlace(PrefabRef::new(name))
                });
            }
        }
        if let Some(placing) = &ui_state.placing {
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(format!("click terrain to place {} (Esc cancels)", placing.as_str()))
                    .small()
                    .weak(),
            );
        }
    });
}
