//! The editor workspace: the full-height navigation panel (with its bottom icon bar), the tab bar,
//! and the per-context editor area.
//!
//! These regions are laid out so egui nests them correctly (see the composition root, `crate::view`):
//! the navigation panel is shown first and claims the full-height left strip, then the view column -
//! the tab bar, the editor well, and the status bar - fills the remaining width. The editor area
//! paints no fill, so the editor-background backdrop (the GPU clear live, the snapshot harness's fill
//! in the test) shows through it as the empty well; the per-context surface lands there in later
//! slices.
//!
//! Static framing only: the views, the tab, and the icon bar take no input and emit no actions. The
//! region behaviors (switching the nav view, opening and closing tabs, docking and toggling the
//! panel) and the model + action seam they would drive are the next slices. Every colour is read
//! through `theme::palette`, so the chrome follows the OS light/dark.

use crate::menu;
use crate::icons;
use crate::theme;

/// Navigation-panel width in points (README shell layout: ~240px on the left).
const NAV_PANEL_WIDTH: f32 = 240.0;

/// The bottom icon bar's height in points (handoff view 2: a Zed-style icon row at the panel foot,
/// ~38-40px).
const ICON_BAR_HEIGHT: f32 = 40.0;

/// Width of each icon cell in the bottom bar; its height fills the bar.
const ICON_CELL: f32 = 34.0;

/// Tab-strip height in points. It must contain the row content (the hamburger and the tab cell, with
/// their margins) with no overflow, because egui clips a top panel's fill to `exact_height` while
/// reserving the larger content-driven height for the panel below - an overflow leaves an unpainted
/// strip exposing the backdrop (sharp-edges 2). 38 fits with room to spare.
const TAB_BAR_HEIGHT: f32 = 38.0;

/// Horizontal padding for the navigation panel's text rows. The panel frame is flush (zero inner
/// margin) so the icon bar and its hairlines reach the panel edges; rows that hold text add this back
/// so the text is not jammed against the edge.
const ROW_PAD: f32 = 10.0;

/// The navigation views, one per icon in the bottom bar, split into the two scope groups the divider
/// separates: the project group (Scenes, Prefabs) is the same whichever scene is open; the this-scene
/// group (Instances, Lighting) is bound to the open scene. Static for this slice - `ACTIVE` is fixed
/// and the header names it; clicking an icon does not switch the view yet (the next slice adds that
/// and the `Action::SelectNavView` seam).
#[derive(Clone, Copy, PartialEq, Eq)]
enum NavView {
    Scenes,
    Prefabs,
    Instances,
    Lighting,
}

impl NavView {
    /// The header title for this view.
    fn title(self) -> &'static str {
        match self {
            NavView::Scenes => "Scenes",
            NavView::Prefabs => "Prefabs",
            NavView::Instances => "Instances",
            NavView::Lighting => "Lighting",
        }
    }

    /// The Nerd Font icon for this view's bottom-bar cell.
    fn icon(self) -> char {
        match self {
            NavView::Scenes => icons::LAYERS,
            NavView::Prefabs => icons::CUBE_OUTLINE,
            NavView::Instances => icons::LIST_BULLETED,
            NavView::Lighting => icons::WEATHER_SUNNY,
        }
    }
}

/// The view shown active in this static slice: Instances, the open scene's placements, the design's
/// default landing view.
const ACTIVE: NavView = NavView::Instances;

/// A themed panel frame with no inner margin: the fill still reaches the panel edge (keeping the
/// surface colour, unlike `Frame::NONE` which would drop the fill and expose the backdrop), but
/// content sits flush, so a full-bleed hairline or an accent line can be painted at the exact edge
/// (sharp-edges 2). Rows that need breathing room add their own padding.
fn flush_panel(ctx: &egui::Context) -> egui::Frame {
    egui::Frame::side_top_panel(&ctx.style()).inner_margin(egui::Margin::ZERO)
}

/// The full-height navigation panel on the left: a header naming the active view, a placeholder body,
/// and the bottom icon bar at the foot. Shown before the central panel (by the composition root) so
/// it claims the full-height left strip; the view column fills what remains.
pub fn nav_panel(ctx: &egui::Context) {
    egui::SidePanel::left("wok_nav_panel")
        .resizable(false)
        .exact_width(NAV_PANEL_WIDTH)
        .frame(flush_panel(ctx))
        .show(ctx, |ui| {
            // The header (top) and icon bar (foot) are nested panels claiming opposite edges; the body
            // then fills what remains between them. The header claims the top first so it sits at the
            // same y as the tab bar (see nav_header).
            nav_header(ui);
            icon_bar(ui);
            nav_body(ui);
        });
}

/// The panel header: a single row naming the active view (the handoff's text_bright, weight-600
/// title) with the view's one contextual control on the right - a dim placeholder here, inert until
/// the views are built. Its height is exactly the tab-bar height (driven off `TAB_BAR_HEIGHT`, not a
/// separate value), so the header and the tab bar read as one band across the top with flush bottom
/// edges; a bottom hairline at the header's foot lands on that shared edge.
fn nav_header(ui: &mut egui::Ui) {
    egui::TopBottomPanel::top("wok_nav_header")
        .exact_height(TAB_BAR_HEIGHT)
        .frame(flush_panel(ui.ctx()))
        .show_inside(ui, |ui| {
            let p = theme::palette(ui.ctx());
            // Bottom hairline at the header's foot, on the same y as the tab bar's bottom edge.
            let bottom = ui.max_rect().bottom();
            ui.painter().hline(ui.max_rect().x_range(), bottom, egui::Stroke::new(1.0, p.border));
            ui.horizontal_centered(|ui| {
                ui.add_space(ROW_PAD);
                ui.label(egui::RichText::new(ACTIVE.title()).color(p.text_bright).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(ROW_PAD);
                    // The contextual-control slot (for Instances, a group-by / sort toggle). A dim
                    // placeholder for now; the real control lands with the view.
                    ui.label(egui::RichText::new("A-Z").color(p.text_dim).small());
                });
            });
        });
}

/// The panel body: wholly the active view in a built editor (the Instances tree, etc.); a dim
/// placeholder here, since those views are later slices.
fn nav_body(ui: &mut egui::Ui) {
    let dim = theme::palette(ui.ctx()).text_dim;
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add_space(ROW_PAD);
        ui.label(egui::RichText::new("(view content lands here)").color(dim).italics());
    });
}

/// The bottom icon bar at the panel foot (handoff view 2): a Zed-style row of view icons, split by a
/// vertical divider into the project group (Scenes, Prefabs) and the this-scene group (Instances,
/// Lighting). The active view's icon carries a 2px accent top-line and an accent tint; the rest sit
/// dim. A top hairline separates the bar from the body above. Static - clicking does not switch views.
fn icon_bar(ui: &mut egui::Ui) {
    let border = theme::palette(ui.ctx()).border;
    egui::TopBottomPanel::bottom("wok_nav_icon_bar")
        .exact_height(ICON_BAR_HEIGHT)
        .frame(flush_panel(ui.ctx()))
        .show_inside(ui, |ui| {
            // Top hairline at the bar's top edge (full width, since the frame is flush).
            let top = ui.max_rect().top();
            ui.painter().hline(ui.max_rect().x_range(), top, egui::Stroke::new(1.0, border));
            let height = ui.available_height();
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                ui.add_space(8.0);
                icon_cell(ui, NavView::Scenes, height);
                icon_cell(ui, NavView::Prefabs, height);
                divider(ui, height);
                icon_cell(ui, NavView::Instances, height);
                icon_cell(ui, NavView::Lighting, height);
            });
        });
}

/// One icon cell: the view's Nerd Font glyph centred in a full-height cell. The active view's cell
/// gets an accent tint behind it and a 2px accent top-line at the bar's top edge, and its glyph is the
/// accent colour; the rest sit dim.
fn icon_cell(ui: &mut egui::Ui, view: NavView, height: f32) {
    let p = theme::palette(ui.ctx());
    let active = view == ACTIVE;
    let (rect, _response) = ui.allocate_exact_size(egui::vec2(ICON_CELL, height), egui::Sense::hover());
    if active {
        let tint = egui::Color32::from_rgba_unmultiplied(p.accent.r(), p.accent.g(), p.accent.b(), 0x33);
        ui.painter().rect_filled(rect, 0.0, tint);
        ui.painter().hline(rect.x_range(), rect.top(), egui::Stroke::new(2.0, p.accent));
    }
    let color = if active { p.accent } else { p.text_dim };
    icons::paint(ui.painter(), rect, view.icon(), color);
}

/// The vertical divider between the project group and the this-scene group: a short 1px rule, inset
/// from the bar's top and bottom so it reads as a separator rather than a full-height line.
fn divider(ui: &mut egui::Ui, height: f32) {
    let border = theme::palette(ui.ctx()).border;
    let (rect, _response) = ui.allocate_exact_size(egui::vec2(9.0, height), egui::Sense::hover());
    let inset = height * 0.28;
    ui.painter().vline(rect.center().x, rect.top() + inset..=rect.bottom() - inset, egui::Stroke::new(1.0, border));
}

/// The tab bar over the view column: the app-menu hamburger at the left, then one placeholder tab.
/// Hand-drawn (egui has no tab widget). Static - the tab does not switch or close; opening, closing,
/// and switching tabs is the next slice. The single tab is rendered active to exercise the active-tab
/// styling: the editor-surface fill (so it reads continuous with the well below) and the one accent
/// as a top line.
pub fn tab_bar(ctx: &egui::Context) {
    egui::TopBottomPanel::top("wok_tab_bar").exact_height(TAB_BAR_HEIGHT).show(ctx, |ui| {
        ui.horizontal_centered(|ui| {
            menu::hamburger(ui);
            ui.add_space(8.0);
            // Tabs nearly touch, as in Zed, with the active fill the only thing parting them.
            ui.spacing_mut().item_spacing.x = 1.0;
            tab_cell(ui, "sample", true);
        });
    });
}

/// One tab cell: the title over the active fill, with an inert close glyph. The active tab borrows the
/// editor surface (so it reads continuous with the well below) and carries the accent as a top line;
/// an inactive tab sits flat and dim (the styling is here for when tab switching lands).
fn tab_cell(ui: &mut egui::Ui, title: &str, active: bool) {
    let p = theme::palette(ui.ctx());
    let fill = if active { p.editor_bg } else { egui::Color32::TRANSPARENT };
    let inner = egui::Frame::NONE.fill(fill).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
        ui.horizontal(|ui| {
            // The strip tightens item spacing for the tabs; restore a gap inside the cell so the close
            // glyph does not crowd the title.
            ui.spacing_mut().item_spacing.x = 6.0;
            let color = if active { p.text_bright } else { p.text_dim };
            let title = egui::RichText::new(title).color(color);
            let title = if active { title.strong() } else { title };
            ui.label(title);
            // The close affordance, the same Nerd Font family as the rest of the chrome, sized small
            // so it sits quietly beside the title rather than competing with it.
            ui.label(egui::RichText::new(icons::CLOSE).size(10.0).color(p.text_dim));
        });
    });
    if active {
        let rect = inner.response.rect;
        ui.painter().hline(rect.x_range(), rect.top(), egui::Stroke::new(2.0, p.accent));
    }
}

/// The editor area: an empty themed well for this slice. A transparent panel over the editor-
/// background backdrop (the GPU clear in the live app, the snapshot harness's background fill in the
/// test), so the well reads as `editor_bg`. The per-context surface - the 3D viewport, the data views
/// - lands here in later slices, drawn into this same transparent panel.
pub fn editor_area(ctx: &egui::Context) {
    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |_ui| {});
}
