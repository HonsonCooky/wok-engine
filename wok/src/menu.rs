//! The editor chrome's menu and status bar.
//!
//! The OS owns the title bar (`crate::app`), so the editor's menu is a single hamburger button at the
//! left of the tab-bar row - always visible, since the tab bar always is, unlike the toggleable
//! navigation panel. The button opens an app-menu with File and View as submenus; this is Zed's
//! grammar, not a horizontal menu bar. Like all the view, the chrome reads the model and emits
//! actions; the handler (`crate::action`) is the single writer.
//!
//! Opening a project goes through the OS-native folder picker (rfd): the File menu calls it
//! synchronously - a modal pick that blocks the UI thread for its duration, expected of a native
//! dialog - and emits [`Action::OpenProject`], or nothing when cancelled. The View menu drives the
//! navigation panel (show/hide, dock side), the same actions Ctrl+B dispatches.

use crate::action::Action;
use crate::model::{Model, Shell, Side};
use crate::project::{self, Project};
use crate::recent::Recents;
use crate::theme;

/// Status-bar height in points: one row of small text plus breathing room.
const STATUS_BAR_HEIGHT: f32 = 24.0;

/// Warning text for a surfaced failure (a failed project open). A muted red, fixed rather than
/// themed: it reads against both the light and dark status-bar surface, and a single transient notice
/// does not earn a per-theme palette entry.
const WARN_COLOR: egui::Color32 = egui::Color32::from_rgb(0xd0, 0x5a, 0x4a);

/// Size of the hamburger button cell, in points.
const HAMBURGER_SIZE: egui::Vec2 = egui::vec2(30.0, 22.0);

/// Half-width of the painted hamburger bars, and the vertical gap between them.
const HAMBURGER_HALF: f32 = 7.0;
const HAMBURGER_GAP: f32 = 4.0;

/// The Ctrl+B (Cmd+B on macOS) shortcut that toggles the navigation panel. Built in one place so the
/// menu's hint and the global handler always agree on the binding.
fn nav_toggle_shortcut() -> egui::KeyboardShortcut {
    egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::B)
}

/// The app-menu, behind a single hamburger button (drawn by the caller into the tab-bar row). The
/// button opens a menu with File and View as submenus. Also consumes the global Ctrl+B, so the toggle
/// works whether or not the menu is open. The painted glyph carries an accessible "Menu" label so
/// tooling (and the snapshot test) can find it.
pub fn hamburger(ui: &mut egui::Ui, model: &Model, actions: &mut Vec<Action>) {
    let toggle = nav_toggle_shortcut();
    if ui.ctx().input_mut(|i| i.consume_shortcut(&toggle)) {
        actions.push(Action::ToggleNav);
    }
    let button = egui::Button::new("").min_size(HAMBURGER_SIZE);
    let response = egui::menu::menu_custom_button(ui, button, |ui| {
        // Let items size to their text instead of wrapping in a narrow menu.
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        file_menu(ui, &model.recents, &model.project, actions);
        view_menu(ui, &model.shell, toggle, actions);
    })
    .response
    .on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Menu"));
    let p = theme::palette(ui.ctx());
    let color = if response.hovered() { p.text_bright } else { p.text_dim };
    paint_hamburger(ui.painter(), response.rect, color);
}

/// Paint the three-bar hamburger glyph centred in `rect`.
fn paint_hamburger(painter: &egui::Painter, rect: egui::Rect, color: egui::Color32) {
    let c = rect.center();
    let stroke = egui::Stroke::new(1.5, color);
    for dy in [-HAMBURGER_GAP, 0.0, HAMBURGER_GAP] {
        painter.hline(c.x - HAMBURGER_HALF..=c.x + HAMBURGER_HALF, c.y + dy, stroke);
    }
}

/// The File menu: New Project (a stub until project creation returns), Open Project (the native
/// folder picker), Open Recent (the persisted MRU list), Close Project, and Quit. Close Project is
/// disabled with no project open, so the menu never offers an action that would do nothing.
fn file_menu(ui: &mut egui::Ui, recents: &Recents, project: &Project, actions: &mut Vec<Action>) {
    ui.menu_button("File", |ui| {
        // Let items size to their text instead of wrapping in a narrow menu.
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
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
        open_recent_menu(ui, recents, actions);
        if ui.add_enabled(project.root().is_some(), egui::Button::new("Close Project")).clicked() {
            actions.push(Action::CloseProject);
            ui.close_menu();
        }
        ui.separator();
        if ui.button("Quit").clicked() {
            actions.push(Action::Quit);
            ui.close_menu();
        }
    });
}

/// The Open Recent submenu: the recent projects most-recent first, each reopening through the same
/// [`Action::OpenProject`] the picker emits, then Clear Recent. A disabled placeholder stands in when
/// nothing has been opened yet, so the entry still reads as present. Each item shows the folder's own
/// name, with the full path on hover to tell same-named folders apart.
fn open_recent_menu(ui: &mut egui::Ui, recents: &Recents, actions: &mut Vec<Action>) {
    ui.menu_button("Open Recent", |ui| {
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        if recents.is_empty() {
            ui.add_enabled(false, egui::Button::new("(no recent projects)"));
            return;
        }
        for path in recents.paths() {
            let label = project::display_name_of(path);
            if ui.button(label).on_hover_text(path.display().to_string()).clicked() {
                actions.push(Action::OpenProject(path.clone()));
                ui.close_menu();
            }
        }
        ui.separator();
        if ui.button("Clear Recent").clicked() {
            actions.push(Action::ClearRecent);
            ui.close_menu();
        }
    });
}

/// The View menu: show or hide the navigation panel (with the Ctrl+B hint), and flip which side it
/// docks to. The dock item names its destination, so one item flips the side either way.
fn view_menu(ui: &mut egui::Ui, shell: &Shell, toggle: egui::KeyboardShortcut, actions: &mut Vec<Action>) {
    ui.menu_button("View", |ui| {
        // Let items size to their text instead of wrapping in a narrow menu.
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
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

/// The bottom status bar. Shows the open project's name on the left (or that none is open - the
/// in-window confirmation that Open Project took effect, which the title bar carries too), then the
/// last open failure when there is one (in a warning color, so a folder that was not a wok project
/// reads as a clear error rather than a silent no-op). The camera is mouse-only with no mode, so there
/// is no mode readout; the right side is redefined (the select/place interaction state) when picking
/// and place land. The richer readouts (snap, counts, framerate, save state, integrity) join as their
/// features land.
pub fn status_bar(ctx: &egui::Context, project: &Project, open_error: Option<&str>) {
    egui::TopBottomPanel::bottom("wok_status_bar").exact_height(STATUS_BAR_HEIGHT).show(ctx, |ui| {
        let dim = theme::palette(ui.ctx()).text_dim;
        ui.horizontal_centered(|ui| {
            match project.display_name() {
                Some(name) => ui.label(egui::RichText::new(name).small().color(dim)),
                None => ui.label(egui::RichText::new("No project open").small().color(dim)),
            };
            if let Some(message) = open_error {
                ui.label(egui::RichText::new("-").small().color(dim));
                ui.label(egui::RichText::new(message).small().color(WARN_COLOR)).on_hover_text(message);
            }
        });
    });
}
