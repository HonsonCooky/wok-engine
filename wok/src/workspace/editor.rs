//! The editor well: the central region left after the navigation panel, the tab bar, and the status
//! bar claim their strips. A transparent `CentralPanel` so wok-render's 3D viewport (drawn behind the
//! egui pass, scoped to this rect - `crate::render`) reads through it; with no scene drawing it is the
//! bare `editor_bg` clear. A left-click in the well casts a pick ray (resolved by the frame loop).

use glam::Vec2;

use crate::action::Action;

/// The editor area: a transparent `CentralPanel` over whatever the renderer drew into this rect - the
/// open scene's 3D viewport when one is loaded (`crate::render`/`crate::render_scene`), or the
/// `editor_bg` clear (the empty well) otherwise. The panel paints nothing of its own; the rect it
/// settles into is captured in `view::chrome` and handed to the render so the 3D lands exactly here.
///
/// A genuine left-click emits [`Action::ViewportClick`] with the click position; the frame loop maps
/// it against this same well rect, casts the cursor ray, and selects the nearest instance under it or
/// deselects on empty space or terrain (the model mutation still goes through Select/Deselect via the
/// single writer). `Sense::click()` never fires on a drag, so a right/middle camera drag - or a future
/// left-drag marquee - never picks. The floating inspector is on a higher layer, so a click landing on
/// it does not reach here - egui assigns the click to the topmost area, so the well's `clicked()` is
/// false under the window.
pub fn editor_area(ctx: &egui::Context, actions: &mut Vec<Action>) {
    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
        let well = ui.interact(ui.max_rect(), ui.id().with("editor_well"), egui::Sense::click());
        if well.clicked() {
            if let Some(pos) = well.interact_pointer_pos() {
                actions.push(Action::ViewportClick(Vec2::new(pos.x, pos.y)));
            }
        }
    });
}
