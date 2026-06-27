//! The editor well: the central region left after the navigation panel, the tab bar, and the status
//! bar claim their strips. A transparent `CentralPanel` so wok-render's 3D viewport (drawn behind the
//! egui pass, scoped to this rect - `crate::render`) reads through it; with no scene drawing it is the
//! bare `editor_bg` clear. The well senses click-and-drag, raising the viewport pointer gestures the
//! frame loop resolves into selection and drag-and-drop moves (`crate::interaction`).

use glam::Vec2;

use crate::action::{Action, Gesture};

/// The editor area: a transparent `CentralPanel` over whatever the renderer drew into this rect - the
/// open scene's 3D viewport when one is loaded (`crate::render`/`crate::render_scene`), or the
/// `editor_bg` clear (the empty well) otherwise. The panel paints nothing of its own; the rect it
/// settles into is captured in `view::chrome` and handed to the render so the 3D lands exactly here.
///
/// It senses [`click_and_drag`](egui::Sense::click_and_drag) and raises one [`Gesture`] per frame as an
/// [`Action::ViewportGesture`], which the frame loop maps against this same well rect and resolves
/// (`crate::interaction`): a click selects the instance under the cursor (or deselects on empty), and a
/// drag grabs that instance and drops it on the surface under the cursor - the drag-and-drop move. The
/// model mutation still goes through the single writer. egui assigns the gesture to the topmost area, so
/// a press on the floating inspector or a menu (higher layers) leaves the well's flags false and never
/// reaches here.
pub fn editor_area(ctx: &egui::Context, actions: &mut Vec<Action>) {
    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
        let well = ui.interact(ui.max_rect(), ui.id().with("editor_well"), egui::Sense::click_and_drag());
        // The pointer position relevant to this interaction - set while pressed, dragging, or on the
        // click frame. A click is a press-release with no drag; drag_started precedes the drag frames.
        if let Some(pos) = well.interact_pointer_pos() {
            let p = Vec2::new(pos.x, pos.y);
            if well.clicked() {
                actions.push(Action::ViewportGesture(Gesture::Click(p)));
            } else if well.drag_started() {
                actions.push(Action::ViewportGesture(Gesture::GrabStart(p)));
            } else if well.dragged() {
                actions.push(Action::ViewportGesture(Gesture::GrabMove(p)));
            }
        }
        // The release frame after a drag: drop. Separate from the position block, since the release
        // carries no interaction pointer position of its own.
        if well.drag_stopped() {
            actions.push(Action::ViewportGesture(Gesture::GrabEnd));
        }
    });
}
