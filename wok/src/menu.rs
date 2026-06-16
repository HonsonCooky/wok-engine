//! The editor chrome's bars: the top header strip and the bottom status bar.
//!
//! With the OS title bar turned off (`crate::app`), the header is ours to draw: it carries the menu
//! (File, View), a centered title, and the min/max/close window controls, with the empty middle a
//! drag handle. Window controls and the drag route through the action seam as [`Action`]s the frame
//! loop applies to the OS window - the same shape as Quit - so this layer never touches the window
//! handle. Like all the view, the chrome reads the model and emits actions; the handler
//! (`crate::action`) is the single writer.
//!
//! Opening a project goes through the OS-native folder picker (rfd): the File menu calls it
//! synchronously - a modal pick that blocks the UI thread for its duration, expected of a native
//! dialog - and emits [`Action::OpenProject`], or nothing when cancelled. The View menu drives the
//! navigation panel (show/hide, dock side), the same actions Ctrl+B dispatches.

use egui::Color32;

use crate::action::Action;
use crate::model::{Model, Shell, Side};
use crate::project::Project;
use crate::theme;

/// Header height in points: room for the menu and full-height window controls.
const HEADER_HEIGHT: f32 = 36.0;

/// Status-bar height in points: one row of small text plus breathing room.
const STATUS_BAR_HEIGHT: f32 = 24.0;

/// Width of one window-control cell. Wider than tall, in the OS title-bar idiom.
const WINDOW_BTN_W: f32 = 44.0;

/// Half-size of a window-control glyph, in points - the icons are painted, not glyph fonts.
const ICON_HALF: f32 = 4.0;

/// Hover fill for the close button: a muted red, the one place the chrome warns.
const CLOSE_HOVER: Color32 = Color32::from_rgb(0xc0, 0x3a, 0x3a);

/// The Ctrl+B (Cmd+B on macOS) shortcut that toggles the navigation panel. Built in one place so the
/// menu's hint and the global handler always agree on the binding.
fn nav_toggle_shortcut() -> egui::KeyboardShortcut {
    egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::B)
}

/// The custom header: the menu on the left, the window controls on the right, and the title over the
/// draggable middle. Also consumes the global panel-toggle shortcut, so Ctrl+B works whether or not
/// the View menu is open.
pub fn header(ctx: &egui::Context, model: &Model, actions: &mut Vec<Action>) {
    let toggle = nav_toggle_shortcut();
    if ctx.input_mut(|i| i.consume_shortcut(&toggle)) {
        actions.push(Action::ToggleNav);
    }
    // A tight frame: a little left padding for the menu, none elsewhere so the controls reach the
    // right edge and fill the strip's height.
    let frame = egui::Frame::NONE
        .fill(ctx.style().visuals.panel_fill)
        .inner_margin(egui::Margin { left: 8, right: 0, top: 0, bottom: 0 });
    egui::TopBottomPanel::top("wok_header").frame(frame).exact_height(HEADER_HEIGHT).show(ctx, |ui| {
        let header_rect = ui.max_rect();
        ui.horizontal_centered(|ui| {
            egui::menu::bar(ui, |ui| {
                file_menu(ui, actions);
                view_menu(ui, &model.shell, toggle, actions);
            });
            // The rest of the strip, from the right: the window controls, then the leftover middle as
            // the drag handle.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if window_button(ui, WindowButton::Close).clicked() {
                    actions.push(Action::Quit);
                }
                if window_button(ui, WindowButton::Maximize).clicked() {
                    actions.push(Action::ToggleMaximize);
                }
                if window_button(ui, WindowButton::Minimize).clicked() {
                    actions.push(Action::Minimize);
                }
                drag_handle(ui, actions);
            });
        });
        // The title, painted over the header's centre, independent of the menu and controls so it
        // stays put as either side grows.
        ui.painter().text(
            header_rect.center(),
            egui::Align2::CENTER_CENTER,
            title_text(&model.project),
            egui::FontId::proportional(13.0),
            theme::TEXT_DIM,
        );
    });
}

/// The header's empty middle, as the window drag handle: press-drag to move the window, double-click
/// to toggle maximize - the affordances the OS title bar used to give.
fn drag_handle(ui: &mut egui::Ui, actions: &mut Vec<Action>) {
    let response = ui.allocate_response(ui.available_size(), egui::Sense::click_and_drag());
    if response.drag_started() {
        actions.push(Action::StartDrag);
    }
    if response.double_clicked() {
        actions.push(Action::ToggleMaximize);
    }
}

/// The header title: the app name, plus the open project's name when one is open.
fn title_text(project: &Project) -> String {
    match project.display_name() {
        Some(name) => format!("wok - {name}"),
        None => "wok".to_string(),
    }
}

/// The three window controls, by glyph.
#[derive(Clone, Copy)]
enum WindowButton {
    Minimize,
    Maximize,
    Close,
}

/// One window control: a full-height cell with a painted glyph and a hover fill (red for close,
/// neutral otherwise). Painting the icons keeps them crisp and the source free of glyph characters.
fn window_button(ui: &mut egui::Ui, kind: WindowButton) -> egui::Response {
    let size = egui::vec2(WINDOW_BTN_W, ui.available_height());
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    if response.hovered() {
        let fill = match kind {
            WindowButton::Close => CLOSE_HOVER,
            _ => ui.visuals().widgets.hovered.weak_bg_fill,
        };
        ui.painter().rect_filled(rect, egui::CornerRadius::ZERO, fill);
    }
    let c = rect.center();
    let color = if response.hovered() { theme::TEXT_BRIGHT } else { theme::TEXT_DIM };
    let stroke = egui::Stroke::new(1.0, color);
    let painter = ui.painter();
    match kind {
        WindowButton::Minimize => {
            painter.hline(c.x - ICON_HALF..=c.x + ICON_HALF, c.y, stroke);
        }
        WindowButton::Maximize => {
            let glyph = egui::Rect::from_center_size(c, egui::Vec2::splat(2.0 * ICON_HALF));
            painter.rect_stroke(glyph, egui::CornerRadius::ZERO, stroke, egui::StrokeKind::Inside);
        }
        WindowButton::Close => {
            // The two diagonals of a square centred on the cell make the X.
            let (lo, hi) = (c - egui::Vec2::splat(ICON_HALF), c + egui::Vec2::splat(ICON_HALF));
            painter.line_segment([lo, hi], stroke);
            painter.line_segment([egui::pos2(lo.x, hi.y), egui::pos2(hi.x, lo.y)], stroke);
        }
    }
    response
}

/// The File menu in Zed's shape, trimmed to what exists: New Project (a stub), Open Project, Open
/// Recent (a stub - no recents tracked yet), and Quit.
fn file_menu(ui: &mut egui::Ui, actions: &mut Vec<Action>) {
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
        ui.menu_button("Open Recent", |ui| {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
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

/// The bottom status bar. Shows the open project's name, or that none is open - the in-window
/// confirmation that Open Project took effect (the title bar carries the same).
pub fn status_bar(ctx: &egui::Context, project: &Project) {
    egui::TopBottomPanel::bottom("wok_status_bar").exact_height(STATUS_BAR_HEIGHT).show(ctx, |ui| {
        ui.horizontal_centered(|ui| match project.display_name() {
            Some(name) => ui.label(egui::RichText::new(name).small().color(theme::TEXT_DIM)),
            None => ui.label(egui::RichText::new("No project open").small().color(theme::TEXT_DIM)),
        });
    });
}
