//! The Scene page: the explorer-grade scene tree.
//!
//! Renders `crate::outline`'s tested tree model as hand-laid rows rather than stock egui widgets,
//! because the reference look (Zed's project panel) is built from invariants the stock widgets do
//! not promise: every row the same fixed height, an icon column that stays put, hover and
//! selection fills that span the panel's full width, and an indent guide under each open chunk.
//! Each row is one allocation - fill, glyphs, and text painted directly - so the geometry is the
//! same for every row by construction.
//!
//! Row anatomy: a chevron column (chunks only), a fixed icon column with the filled kind glyph
//! centered, the one-line label, and the prefab name right-aligned and very dim at the row's far
//! edge. The full identity (label, prefab, instance id) lives in the row's tooltip. Children
//! indent one icon-column width under their chunk, with a hairline guide at the folder icon's
//! center.
//!
//! Behavior is the part-1 contract unchanged: chunks collapse (open by default; a viewport
//! selection forces its chunk open and scrolls into view), click selects, double-click frames,
//! right-click opens the shared Duplicate / Rename / Delete menu ([`placement_menu`]), and rename
//! is an inline edit over the row, committing on Enter or focus loss and cancelling on Esc; the
//! committed text goes through `EditorModel::rename`, where empty clears back to no name.

use egui::text::{LayoutJob, TextWrapping};
use egui::{Align, Color32, Rect, Response, Sense, pos2, vec2};
use wok_scene::ChunkCoord;

use crate::glyphs;
use crate::model::{EditorModel, Selection};
use crate::outline::{self, PlacementRow};
use crate::pages::Page;
use crate::panels::{Action, Rename, UiState};
use crate::theme;

/// Row height in points: the fixed vertical rhythm, every row identical.
const ROW_HEIGHT: f32 = 22.0;

/// The icon column's width in points: also the indent unit, so children sit exactly one icon
/// column deeper than their chunk.
const ICON_COL: f32 = 16.0;

/// Glyph box, in points: centered in its column.
const ICON: f32 = 11.0;

/// Breathing room between the panel's left edge and the first column.
const LEFT_PAD: f32 = 4.0;

/// Breathing room between the right-aligned metadata and the panel's right edge.
const RIGHT_PAD: f32 = 8.0;

/// Gap between a column and the text that follows it.
const TEXT_GAP: f32 = 6.0;

/// Build the Scene page into the left panel.
pub fn page(ui: &mut egui::Ui, model: &EditorModel, ui_state: &mut UiState, actions: &mut Vec<Action>) {
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        // Rows are contiguous fixed-height allocations: the rhythm is ROW_HEIGHT alone.
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.add_space(2.0);
        for node in outline::tree(model) {
            // A viewport selection must be visible even if its chunk was folded away.
            let force_open = ui_state.scroll_to_selection
                && model.selection.primary().is_some_and(|sel| sel.coord == node.coord);
            if force_open {
                ui_state.collapsed.remove(&node.coord);
            }
            let open = !ui_state.collapsed.contains(&node.coord);
            chunk_row(ui, node.coord, open, ui_state);
            if !open {
                continue;
            }
            let children_top = ui.cursor().min.y;
            for row in &node.rows {
                let sel = Selection { coord: node.coord, id: row.id };
                placement_row(ui, model, sel, row, ui_state, actions);
            }
            if node.rows.is_empty() {
                empty_row(ui);
            }
            // The indent guide: one hairline at the folder icon's center, the full height of the
            // visible children. Painted after them, so it rides over their fills like the
            // reference's guides do.
            let x = ui.max_rect().left() + LEFT_PAD + ICON_COL * 1.5;
            let stroke = ui.visuals().widgets.noninteractive.bg_stroke;
            ui.painter().vline(x, children_top..=ui.cursor().min.y, stroke);
        }
    });
    // The scroll request is satisfied by the build above (or had nothing to land on).
    ui_state.scroll_to_selection = false;
}

/// Allocate one full-panel-width row: hover and selection fills share this exact rect, so
/// selection reads as "hover that stuck".
fn row_alloc(ui: &mut egui::Ui, sense: Sense) -> (Rect, Response) {
    ui.allocate_exact_size(vec2(ui.available_width(), ROW_HEIGHT), sense)
}

/// Paint the row's background: solid accent when selected, faint when hovered, nothing otherwise.
fn row_fill(ui: &egui::Ui, rect: Rect, response: &Response, selected: bool) {
    if selected {
        ui.painter().rect_filled(rect, 0.0, ui.visuals().selection.bg_fill);
    } else if response.hovered() {
        ui.painter().rect_filled(rect, 0.0, ui.visuals().widgets.hovered.weak_bg_fill);
    }
}

/// The glyph box centered in the column starting at `left`.
fn icon_box(left: f32, center_y: f32) -> Rect {
    Rect::from_center_size(pos2(left + ICON_COL * 0.5, center_y), vec2(ICON, ICON))
}

/// Paint a one-line label from `left`, truncated before `right`.
fn label_text(ui: &egui::Ui, rect: Rect, left: f32, text: &str, color: Color32, right: f32) {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let mut job = LayoutJob::simple_singleline(text.to_string(), font, color);
    job.wrap = TextWrapping::truncate_at_width((right - left).max(0.0));
    let galley = ui.fonts(|f| f.layout_job(job));
    let pos = pos2(left, rect.center().y - galley.size().y * 0.5);
    ui.painter().galley(pos, galley, color);
}

/// Paint the far-right dim metadata; returns its left edge, which caps the label.
fn meta_text(ui: &egui::Ui, rect: Rect, text: &str) -> f32 {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let color = theme::meta_color(ui.visuals());
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, color);
    let pos = pos2(rect.right() - RIGHT_PAD - galley.size().x, rect.center().y - galley.size().y * 0.5);
    ui.painter().galley(pos, galley, color);
    pos.x
}

/// A chunk's row: chevron column, filled folder glyph, label at regular weight. Clicking anywhere
/// on the row toggles the fold.
fn chunk_row(ui: &mut egui::Ui, coord: ChunkCoord, open: bool, ui_state: &mut UiState) {
    let (rect, response) = row_alloc(ui, Sense::click());
    row_fill(ui, rect, &response, false);
    let color = theme::icon_color(ui.visuals());
    let x0 = rect.left() + LEFT_PAD;
    glyphs::chevron(ui.painter(), icon_box(x0, rect.center().y), open, color);
    glyphs::folder(ui.painter(), icon_box(x0 + ICON_COL, rect.center().y), color);
    let label = format!("chunk {}_{}", coord.x, coord.z);
    let left = x0 + 2.0 * ICON_COL + TEXT_GAP;
    label_text(ui, rect, left, &label, ui.visuals().text_color(), rect.right() - RIGHT_PAD);
    if response.clicked() {
        if open {
            ui_state.collapsed.insert(coord);
        } else {
            ui_state.collapsed.remove(&coord);
        }
    }
}

fn placement_row(
    ui: &mut egui::Ui,
    model: &EditorModel,
    sel: Selection,
    row: &PlacementRow,
    ui_state: &mut UiState,
    actions: &mut Vec<Action>,
) {
    if ui_state.renaming.as_ref().is_some_and(|r| r.sel == sel) {
        rename_row(ui, row, sel, ui_state, actions);
        return;
    }

    let (rect, response) = row_alloc(ui, Sense::click());
    let selected = model.selection.contains(sel);
    row_fill(ui, rect, &response, selected);

    // The selected row's marks take the selection foreground; the metadata stays dim either way.
    let (icon_color, label_color) = if selected {
        let fg = ui.visuals().selection.stroke.color;
        (fg, fg)
    } else {
        (theme::icon_color(ui.visuals()), ui.visuals().text_color())
    };
    let x0 = rect.left() + LEFT_PAD + ICON_COL;
    glyphs::kind(ui.painter(), icon_box(x0 + ICON_COL, rect.center().y), row.kind, icon_color);
    let meta_left = meta_text(ui, rect, &row.prefab);
    let left = x0 + 2.0 * ICON_COL + TEXT_GAP;
    label_text(ui, rect, left, &row.label, label_color, meta_left - TEXT_GAP);

    let response = response
        .on_hover_text(format!("{}\nprefab: {}\ninstance: {}", row.label, row.prefab, row.id.0));
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
}

/// The dim "no placements" row under an empty chunk, at the placement label's indent.
fn empty_row(ui: &mut egui::Ui) {
    let (rect, _) = row_alloc(ui, Sense::hover());
    let left = rect.left() + LEFT_PAD + 3.0 * ICON_COL + TEXT_GAP;
    label_text(ui, rect, left, "no placements", theme::meta_color(ui.visuals()), rect.right() - RIGHT_PAD);
}

/// The inline rename editor in the row's place: the kind glyph stays put and the text zone
/// becomes the field. Commits on Enter or focus loss, cancels on Esc; the empty string commits
/// too - that is how a name is cleared back to the generated label.
fn rename_row(
    ui: &mut egui::Ui,
    row: &PlacementRow,
    sel: Selection,
    ui_state: &mut UiState,
    actions: &mut Vec<Action>,
) {
    let (rect, _) = row_alloc(ui, Sense::hover());
    let x0 = rect.left() + LEFT_PAD + ICON_COL;
    glyphs::kind(
        ui.painter(),
        icon_box(x0 + ICON_COL, rect.center().y),
        row.kind,
        theme::icon_color(ui.visuals()),
    );
    let Some(rename) = ui_state.renaming.as_mut() else { return };
    let field = Rect::from_min_max(
        pos2(x0 + 2.0 * ICON_COL + TEXT_GAP - 4.0, rect.top() + 1.0),
        pos2(rect.right() - RIGHT_PAD, rect.bottom() - 1.0),
    );
    let response = ui.put(
        field,
        egui::TextEdit::singleline(&mut rename.buffer).hint_text(row.generated.as_str()),
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
