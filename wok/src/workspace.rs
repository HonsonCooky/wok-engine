//! The editor workspace: the navigation panel, the tab bar, and the per-context editor area.
//!
//! These three regions fill the space above the status bar (`crate::menu`). They are laid out in
//! order so egui nests them correctly: the navigation panel claims one side (when shown), then the
//! tab bar caps the remaining width - with the app-menu hamburger at its left - and the editor area
//! takes what is left. The editor area paints no background, so the GPU viewport clear shows through
//! as the 3D surface that lands later. Like the chrome, this layer only reads the model and emits
//! actions; it never mutates state.

use crate::action::Action;
use crate::menu;
use crate::model::{Shell, Side, Tab};
use crate::theme;

/// Tab-strip height in points: enough that the active tab's fill reads as a panel, not a chip.
const TAB_BAR_HEIGHT: f32 = 34.0;

/// Default navigation-panel width in points.
const NAV_PANEL_WIDTH: f32 = 216.0;

/// Draw the workspace for one frame. Order matters: the side panel first, then the tab bar over the
/// remaining width, then the editor area; when the panel is hidden the tab bar and editor area take
/// the full width.
pub fn ui(ctx: &egui::Context, shell: &Shell, actions: &mut Vec<Action>) {
    if shell.nav_visible() {
        nav_panel(ctx, shell);
    }
    tab_bar(ctx, shell, actions);
    editor_area(ctx, shell);
}

/// The navigation panel: docked left or right, with placeholder content. Real navigation (scene
/// tree, prefab library, ...) binds to the active tab in a later piece; one panel id across both
/// sides keeps its resized width when the dock flips.
fn nav_panel(ctx: &egui::Context, shell: &Shell) {
    let contents = |ui: &mut egui::Ui| {
        ui.add_space(6.0);
        // A small, dim section header in Zed's style, not a loud heading.
        ui.label(egui::RichText::new("NAVIGATION").color(theme::TEXT_DIM).small().strong());
        ui.add_space(2.0);
        ui.separator();
        // Stub list, hinting at the views that bind here later.
        for item in ["(scene tree)", "(prefab library)"] {
            ui.label(egui::RichText::new(item).color(theme::TEXT_DIM));
        }
    };
    match shell.nav_side() {
        Side::Left => {
            egui::SidePanel::left("wok_nav_panel").default_width(NAV_PANEL_WIDTH).show(ctx, contents);
        }
        Side::Right => {
            egui::SidePanel::right("wok_nav_panel").default_width(NAV_PANEL_WIDTH).show(ctx, contents);
        }
    }
}

/// The tab bar: the app-menu hamburger at the left, then one cell per open tab, plus a "+" that opens
/// a new untitled tab. Hand-drawn (egui has no tab widget) so the active-tab highlight and the close
/// affordance are ours to shape.
fn tab_bar(ctx: &egui::Context, shell: &Shell, actions: &mut Vec<Action>) {
    egui::TopBottomPanel::top("wok_tab_bar").exact_height(TAB_BAR_HEIGHT).show(ctx, |ui| {
        ui.horizontal_centered(|ui| {
            // The app-menu sits at the left of the row, always visible regardless of nav-panel state.
            menu::hamburger(ui, shell, actions);
            ui.add_space(4.0);
            // Tabs nearly touch, as in Zed, with the active fill the only thing parting them.
            ui.spacing_mut().item_spacing.x = 1.0;
            for tab in shell.tabs() {
                tab_cell(ui, tab, shell.active() == Some(tab.id), actions);
            }
            if ui.add(egui::Button::new("+").frame(false)).on_hover_text("New tab").clicked() {
                actions.push(Action::OpenTab);
            }
        });
    });
}

/// One tab cell: a title that switches to the tab on click and an x that closes it. The active tab
/// borrows the editor surface so it reads as continuous with the view below, and carries the one
/// accent as a top line; inactive tabs sit flat and dim on the strip.
fn tab_cell(ui: &mut egui::Ui, tab: &Tab, active: bool, actions: &mut Vec<Action>) {
    let fill = if active { theme::EDITOR_BG } else { egui::Color32::TRANSPARENT };
    let inner = egui::Frame::NONE.fill(fill).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
        ui.horizontal(|ui| {
            let color = if active { theme::TEXT_BRIGHT } else { theme::TEXT_DIM };
            let title = egui::RichText::new(&tab.title).color(color);
            let title = if active { title.strong() } else { title };
            if ui.add(egui::Label::new(title).selectable(false).sense(egui::Sense::click())).clicked() {
                actions.push(Action::SelectTab(tab.id));
            }
            let x = egui::RichText::new("x").color(theme::TEXT_DIM);
            if ui.add(egui::Button::new(x).small().frame(false)).on_hover_text("Close tab").clicked() {
                actions.push(Action::CloseTab(tab.id));
            }
        });
    });
    if active {
        let rect = inner.response.rect;
        ui.painter().hline(rect.x_range(), rect.top(), egui::Stroke::new(2.0, theme::ACCENT));
    }
}

/// The editor area: the active tab's content over the viewport clear, or an empty state when no tab
/// is open. A transparent frame lets the GPU clear show through where the 3D viewport will live.
fn editor_area(ctx: &egui::Context, shell: &Shell) {
    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
        ui.centered_and_justified(|ui| match shell.active_tab() {
            // Placeholder content: the tab's title, centered over the cleared viewport.
            Some(tab) => ui.label(egui::RichText::new(&tab.title).heading().color(theme::TEXT_DIM)),
            None => ui.label(egui::RichText::new("No tab open - use + to open one").color(theme::TEXT_DIM)),
        });
    });
}
