//! The editor chrome: the top menu bar and the bottom status bar.
//!
//! The chrome reads the current model and emits [`Action`]s into the frame's buffer; it never
//! mutates state itself (that is the handler's job, `crate::action`). Opening a project goes through
//! the OS-native folder picker (rfd): the File menu calls it synchronously - a modal folder pick
//! that blocks the UI thread for its duration, which is expected of a native dialog - and emits
//! [`Action::OpenProject`] with the chosen folder, or nothing when the pick is cancelled. The View
//! menu drives the navigation panel (show/hide, dock side), the same actions Ctrl+B and a future
//! keybind table dispatch.

use crate::action::Action;
use crate::model::{Shell, Side};
use crate::project::Project;

/// Status-bar height in points: one row of small text plus breathing room.
const STATUS_BAR_HEIGHT: f32 = 24.0;

/// The Ctrl+B (Cmd+B on macOS) shortcut that toggles the navigation panel. Built in one place so
/// the menu's hint and the global handler always agree on the binding.
fn nav_toggle_shortcut() -> egui::KeyboardShortcut {
    egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::B)
}

/// The top menu bar: File (project lifecycle) and View (the navigation panel). Also consumes the
/// global panel-toggle shortcut, so Ctrl+B works whether or not the View menu is open.
pub fn menu_bar(ctx: &egui::Context, shell: &Shell, actions: &mut Vec<Action>) {
    let toggle = nav_toggle_shortcut();
    if ctx.input_mut(|i| i.consume_shortcut(&toggle)) {
        actions.push(Action::ToggleNav);
    }
    egui::TopBottomPanel::top("wok_menu_bar").show(ctx, |ui| {
        egui::menu::bar(ui, |ui| {
            file_menu(ui, actions);
            view_menu(ui, shell, toggle, actions);
        });
    });
}

/// The File menu in Zed's shape, trimmed to what exists: New Project (a stub), Open Project, Open
/// Recent (a stub - no recents tracked yet), and Quit.
fn file_menu(ui: &mut egui::Ui, actions: &mut Vec<Action>) {
    ui.menu_button("File", |ui| {
        if ui.button("New Project").clicked() {
            actions.push(Action::NewProject);
            ui.close_menu();
        }
        if ui.button("Open Project...").clicked() {
            ui.close_menu();
            // Synchronous native folder picker: blocks until the user chooses or cancels. A chosen
            // folder opens; a cancelled pick (None) leaves the current project be.
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
}

/// The View menu: show or hide the navigation panel (with the Ctrl+B hint), and flip which side it
/// docks to. The dock item names its destination, so one item flips the side either way.
fn view_menu(ui: &mut egui::Ui, shell: &Shell, toggle: egui::KeyboardShortcut, actions: &mut Vec<Action>) {
    ui.menu_button("View", |ui| {
        let toggle_label = if shell.nav_visible() { "Hide Navigation Panel" } else { "Show Navigation Panel" };
        let hint = ui.ctx().format_shortcut(&toggle);
        if ui.add(egui::Button::new(toggle_label).shortcut_text(hint)).clicked() {
            actions.push(Action::ToggleNav);
            ui.close_menu();
        }
        let (side_label, target) = match shell.nav_side() {
            Side::Left => ("Dock Panel Right", Side::Right),
            Side::Right => ("Dock Panel Left", Side::Left),
        };
        if ui.button(side_label).clicked() {
            actions.push(Action::SetNavSide(target));
            ui.close_menu();
        }
    });
}

/// The bottom status bar. Shows the open project's name, or that none is open - the in-window
/// confirmation that Open Project took effect (the title bar carries the same).
pub fn status_bar(ctx: &egui::Context, project: &Project) {
    egui::TopBottomPanel::bottom("wok_status_bar").exact_height(STATUS_BAR_HEIGHT).show(ctx, |ui| {
        ui.horizontal_centered(|ui| match project.display_name() {
            Some(name) => ui.label(egui::RichText::new(name).small()),
            None => ui.label(egui::RichText::new("No project open").weak().small()),
        });
    });
}
