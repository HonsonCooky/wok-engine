//! The editor chrome: the top menu bar and the bottom status bar.
//!
//! The chrome reads the current project and emits [`Action`]s into the frame's buffer; it never
//! mutates project state itself (that is the handler's job, `crate::action`). Opening a project
//! goes through the OS-native folder picker (rfd): the File menu calls it synchronously - a modal
//! folder pick that blocks the UI thread for its duration, which is expected of a native dialog -
//! and emits [`Action::OpenProject`] with the chosen folder, or nothing when the pick is cancelled.
//! The picker only ever returns an existing folder, so there is nothing to validate here and the
//! handler stays a pure path-in writer.

use crate::action::Action;
use crate::project::Project;

/// Status-bar height in points: one row of small text plus breathing room.
const STATUS_BAR_HEIGHT: f32 = 24.0;

/// Build the editor chrome for one frame: the menu bar and the status bar. Emits actions into
/// `actions`. The central region is left unpainted so the viewport clear shows through as the empty
/// viewport.
pub fn ui(ctx: &egui::Context, project: &Project, actions: &mut Vec<Action>) {
    menu_bar(ctx, actions);
    status_bar(ctx, project);
}

/// The top menu bar. File menu in Zed's shape, trimmed to what exists: New Project (a stub), Open
/// Project, Open Recent (a stub - no recents tracked yet), and Quit.
fn menu_bar(ctx: &egui::Context, actions: &mut Vec<Action>) {
    egui::TopBottomPanel::top("wok_menu_bar").show(ctx, |ui| {
        egui::menu::bar(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("New Project").clicked() {
                    actions.push(Action::NewProject);
                    ui.close_menu();
                }
                if ui.button("Open Project...").clicked() {
                    ui.close_menu();
                    // Synchronous native folder picker: blocks until the user chooses or cancels.
                    // A chosen folder opens; a cancelled pick (None) leaves the current project be.
                    if let Some(folder) = rfd::FileDialog::new().pick_folder() {
                        actions.push(Action::OpenProject(folder));
                    }
                }
                ui.menu_button("Open Recent", |ui| {
                    ui.add_enabled(false, egui::Button::new("(no recent projects)"));
                });
                ui.separator();
                if ui.button("Quit").clicked() {
                    actions.push(Action::Quit);
                    ui.close_menu();
                }
            });
        });
    });
}

/// The bottom status bar. Shows the open project's name, or that none is open - the in-window
/// confirmation that Open Project took effect (the title bar carries the same).
fn status_bar(ctx: &egui::Context, project: &Project) {
    egui::TopBottomPanel::bottom("wok_status_bar").exact_height(STATUS_BAR_HEIGHT).show(ctx, |ui| {
        ui.horizontal_centered(|ui| match project.display_name() {
            Some(name) => ui.label(egui::RichText::new(name).small()),
            None => ui.label(egui::RichText::new("No project open").weak().small()),
        });
    });
}
