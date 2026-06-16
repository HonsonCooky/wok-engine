//! The editor application: the window state and the per-frame loop.
//!
//! Shell frame: the chrome (`crate::menu`, the hamburger app-menu and the status bar) and the
//! workspace (`crate::workspace`, the navigation panel, tab bar, and editor area) paint over the
//! editor-surface viewport clear, each reading the model and emitting actions the loop applies
//! through the one handler (`crate::action`, the single writer for the model). The OS owns the title
//! bar - drag, resize, min/max/close - so the editor draws only its client area. The frame clears the
//! viewport, runs the egui pass, applies its actions, then paints the UI; the editor area stays
//! transparent so the clear shows through where the 3D view lands later. The authoring surfaces -
//! scene model, selection, camera, render - return as later pieces and slot into this loop.

use std::path::PathBuf;

use wok_platform::winit::event::WindowEvent;
use wok_platform::{App, FrameCtx, Platform, gfx};

use crate::action::{self, Action};
use crate::gui::Gui;
use crate::model::Model;
use crate::project::Project;
use crate::recent;
use crate::theme;
use crate::view;

pub struct EditorApp {
    /// The editor state the action layer writes and the view reads: the open project and the shell.
    model: Model,
    /// egui integration, built once a GPU device exists (`init`).
    gui: Option<Gui>,
    /// The window title last set, so it is only pushed to the OS when it changes.
    title: String,
}

impl EditorApp {
    /// Build the app. The recent-projects list is seeded from disk (a missing or malformed file reads
    /// as empty, so the editor always starts). An optional startup folder (from the CLI) opens as the
    /// initial project, routed through the one writer so it lands in recents the way a menu open does
    /// and is persisted at once; with none, the editor starts with no project open.
    pub fn new(initial: Option<PathBuf>) -> EditorApp {
        let mut model = Model::new(Project::None);
        model.recents = recent::load();
        if let Some(root) = initial {
            if action::handle(Action::OpenProject(root), &mut model).save_recents {
                recent::save(&model.recents);
            }
        }
        EditorApp { model, gui: None, title: String::new() }
    }

    /// Keep the window title showing the open project's name, or just the app name when none.
    fn refresh_title(&mut self, platform: &Platform) {
        let title = match self.model.project.display_name() {
            Some(name) => format!("wok - {name}"),
            None => "wok".to_string(),
        };
        if title != self.title {
            platform.window.set_title(&title);
            self.title = title;
        }
    }
}

impl App for EditorApp {
    fn init(&mut self, platform: &Platform) {
        // The OS provides the title bar, window drag, resize, and the min/max/close buttons; the
        // editor draws only its client area. The window title shows the open project.
        let gui = Gui::new(platform);
        theme::apply(&gui.ctx);
        self.gui = Some(gui);
        self.refresh_title(platform);
    }

    fn on_window_event(&mut self, platform: Option<&Platform>, event: &WindowEvent) {
        if let (Some(platform), Some(gui)) = (platform, self.gui.as_mut()) {
            gui.on_event(&platform.window, event);
        }
    }

    fn frame(&mut self, ctx: &mut FrameCtx) {
        // Run the egui pass: the regions read the model and emit actions into the buffer. Panel
        // order is layout order - the status bar (bottom), then the workspace fits the navigation
        // panel, tab bar (with the app-menu at its left), and editor area into what is left.
        let mut actions: Vec<Action> = Vec::new();
        let mut ui_output = None;
        {
            let model = &self.model;
            if let Some(gui) = self.gui.as_mut() {
                ui_output = Some(gui.run(&ctx.platform.window, |egui_ctx| {
                    view::chrome(egui_ctx, model, &mut actions);
                }));
            }
        }

        // Apply the actions through the one handler - the single writer for the model. The effects the
        // pure state cannot perform itself are carried out here: quit closes the window, and a recents
        // change is flushed to disk.
        for action in actions {
            let handled = action::handle(action, &mut self.model);
            if handled.quit {
                ctx.should_close = true;
            }
            if handled.save_recents {
                recent::save(&self.model.recents);
            }
        }

        // Clear the viewport to the active theme's editor surface, then paint the UI over it. The
        // editor area's panel is transparent, so the clear shows through as the empty viewport and
        // follows the OS light/dark with the chrome. The surface is sRGB and wgpu reads the clear
        // value as linear, so decode the color through egui::Rgba; the sRGB surface re-encodes it.
        if let Some(mut frame) = gfx::begin_frame(ctx.platform) {
            let editor_bg = self.gui.as_ref().map_or(egui::Color32::BLACK, |g| theme::palette(&g.ctx).editor_bg);
            let clear = egui::Rgba::from(editor_bg);
            frame.clear(clear.r().into(), clear.g().into(), clear.b().into());
            if let (Some(gui), Some(output)) = (self.gui.as_mut(), ui_output) {
                let size = (ctx.width.max(1), ctx.height.max(1));
                gui.paint(ctx.platform, &mut frame.encoder, &frame.view, output, size);
            }
            frame.finish(ctx.platform);
        }

        self.refresh_title(ctx.platform);
    }

    fn cleanup(&mut self, _platform: &Platform) {}
}
