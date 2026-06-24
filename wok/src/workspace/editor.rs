//! The editor well: the central region left after the navigation panel, the tab bar, and the status
//! bar claim their strips. A transparent `CentralPanel` so wok-render's 3D viewport (drawn behind the
//! egui pass, scoped to this rect - `crate::render`) reads through it; with no scene drawing it is the
//! bare `editor_bg` clear. A click on the empty well deselects.

use crate::action::Action;
use crate::model::Model;

/// The editor area: a transparent `CentralPanel` over whatever the renderer drew into this rect - the
/// open scene's 3D viewport when one is loaded (`crate::render`/`crate::render_scene`), or the
/// `editor_bg` clear (the empty well) otherwise. The panel paints nothing of its own; the rect it
/// settles into is captured in `view::chrome` and handed to the render so the 3D lands exactly here.
///
/// A click on the empty well clears the selection (editor-design.md: a click on empty space deselects;
/// viewport picking that selects on a hit lands with the 3D's picking, a later bite). The floating
/// inspector is on a higher layer, so a click landing on it does not reach here - egui assigns the
/// click to the topmost area, so the well's `clicked()` is false under the window.
pub fn editor_area(ctx: &egui::Context, model: &Model, actions: &mut Vec<Action>) {
    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
        let well = ui.interact(ui.max_rect(), ui.id().with("editor_well"), egui::Sense::click());
        if well.clicked() && model.shell.selection().is_some() {
            actions.push(Action::Deselect);
        }
    });
}
