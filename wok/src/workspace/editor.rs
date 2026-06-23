//! The editor well: the central region left after the navigation panel, the tab bar, and the status
//! bar claim their strips. A transparent panel over the editor-background backdrop, so it reads as the
//! empty well; with a tab open it names the active scene as a stand-in for the per-context surface (the
//! 3D viewport, the data views) that lands in later bites, and a click on the empty well deselects.

use crate::action::Action;
use crate::model::Model;
use crate::theme;

/// The editor area: the active tab's placeholder, or an empty well when no tab is open. A transparent
/// panel over the editor-background backdrop (the GPU clear in the live app, the snapshot harness's
/// background fill in the test), so the well reads as `editor_bg`. With a tab open it names the open
/// scene, dim and centred - the stand-in for the per-context surface (the 3D viewport, the data views),
/// which lands in later bites, drawn into this same transparent panel; with no tab open it is the bare
/// well.
///
/// A click on the empty well clears the selection (editor-design.md: a click on empty space deselects;
/// viewport picking that selects on a hit lands with the 3D, a later bite). The floating inspector is on
/// a higher layer, so a click landing on it does not reach here - egui assigns the click to the topmost
/// area, so the well's `clicked()` is false under the window.
pub fn editor_area(ctx: &egui::Context, model: &Model, actions: &mut Vec<Action>) {
    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
        let well = ui.interact(ui.max_rect(), ui.id().with("editor_well"), egui::Sense::click());
        if well.clicked() && model.shell.selection().is_some() {
            actions.push(Action::Deselect);
        }
        let Some(tab) = model.shell.active_tab().and_then(|i| model.shell.tabs().get(i)) else {
            return;
        };
        let p = theme::palette(ui.ctx());
        let center = ui.max_rect().center();
        // The open scene's name over a one-line hint that the real surface is still to come, both dim
        // and centred on the well. Painted directly (like the nav rows) rather than laid out, so the
        // block sits at the centre regardless of the panel size; the name sits just above the centre
        // line and the hint just below.
        let name = egui::FontId::proportional(20.0);
        let hint = egui::FontId::proportional(12.0);
        let painter = ui.painter();
        painter.text(center, egui::Align2::CENTER_BOTTOM, tab.title(), name, p.text_dim);
        painter.text(center + egui::vec2(0.0, 6.0), egui::Align2::CENTER_TOP, "viewport lands here", hint, p.text_dim);
    });
}
