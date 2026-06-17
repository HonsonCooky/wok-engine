//! The editor workspace: the navigation panel, the tab bar, and the per-context editor area.
//!
//! These three regions fill the space above the status bar (`crate::menu`). They are laid out in
//! order so egui nests them correctly: the navigation panel claims one side (when shown), then the
//! tab bar caps the remaining width - with the app-menu hamburger at its left - and the editor area
//! takes what is left. The editor area paints no background, so the GPU scene render shows through
//! it as the viewport, with this chrome framing it. Like the chrome, this layer only reads the model
//! (and the content summary) and emits actions; it never mutates state.
//!
//! The navigation panel hosts the content browser: the open project's scene (clickable, opening the
//! Scene tab), its prefabs, and its lighting states. Prefab and lighting entries are listed but inert
//! until those views exist. When context navigation (the scene tree) is built, the panel flips
//! between the browser and that; today the browser is all there is.

use crate::action::Action;
use crate::menu;
use crate::model::{Model, Shell, Side, Tab, TabKind};
use crate::scene::ContentView;
use crate::theme;

/// Tab-strip height in points: enough that the active tab's fill reads as a panel, not a chip.
const TAB_BAR_HEIGHT: f32 = 34.0;

/// Default navigation-panel width in points.
const NAV_PANEL_WIDTH: f32 = 216.0;

/// Mark a clickable response with a pointing-hand cursor - the affordance that it acts on click.
fn clickable(response: egui::Response) -> egui::Response {
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Draw the workspace for one frame. Order matters: the side panel first, then the tab bar over the
/// remaining width, then the editor area; when the panel is hidden the tab bar and editor area take
/// the full width. `content` is the open project's content summary (none when no project is open).
pub fn ui(ctx: &egui::Context, model: &Model, content: Option<ContentView>, actions: &mut Vec<Action>) {
    if model.shell.nav_visible() {
        nav_panel(ctx, &model.shell, content, actions);
    }
    tab_bar(ctx, model, actions);
    editor_area(ctx, &model.shell, content.is_some());
}

/// The navigation panel: docked left or right, hosting the content browser. One panel id across both
/// sides keeps its resized width when the dock flips.
fn nav_panel(ctx: &egui::Context, shell: &Shell, content: Option<ContentView>, actions: &mut Vec<Action>) {
    let body = |ui: &mut egui::Ui| {
        // A resizable SidePanel only keeps a dragged width if its content fills that width: egui
        // stores the content rect, so a body narrower than the drag snaps back to the content on
        // release (egui's SidePanel docs: a resizable panel needs a width-filling widget). The
        // content browser fills via its separators; force the fill here so the empty no-project
        // state resizes too, instead of pinning to the width of its one short label.
        ui.set_min_width(ui.available_width());
        match content {
            Some(content) => content_browser(ui, content, actions),
            None => {
                let p = theme::palette(ui.ctx());
                ui.add_space(6.0);
                ui.label(egui::RichText::new("No project open").color(p.text_dim));
            }
        }
    };
    match shell.nav_side() {
        Side::Left => {
            egui::SidePanel::left("wok_nav_panel").default_width(NAV_PANEL_WIDTH).show(ctx, body);
        }
        Side::Right => {
            egui::SidePanel::right("wok_nav_panel").default_width(NAV_PANEL_WIDTH).show(ctx, body);
        }
    }
}

/// The content browser: the scene (clickable, opening the Scene tab), then the prefab and lighting
/// listings (inert until those views exist). Each group is a dim Zed-style header over its entries.
fn content_browser(ui: &mut egui::Ui, content: ContentView, actions: &mut Vec<Action>) {
    let p = theme::palette(ui.ctx());

    section_header(ui, "SCENE");
    let entry = egui::RichText::new(content.scene_name).color(p.text_bright);
    let response = clickable(ui.add(egui::Label::new(entry).selectable(false).sense(egui::Sense::click())));
    if response.on_hover_text("Open the scene").clicked() {
        actions.push(Action::OpenScene);
    }

    section_header(ui, "PREFABS");
    listing(ui, content.prefabs, p.text_dim);

    section_header(ui, "LIGHTING");
    listing(ui, content.lights, p.text_dim);
}

/// A small, dim section header in Zed's style (not a loud heading), with a little room above so the
/// groups read apart.
fn section_header(ui: &mut egui::Ui, label: &str) {
    let dim = theme::palette(ui.ctx()).text_dim;
    ui.add_space(8.0);
    ui.label(egui::RichText::new(label).color(dim).small().strong());
    ui.add_space(2.0);
    ui.separator();
}

/// An inert listing of names under a section, or a dim placeholder when the section is empty. These
/// entries become clickable when their views (prefab, lighting) are built.
fn listing(ui: &mut egui::Ui, names: &[String], color: egui::Color32) {
    if names.is_empty() {
        ui.label(egui::RichText::new("(none)").color(color).italics());
        return;
    }
    for name in names {
        ui.label(egui::RichText::new(name).color(color));
    }
}

/// The tab bar: the app-menu hamburger at the left, then one cell per open tab. Hand-drawn (egui has
/// no tab widget) so the active-tab highlight and the close affordance are ours to shape. Tabs open
/// from content (the content browser), so there is no generic new-tab button.
fn tab_bar(ctx: &egui::Context, model: &Model, actions: &mut Vec<Action>) {
    egui::TopBottomPanel::top("wok_tab_bar").exact_height(TAB_BAR_HEIGHT).show(ctx, |ui| {
        ui.horizontal_centered(|ui| {
            // The app-menu sits at the left of the row, always visible regardless of nav-panel state.
            // It reads the whole model (recents, project) for the File menu.
            menu::hamburger(ui, model, actions);
            ui.add_space(8.0);
            // Tabs nearly touch, as in Zed, with the active fill the only thing parting them.
            ui.spacing_mut().item_spacing.x = 1.0;
            for tab in model.shell.tabs() {
                tab_cell(ui, tab, model.shell.active() == Some(tab.id), actions);
            }
        });
    });
}

/// One tab cell: a title that switches to the tab on click and an x that closes it. The active tab
/// borrows the editor surface so it reads as continuous with the view below, and carries the one
/// accent as a top line; inactive tabs sit flat and dim on the strip.
fn tab_cell(ui: &mut egui::Ui, tab: &Tab, active: bool, actions: &mut Vec<Action>) {
    let p = theme::palette(ui.ctx());
    let fill = if active { p.editor_bg } else { egui::Color32::TRANSPARENT };
    let inner = egui::Frame::NONE.fill(fill).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
        ui.horizontal(|ui| {
            // The strip tightens item spacing to 1px for the tabs; restore a gap inside the cell so
            // the close button does not crowd the title.
            ui.spacing_mut().item_spacing.x = 6.0;
            let color = if active { p.text_bright } else { p.text_dim };
            let title = egui::RichText::new(&tab.title).color(color);
            let title = if active { title.strong() } else { title };
            if clickable(ui.add(egui::Label::new(title).selectable(false).sense(egui::Sense::click()))).clicked() {
                actions.push(Action::SelectTab(tab.id));
            }
            let x = egui::RichText::new("x").color(p.text_dim);
            if clickable(ui.add(egui::Button::new(x).small()).on_hover_text("Close tab")).clicked() {
                actions.push(Action::CloseTab(tab.id));
            }
        });
    });
    if active {
        let rect = inner.response.rect;
        ui.painter().hline(rect.x_range(), rect.top(), egui::Stroke::new(2.0, p.accent));
    }
}

/// The editor area: the Scene tab's viewport, or an empty state. A transparent frame lets the GPU
/// scene render show through where a Scene tab is active and a scene is loaded; otherwise it paints a
/// dim prompt, since there is nothing to draw.
fn editor_area(ctx: &egui::Context, shell: &Shell, scene_loaded: bool) {
    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
        let active_is_scene = matches!(shell.active_tab().map(|t| t.kind), Some(TabKind::Scene));
        if active_is_scene && scene_loaded {
            // The viewport: the 3D render shows through this transparent panel, so paint nothing.
            return;
        }
        let dim = theme::palette(ui.ctx()).text_dim;
        let message = if shell.active_tab().is_some() {
            "Loading scene..."
        } else if scene_loaded {
            "Open the scene from the content browser"
        } else {
            "No project open - use File to open one"
        };
        ui.centered_and_justified(|ui| ui.label(egui::RichText::new(message).color(dim)));
    });
}
