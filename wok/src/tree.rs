//! The Scene page: the explorer-grade scene tree.
//!
//! Renders `crate::outline`'s tested tree model: collapsible chunk nodes, one row per placement
//! with a painter-drawn kind glyph, the display name (or generated label), and the prefab name
//! dimmed beside it. Selection highlights the row; when the selection arrived from the viewport
//! (`UiState::scroll_to_selection`) the row's chunk is forced open and the row scrolled into
//! view, so a click in the world always lands the eye in the tree. Double-click frames the camera
//! on the placement; right-click opens the same Duplicate / Rename / Delete menu the viewport
//! uses ([`placement_menu`]). Rename is an inline edit over the row, committing on Enter or focus
//! loss and cancelling on Esc; the committed text goes through `EditorModel::rename`, where empty
//! clears back to no name.

use egui::text::LayoutJob;
use egui::{Align, Sense, TextFormat, vec2};

use crate::glyphs;
use crate::model::{EditorModel, Selection};
use crate::outline::{self, PlacementRow};
use crate::pages::Page;
use crate::panels::{Action, Rename, UiState};

/// Kind-glyph square, in points: sized to the row's text height.
const GLYPH_SIZE: f32 = 14.0;

/// Build the Scene page into the left panel.
pub fn page(ui: &mut egui::Ui, model: &EditorModel, ui_state: &mut UiState, actions: &mut Vec<Action>) {
    ui.add_space(2.0);
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for node in outline::tree(model) {
            // A viewport selection must be visible even if its chunk was folded away.
            let force_open = ui_state.scroll_to_selection
                && model.selection.is_some_and(|sel| sel.coord == node.coord);
            egui::CollapsingHeader::new(format!("chunk {}_{}", node.coord.x, node.coord.z))
                .default_open(true)
                .open(force_open.then_some(true))
                .show(ui, |ui| {
                    for row in &node.rows {
                        let sel = Selection { coord: node.coord, id: row.id };
                        placement_row(ui, model, sel, row, ui_state, actions);
                    }
                    if node.rows.is_empty() {
                        ui.weak("no placements");
                    }
                });
        }
    });
    // The scroll request is satisfied by the build above (or had nothing to land on).
    ui_state.scroll_to_selection = false;
}

fn placement_row(
    ui: &mut egui::Ui,
    model: &EditorModel,
    sel: Selection,
    row: &PlacementRow,
    ui_state: &mut UiState,
    actions: &mut Vec<Action>,
) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(GLYPH_SIZE, GLYPH_SIZE), Sense::hover());
        glyphs::kind(ui.painter(), rect, row.kind, ui.visuals().weak_text_color());

        if ui_state.renaming.as_ref().is_some_and(|r| r.sel == sel) {
            rename_field(ui, sel, row, ui_state, actions);
            return;
        }

        let selected = model.selection == Some(sel);
        let response = ui.selectable_label(selected, row_text(ui, row));
        if response.double_clicked() {
            // Select and frame: the double click is "take me to it".
            actions.push(Action::Select(Some(sel)));
            actions.push(Action::Frame(sel));
        } else if response.clicked() {
            actions.push(Action::Select(Some(sel)));
        }
        response.context_menu(|ui| {
            if placement_menu(ui, sel, model, ui_state, actions) {
                ui.close_menu();
            }
        });
        if selected && ui_state.scroll_to_selection {
            response.scroll_to_me(Some(Align::Center));
        }
    });
}

/// The row's two-tone text: the display label, then the prefab name dimmed beside it.
fn row_text(ui: &egui::Ui, row: &PlacementRow) -> LayoutJob {
    let body = egui::TextStyle::Body.resolve(ui.style());
    let mut job = LayoutJob::default();
    job.append(
        &row.label,
        0.0,
        TextFormat { font_id: body.clone(), color: ui.visuals().text_color(), ..Default::default() },
    );
    job.append(
        &row.prefab,
        8.0,
        TextFormat { font_id: body, color: ui.visuals().weak_text_color(), ..Default::default() },
    );
    job
}

/// The inline rename editor in the row's place. Commits on Enter or focus loss, cancels on Esc;
/// the empty string commits too - that is how a name is cleared back to the generated label.
fn rename_field(
    ui: &mut egui::Ui,
    sel: Selection,
    row: &PlacementRow,
    ui_state: &mut UiState,
    actions: &mut Vec<Action>,
) {
    let Some(rename) = ui_state.renaming.as_mut() else { return };
    let response = ui.add(
        egui::TextEdit::singleline(&mut rename.buffer)
            .hint_text(row.generated.as_str())
            .desired_width(f32::INFINITY),
    );
    if rename.take_focus {
        response.request_focus();
        rename.take_focus = false;
    }
    if response.lost_focus() {
        if !ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            actions.push(Action::Rename { sel, name: rename.buffer.clone() });
        }
        ui_state.renaming = None;
    }
}

/// The placement context menu's items, shared by the tree rows and the viewport menu. Returns
/// whether an item was chosen (the caller closes its menu).
pub fn placement_menu(
    ui: &mut egui::Ui,
    sel: Selection,
    model: &EditorModel,
    ui_state: &mut UiState,
    actions: &mut Vec<Action>,
) -> bool {
    let mut chose = false;
    if ui.button("Duplicate").clicked() {
        actions.push(Action::Duplicate(sel));
        chose = true;
    }
    if ui.button("Rename").clicked() {
        begin_rename(model, sel, ui_state);
        chose = true;
    }
    if ui.button("Delete").clicked() {
        actions.push(Action::Delete(sel));
        chose = true;
    }
    chose
}

/// Start an inline rename: seed the buffer with the current name (empty for unnamed; the hint
/// shows the generated label), make sure the tree page is showing, and bring the row into view.
pub fn begin_rename(model: &EditorModel, sel: Selection, ui_state: &mut UiState) {
    let buffer = model.placement(sel).and_then(|p| p.name.clone()).unwrap_or_default();
    ui_state.renaming = Some(Rename { sel, buffer, take_focus: true });
    ui_state.pages.select(Page::Scene);
    ui_state.scroll_to_selection = true;
}
