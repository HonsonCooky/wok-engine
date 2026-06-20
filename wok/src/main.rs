//! wok: the engine's reference editor application.
//!
//! This is the editor shell over the platform frame loop (see designs/editor-design.md "The shell"
//! and designs/design_handoff_editor_surfaces): the egui chrome's five regions - the full-height
//! navigation panel and its bottom icon bar, the tab bar with the app-menu hamburger, the editor well,
//! and the status bar - drawn through one composition root (`view::chrome`). The chrome reads a
//! [`Model`] and emits [`Action`](action::Action)s; this frame loop drains them through the single
//! writer, `action::handle` (see `crate::action`), so the model has exactly one mutation point. This
//! slice wires that seam for one action - the icon bar switching the active navigation view; the
//! tab/dock/menu behaviors and the per-view content are later slices.
//!
//! The frame loop is the platform's `gfx::begin_frame -> draw -> Frame::finish`. Each frame runs the
//! egui pass (building the chrome), clears the surface to the editor background, then paints the chrome
//! over it as the final pass. The clear and the snapshot harness's background fill use the same
//! `theme::palette(ctx).editor_bg`, so the transparent editor well reads identically live and in the
//! snapshot. Sizing comes from `Frame::size()` (the acquired surface texture), never a separately
//! tracked window size - see designs/sharp-edges.md section 1.

mod action;
mod gui;
mod icons;
mod menu;
mod model;
mod theme;
mod view;
mod workspace;

use gui::Gui;
use model::Model;
use wok_platform::winit::event::WindowEvent;
use wok_platform::{App, Desc, FrameCtx, Platform, gfx};

struct Editor {
    /// egui integration, built once a GPU device exists (`init`).
    gui: Option<Gui>,
    /// The editor state the chrome reads and the action layer writes - the single writer's model.
    model: Model,
}

impl Editor {
    fn new() -> Editor {
        Editor { gui: None, model: Model::default() }
    }
}

impl App for Editor {
    fn init(&mut self, platform: &Platform) {
        // The OS owns the title bar, window drag, resize, and the min/max/close buttons; the editor
        // draws only its client area. Build egui and apply the editor theme (which follows the OS
        // light/dark from here on).
        let gui = Gui::new(platform);
        theme::apply(&gui.ctx);
        self.gui = Some(gui);
    }

    fn on_window_event(&mut self, platform: Option<&Platform>, event: &WindowEvent) {
        if let (Some(platform), Some(gui)) = (platform, self.gui.as_mut()) {
            gui.on_event(&platform.window, event);
        }
    }

    fn frame(&mut self, ctx: &mut FrameCtx) {
        let Some(gui) = self.gui.as_mut() else { return };
        let model = &mut self.model;

        // Build the chrome for this frame, reading the model. The regions emit actions into a buffer
        // rather than mutating the model inside their egui closures.
        let mut actions = Vec::new();
        let output = gui.run(&ctx.platform.window, |egui_ctx| actions.extend(view::chrome(egui_ctx, model)));
        // Drain the buffer through the single writer: click -> Action -> handle, and the next frame
        // re-renders the new state. This is the one path the model changes by.
        for action in actions {
            action::handle(model, action);
        }
        // The editor well is a transparent egui panel, so the surface clear behind it is the well's
        // colour: clear to the active theme's editor background. The surface is sRGB and wgpu reads the
        // clear value as linear, so decode through Rgba.
        let editor_bg = egui::Rgba::from(theme::palette(&gui.ctx).editor_bg);

        let Some(mut frame) = gfx::begin_frame(ctx.platform) else { return };
        frame.clear(editor_bg.r().into(), editor_bg.g().into(), editor_bg.b().into());
        // Paint the chrome over the clear, sized from the acquired texture (the one authoritative size
        // for this frame), never a separately tracked window size that can race ahead mid-resize.
        let size = frame.size();
        gui.paint(ctx.platform, &mut frame.encoder, &frame.view, output, size);
        frame.finish(ctx.platform);
    }

    fn cleanup(&mut self, _platform: &Platform) {}
}

fn main() {
    wok_platform::run(Editor::new(), Desc { title: "wok", width: 0, height: 0, vsync: true });
}
