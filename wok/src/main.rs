//! wok: the engine's reference editor application.
//!
//! This is the editor shell over the platform frame loop (see designs/editor-design.md "The shell"
//! and designs/design_handoff_editor_surfaces): the egui chrome's five regions - the full-height
//! navigation panel and its bottom icon bar, the tab bar with the app-menu hamburger, the editor well,
//! and the status bar - drawn through one composition root (`view::chrome`). The chrome reads a
//! [`Model`] and emits [`Action`](action::Action)s; this frame loop drains them through the single
//! writer, `action::handle` (see `crate::action`), so the model has exactly one mutation point. The
//! loop also carries out the effects the pure handler cannot: it runs the folder-open validation
//! (filesystem I/O, `crate::project`) before applying an open and surfaces a failure, persists the
//! recent-projects list when it changes (`crate::recent`), and keeps the window title on the open
//! project. The per-view content (the nav and editor area) is a later slice.
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
mod project;
mod recent;
mod theme;
mod view;
mod workspace;

use action::Action;
use gui::Gui;
use model::Model;
use wok_platform::winit::event::WindowEvent;
use wok_platform::{App, Desc, FrameCtx, Platform, gfx};

struct Editor {
    /// egui integration, built once a GPU device exists (`init`).
    gui: Option<Gui>,
    /// The editor state the chrome reads and the action layer writes - the single writer's model: the
    /// open project, the recent-projects list, and the shell layout.
    model: Model,
    /// The last project-open failure's message (a folder that was not a wok project), surfaced in the
    /// status bar until the next successful open. A failed open leaves any current project intact (the
    /// validation runs before the open is applied), so it only reports the failed attempt; it never
    /// drops the open project. App-side, not `Model` state: it comes from the frame loop's filesystem
    /// validation, not a pure action.
    open_error: Option<String>,
    /// The window title last pushed to the OS, so `set_title` fires only when it changes.
    title: String,
}

impl Editor {
    fn new() -> Editor {
        // Seed the recent-projects list from disk (a missing or malformed file reads as empty), then
        // reopen the most-recent recent that is still a loadable wok project, so a relaunch returns to
        // where it left off. A deleted or emptied recent is skipped, and an empty list (a fresh
        // install or a cleared list) starts with no project. Reopen-last is routed through the single
        // writer the same way a menu open is, and the reordered recents are persisted.
        let mut model = Model { recents: recent::load(), ..Model::default() };
        if let Some(root) = project::pick_startup(&model.recents, project::is_wok_project) {
            if action::handle(&mut model, Action::OpenProject(root)).save_recents {
                recent::save(&model.recents);
            }
        }
        Editor { gui: None, model, open_error: None, title: String::new() }
    }

    /// Keep the window title showing the open project's name (`wok - {name}`), or just `wok` when none
    /// is open. Pushed to the OS only when it changes, so it is cheap to call every frame.
    fn refresh_title(&mut self, platform: &Platform) {
        let title = match self.model.project.as_ref() {
            Some(project) => format!("wok - {}", project.name()),
            None => "wok".to_string(),
        };
        if title != self.title {
            platform.window.set_title(&title);
            self.title = title;
        }
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
        // The chrome reads the last open error too; clone it out so the action drain below can rewrite
        // self.open_error without the closure still borrowing it.
        let open_error = self.open_error.clone();

        // Build the chrome for this frame, reading the model. The regions emit actions into a buffer
        // rather than mutating the model inside their egui closures.
        let mut actions = Vec::new();
        let output = gui.run(&ctx.platform.window, |egui_ctx| {
            actions.extend(view::chrome(egui_ctx, model, open_error.as_deref()));
        });
        // Drain the buffer through the single writer: click -> Action -> handle, and the next frame
        // re-renders the new state. Opening a project is split across the seam: validating that the
        // picked folder is a wok project is filesystem I/O, so it runs here in the loop, never in the
        // pure handler, and the open is applied only on success - so a recent is recorded for a folder
        // that really opened, and a non-wok folder surfaces an error and changes nothing. The handler
        // returns the effects it cannot perform itself (persisting the recent-projects list).
        for action in actions {
            match action {
                Action::OpenProject(root) => match project::open(&root) {
                    Ok(_) => {
                        if action::handle(model, Action::OpenProject(root)).save_recents {
                            recent::save(&model.recents);
                        }
                        self.open_error = None;
                    }
                    Err(err) => self.open_error = Some(err.to_string()),
                },
                other => {
                    if action::handle(model, other).save_recents {
                        recent::save(&model.recents);
                    }
                }
            }
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

        // Keep the window title on the open project's name (or just the app name when none).
        self.refresh_title(ctx.platform);
    }

    fn cleanup(&mut self, _platform: &Platform) {}
}

fn main() {
    wok_platform::run(Editor::new(), Desc { title: "wok", width: 0, height: 0, vsync: true });
}
