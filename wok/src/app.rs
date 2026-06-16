//! The editor application: the window state and the per-frame loop.
//!
//! Shell frame: the chrome (`crate::menu`, the menu and status bars) and the workspace
//! (`crate::workspace`, the navigation panel, tab bar, and editor area) paint over a flat viewport
//! clear, each reading the model and emitting actions the loop applies through the one handler
//! (`crate::action`, the single writer for the model). The frame clears the viewport, runs the egui
//! pass, applies its actions, then paints the UI; the editor area stays transparent so the clear
//! shows through where the 3D view lands later. The authoring surfaces - scene model, selection,
//! camera, render - return as later pieces and slot into this loop.

use std::path::PathBuf;

use wok_platform::winit::event::WindowEvent;
use wok_platform::{App, FrameCtx, Platform, gfx};

use crate::action::{self, Action};
use crate::gui::Gui;
use crate::model::Model;
use crate::project::Project;
use crate::theme;
use crate::view;

/// Flat viewport background, shown wherever the chrome does not paint. A neutral dark, so an empty
/// viewport reads as "nothing here yet" rather than a specific lit scene; not theme-aware, since it
/// is a GPU clear rather than an egui surface.
const VIEWPORT_CLEAR: (f64, f64, f64) = (0.09, 0.10, 0.12);

pub struct EditorApp {
    /// The editor state the action layer writes and the view reads: the open project and the shell.
    model: Model,
    /// egui integration, built once a GPU device exists (`init`).
    gui: Option<Gui>,
    /// The window title last set, so it is only pushed to the OS when it changes.
    title: String,
}

impl EditorApp {
    /// Build the app. An optional startup folder (from the CLI) opens as the initial project; with
    /// none, the editor starts with no project open.
    pub fn new(initial: Option<PathBuf>) -> EditorApp {
        let project = match initial {
            Some(root) => Project::open(root),
            None => Project::None,
        };
        EditorApp { model: Model::new(project), gui: None, title: String::new() }
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
        // order is layout order - menu (top) and status (bottom), then the workspace fits the
        // navigation panel, tab bar, and editor area into what is left.
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

        // Apply the actions through the one handler - the single writer for the model. Quit is the
        // one effect the pure state cannot perform, so the loop carries it out here.
        for action in actions {
            if action::handle(action, &mut self.model).quit {
                ctx.should_close = true;
            }
        }

        // Clear the viewport to the flat background, then paint the UI over it. The editor area's
        // panel is transparent, so the clear shows through there as the empty viewport.
        if let Some(mut frame) = gfx::begin_frame(ctx.platform) {
            let (r, g, b) = VIEWPORT_CLEAR;
            frame.clear(r, g, b);
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
