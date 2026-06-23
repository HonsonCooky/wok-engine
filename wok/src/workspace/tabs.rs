//! The tab bar over the editor well: the app-menu hamburger at the left, then one cell per open scene
//! tab. Hand-drawn (egui has no tab widget) - a click on a tab selects it and its close glyph closes
//! it, decided by hit-testing the close rect so there are no overlapping click widgets; the active tab
//! borrows the editor surface and carries the accent top-line.

use crate::action::Action;
use crate::icons;
use crate::menu;
use crate::model::Model;
use crate::theme;

use super::TAB_BAR_HEIGHT;

/// The tab bar over the view column: the app-menu hamburger at the left (which opens the File / View /
/// Run / Help menu), then one cell per open tab (`model.shell.tabs`). With no tab open the bar is just
/// the hamburger. Hand-drawn (egui has no tab widget). A click on a tab selects it and its close glyph
/// closes it (`tab_cell`); the active tab (`model.shell.active_tab`) carries the active styling.
pub fn tab_bar(ctx: &egui::Context, model: &Model, actions: &mut Vec<Action>) {
    egui::TopBottomPanel::top("wok_tab_bar").exact_height(TAB_BAR_HEIGHT).show(ctx, |ui| {
        ui.horizontal_centered(|ui| {
            menu::hamburger(ui, model, actions);
            ui.add_space(8.0);
            // Tabs nearly touch, as in Zed, with the active fill the only thing parting them.
            ui.spacing_mut().item_spacing.x = 1.0;
            let active = model.shell.active_tab();
            for (i, tab) in model.shell.tabs().iter().enumerate() {
                tab_cell(ui, tab.title(), active == Some(i), i, actions);
            }
        });
    });
}

/// One tab cell: the title and a close glyph over the tab fill. The active tab borrows the editor
/// surface (so it reads continuous with the well below) and carries the accent as a top line; an
/// inactive tab sits flat and dim. The whole cell senses one click: a click landing on the close glyph
/// emits `CloseTab` (decided by hit-testing the glyph's rect, so there are no overlapping click widgets
/// racing for the press), any other click on the cell emits `SelectTab`.
fn tab_cell(ui: &mut egui::Ui, title: &str, active: bool, index: usize, actions: &mut Vec<Action>) {
    let p = theme::palette(ui.ctx());
    let fill = if active { p.editor_bg } else { egui::Color32::TRANSPARENT };
    let inner = egui::Frame::NONE.fill(fill).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
        ui.horizontal(|ui| {
            // The strip tightens item spacing for the tabs; restore a gap inside the cell so the close
            // glyph does not crowd the title.
            ui.spacing_mut().item_spacing.x = 6.0;
            let color = if active { p.text_bright } else { p.text_dim };
            let title = egui::RichText::new(title).color(color);
            let title = if active { title.strong() } else { title };
            // Non-selectable so the labels sense nothing - no click to fight the cell's, and no
            // text/IBeam cursor on hover (the cell below owns the interaction and sets the cursor).
            ui.add(egui::Label::new(title).selectable(false));
            // The close affordance, the same Nerd Font family as the rest of the chrome, sized small so
            // it sits quietly beside the title. Its rect is returned so a click landing on it closes the
            // tab rather than selecting it.
            let close = egui::RichText::new(icons::CLOSE).size(10.0).color(p.text_dim);
            ui.add(egui::Label::new(close).selectable(false)).rect
        })
        .inner
    });
    let close_rect = inner.inner;
    // One click-sensing region over the whole cell (the labels sense nothing), so close-vs-select is
    // decided by where the press landed, not by overlapping widgets fighting for it. The whole tab shows
    // the pointing-hand cursor (it is clickable), including over the close glyph.
    let cell = inner.response.interact(egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    if cell.clicked() {
        let on_close = cell.interact_pointer_pos().is_some_and(|pos| close_rect.contains(pos));
        actions.push(if on_close { Action::CloseTab(index) } else { Action::SelectTab(index) });
    }
    if active {
        let rect = inner.response.rect;
        ui.painter().hline(rect.x_range(), rect.top(), egui::Stroke::new(2.0, p.accent));
    }
}
