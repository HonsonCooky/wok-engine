//! The editor well: the central region left after the navigation panel, the tab bar, and the status
//! bar claim their strips. A transparent `CentralPanel` so wok-render's 3D viewport (drawn behind the
//! egui pass, scoped to this rect - `crate::render`) reads through it; with no scene drawing it is the
//! bare `editor_bg` clear.
//!
//! Render-only baseline: the well senses nothing and raises no gestures - the interaction layer was
//! demolished. It exists to claim the central rect (captured in `view::chrome` and handed to the render),
//! so the 3D lands exactly here. The viewport-input workflow (the first rebuild bite) wires click and
//! drag back in through the frame loop.

/// The editor area: a transparent `CentralPanel` over whatever the renderer drew into this rect - the
/// open scene's 3D viewport when one is loaded (`crate::render`/`crate::render_scene`), or the
/// `editor_bg` clear (the empty well) otherwise. The panel paints nothing of its own and senses nothing;
/// the rect it settles into is captured in `view::chrome` and handed to the render so the 3D lands here.
pub fn editor_area(ctx: &egui::Context) {
    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |_ui| {});
}
