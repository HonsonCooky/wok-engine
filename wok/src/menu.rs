//! The editor chrome: the top menu bar, the bottom status bar, and the open-project dialog.
//!
//! The chrome reads the current project and emits [`Action`]s into the frame's buffer; it never
//! mutates project state itself (that is the handler's job, `crate::action`). The one thing it owns
//! outright is [`UiState`] - transient presentation flags like whether the open-project dialog is
//! up and the path being typed into it - which is not project state and is never saved.
//!
//! Folder picker: a native folder dialog needs a new dependency (rfd or similar), not yet approved,
//! so opening a project is by typed path for now - this dialog's text field, or the optional folder
//! argument at startup (`crate::cli`). The native picker drops into the dialog when the dependency
//! lands.

use std::path::{Path, PathBuf};

use crate::action::Action;
use crate::project::Project;

/// Status-bar height in points: one row of small text plus breathing room.
const STATUS_BAR_HEIGHT: f32 = 24.0;

/// Transient UI state the chrome owns: never saved, never project state.
#[derive(Default)]
pub struct UiState {
    /// Whether the open-project dialog is showing.
    pub open_dialog: bool,
    /// The path being typed into the open-project dialog.
    pub open_path: String,
}

/// Build the editor chrome for one frame: the menu bar, the status bar, and the open-project dialog
/// when it is up. Emits actions into `actions`; mutates only `state` (transient UI state). The
/// central region is left unpainted so the viewport clear shows through as the empty viewport.
pub fn ui(ctx: &egui::Context, project: &Project, state: &mut UiState, actions: &mut Vec<Action>) {
    menu_bar(ctx, state, actions);
    status_bar(ctx, project);
    open_dialog(ctx, state, actions);
}

/// The top menu bar. File menu in Zed's shape, trimmed to what exists: New Project (a stub), Open
/// Project, Open Recent (a stub - no recents tracked yet), and Quit.
fn menu_bar(ctx: &egui::Context, state: &mut UiState, actions: &mut Vec<Action>) {
    egui::TopBottomPanel::top("wok_menu_bar").show(ctx, |ui| {
        egui::menu::bar(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("New Project").clicked() {
                    actions.push(Action::NewProject);
                    ui.close_menu();
                }
                if ui.button("Open Project...").clicked() {
                    state.open_dialog = true;
                    ui.close_menu();
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

/// The open-project dialog: a typed-path field and an Open button, the no-dependency stand-in for a
/// native folder picker. Open is enabled only when the path names an existing directory, so a typo
/// is caught here rather than opening a project at a path with nothing in it; that directory check
/// lives in the chrome, keeping the handler pure. Open emits the action and closes; Cancel and the
/// window's own close button just close.
fn open_dialog(ctx: &egui::Context, state: &mut UiState, actions: &mut Vec<Action>) {
    if !state.open_dialog {
        return;
    }
    let mut open = true;
    let mut close = false;
    egui::Window::new("Open Project")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.label("Project folder:");
            ui.text_edit_singleline(&mut state.open_path);
            let path = state.open_path.trim();
            let is_dir = !path.is_empty() && Path::new(path).is_dir();
            if !path.is_empty() && !is_dir {
                let warn = ui.visuals().warn_fg_color;
                ui.colored_label(warn, "Not a folder on disk.");
            }
            ui.label(
                egui::RichText::new(
                    "Paste or type a folder path. A native picker arrives with the file-dialog dependency.",
                )
                .weak()
                .small(),
            );
            ui.separator();
            ui.horizontal(|ui| {
                if ui.add_enabled(is_dir, egui::Button::new("Open")).clicked() {
                    actions.push(Action::OpenProject(PathBuf::from(path)));
                    close = true;
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });
    if !open || close {
        state.open_dialog = false;
    }
}
