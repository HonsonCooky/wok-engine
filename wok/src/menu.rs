//! The editor chrome's app-menu hamburger and status bar.
//!
//! The OS owns the title bar (`crate::main`), so the editor's menu is a single hamburger button at
//! the left of the tab-bar row - always visible, since the tab bar always is, unlike the toggleable
//! navigation panel (Zed's grammar, not a horizontal menu bar). The button opens an app-menu with
//! File, View, Run, and Help; this slice wires View (it drives the navigation panel - show/hide and
//! dock side) and renders File / Run / Help present-but-disabled until their own slices. Like all the
//! view, the menu reads the model and emits actions; `crate::action::handle` is the single writer.
//! Every colour is read through `theme::palette`, so the chrome follows the OS light/dark.

use crate::action::Action;
use crate::model::{Model, NavSide, Shell};
use crate::theme;

/// Status-bar height in points (README shell layout): one row of small text plus breathing room.
const STATUS_BAR_HEIGHT: f32 = 26.0;

/// Size of the hamburger button cell, in points.
const HAMBURGER_CELL: egui::Vec2 = egui::vec2(30.0, 22.0);

/// The app-menu hamburger, drawn by the caller into the tab-bar row. Opens a menu (File / View / Run
/// / Help) on click; only View is wired this slice, the rest render present-but-disabled. The
/// `nf-md-menu` glyph is painted over the button's rect - dim at rest, bright on hover - at the chrome
/// icon size like the nav icons. The glyph carries an accessible "Menu" label so the snapshot test
/// (and a11y tooling) can find and open it.
pub fn hamburger(ui: &mut egui::Ui, model: &Model, actions: &mut Vec<Action>) {
    // A frameless button: no background box at rest, so the look stays the bare glyph, but it still
    // senses hover and click and opens the menu.
    let button = egui::Button::new("").min_size(HAMBURGER_CELL).frame(false);
    let response = egui::menu::menu_custom_button(ui, button, |ui| {
        // Let items size to their text instead of wrapping in a narrow menu.
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        disabled_item(ui, "File");
        view_menu(ui, &model.shell, actions);
        disabled_item(ui, "Run");
        disabled_item(ui, "Help");
    })
    .response
    .on_hover_cursor(egui::CursorIcon::PointingHand);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, "Menu"));
    let p = theme::palette(ui.ctx());
    let color = if response.hovered() { p.text_bright } else { p.text_dim };
    crate::icons::paint(ui.painter(), response.rect, crate::icons::MENU, color);
}

/// The View menu: show or hide the navigation panel (the label tracks the current state), then dock it
/// left or right (the current side marked with a filled radio). Each item emits into the same
/// per-frame action buffer the icon bar uses; `action::handle` applies them. Closing the menu after a
/// pick keeps a click from re-firing while the popup lingers.
fn view_menu(ui: &mut egui::Ui, shell: &Shell, actions: &mut Vec<Action>) {
    ui.menu_button("View", |ui| {
        // Let items size to their text instead of wrapping in a narrow menu.
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        let toggle_label = if shell.nav_visible() { "Hide Navigation Panel" } else { "Show Navigation Panel" };
        if ui.button(toggle_label).clicked() {
            actions.push(Action::ToggleNavPanel);
            ui.close_menu();
        }
        ui.separator();
        // Left and Right are mutually exclusive, so a radio marks the current side; clicking the other
        // side re-docks, clicking the current side is a harmless no-op.
        if ui.radio(shell.nav_side() == NavSide::Left, "Dock Left").clicked() {
            actions.push(Action::SetNavSide(NavSide::Left));
            ui.close_menu();
        }
        if ui.radio(shell.nav_side() == NavSide::Right, "Dock Right").clicked() {
            actions.push(Action::SetNavSide(NavSide::Right));
            ui.close_menu();
        }
    });
}

/// A present-but-disabled menu entry: a greyed button that does nothing, standing in for a menu (File,
/// Help) or verb (Run) whose own slice wires it. It reads as "here, not yet" rather than absent.
fn disabled_item(ui: &mut egui::Ui, label: &str) {
    ui.add_enabled(false, egui::Button::new(label));
}

/// The bottom status bar, within the view column only (the composition root shows the navigation
/// panel first, so this bottom panel spans only the width right of it, never under the nav). Reads
/// contextual diagnostics in a built editor - mode, snap, counts, framerate, save state, integrity;
/// here it holds dim placeholder text at each end (left context, right diagnostics) to exercise the
/// layout. The richer readouts join as their features land.
pub fn status_bar(ctx: &egui::Context) {
    egui::TopBottomPanel::bottom("wok_status_bar").exact_height(STATUS_BAR_HEIGHT).show(ctx, |ui| {
        let dim = theme::palette(ui.ctx()).text_dim;
        ui.horizontal_centered(|ui| {
            ui.label(egui::RichText::new("No project open").small().color(dim));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new("snap 1 m / 5 deg").small().color(dim));
            });
        });
    });
}
