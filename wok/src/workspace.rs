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
//! The icon bar reads the active navigation view from the model and emits `Action::SelectNavView` on a
//! click, switching the view through the action seam (`crate::action::handle`); the header label and
//! the body track the active view too. The body lists the open project's content for the project-scoped
//! views (Scenes, Prefabs, Lighting) through `wok_scene::ContentLayout` discovery, scanned per frame;
//! Instances keeps a placeholder until it has an open scene to read (a later slice). The panel docks to
//! either side and toggles through the View menu (`crate::menu`) - the composition root shows `nav_panel`
//! only when visible, on the model's chosen side, and the menu drives both - and it resizes by dragging
//! its inner edge, with egui owning the live width (so there is no Shell state for it). The tab still
//! does not switch or close - tabs are a later slice. Every colour is read through `theme::palette`, so
//! the chrome follows the OS light/dark.

use wok_scene::ContentLayout;

use crate::action::Action;
use crate::icons;
use crate::menu;
use crate::model::{Model, NavSide, NavView};
use crate::theme;

/// The navigation panel's default width in points (README shell layout: ~240px on the left). The panel
/// is resizable, so this is only the width it opens at; egui owns the live width from there (the drag
/// is kept in egui's own memory, not the Shell - resizing is a view-local affordance, not model state).
const NAV_PANEL_WIDTH: f32 = 240.0;

/// The navigation panel's resize clamp in points. The floor keeps the header label, the placeholder
/// body, and the bottom icon bar legible; the ceiling stops the panel from swallowing the editor area.
/// egui constrains the resize drag to this range.
const NAV_PANEL_MIN_WIDTH: f32 = 180.0;
const NAV_PANEL_MAX_WIDTH: f32 = 420.0;

/// The bottom icon bar's height in points (handoff view 2: a Zed-style icon row at the panel foot).
/// Set equal to the status bar's height (`menu::STATUS_BAR_HEIGHT`) so the two bottom bars - this one
/// under the nav panel and the status bar in the view column - line up into one continuous band along
/// the window foot. Cells fill this height, so it also sets each icon button's vertical padding.
const ICON_BAR_HEIGHT: f32 = 28.0;

/// Width of each icon cell in the bottom bar; its height fills the bar. Sets each icon button's
/// horizontal padding around the ~12px glyph - a little room each side so the row reads as discrete
/// buttons rather than glyphs jammed against their neighbours.
const ICON_CELL: f32 = 32.0;

/// Tab-strip height in points. It must contain the row content (the hamburger and the tab cell, with
/// their margins) with no overflow, because egui clips a top panel's fill to `exact_height` while
/// reserving the larger content-driven height for the panel below - an overflow leaves an unpainted
/// strip exposing the backdrop (sharp-edges 2). 38 fits with room to spare.
const TAB_BAR_HEIGHT: f32 = 38.0;

/// Horizontal padding for the navigation panel's text rows. The panel frame is flush (zero inner
/// margin) so the icon bar and its hairlines reach the panel edges; rows that hold text add this back
/// so the text is not jammed against the edge.
const ROW_PAD: f32 = 10.0;

/// Height of one content-list row in points (handoff view 2: tight file-list rows, ~24-25px). Set as a
/// fixed cell height rather than letting each label size itself, so the rows read as an even list and
/// the full-bleed selection highlight (a later slice) has a row rect to fill.
const NAV_ROW_HEIGHT: f32 = 24.0;

/// The Nerd Font glyph for a navigation view's bottom-bar cell. Which icon-font codepoint a view draws
/// is a chrome concern, so the mapping lives here with the view rather than on `NavView` in the model
/// (whose `title` carries the view's canonical name). The grouping into project (Scenes, Prefabs) and
/// this-scene (Instances, Lighting) is the bar's layout, in `icon_bar`.
fn nav_icon(view: NavView) -> char {
    match view {
        NavView::Scenes => icons::LAYERS,
        NavView::Prefabs => icons::CUBE_OUTLINE,
        NavView::Instances => icons::LIST_BULLETED,
        NavView::Lighting => icons::WEATHER_SUNNY,
    }
}

/// A themed panel frame with no inner margin: the fill still reaches the panel edge (keeping the
/// surface colour, unlike `Frame::NONE` which would drop the fill and expose the backdrop), but
/// content sits flush, so a full-bleed hairline or an accent line can be painted at the exact edge
/// (sharp-edges 2). Rows that need breathing room add their own padding.
fn flush_panel(ctx: &egui::Context) -> egui::Frame {
    egui::Frame::side_top_panel(&ctx.style()).inner_margin(egui::Margin::ZERO)
}

/// The full-height navigation panel: a header naming the active view, a placeholder body, and the
/// bottom icon bar at the foot. Docked to the model's chosen side and shown before the view column (by
/// the composition root) so it claims the full-height strip on that side; the view column fills what
/// remains. The composition root only calls this when the panel is visible. Resizable by dragging its
/// inner edge (egui owns the width, clamped to [`NAV_PANEL_MIN_WIDTH`]..=[`NAV_PANEL_MAX_WIDTH`]);
/// docking and toggling are unaffected.
pub fn nav_panel(ctx: &egui::Context, model: &Model, actions: &mut Vec<Action>) {
    // Same builder either way - only the docked edge differs (`SidePanel::left` vs `::right`).
    let panel = match model.shell.nav_side() {
        NavSide::Left => egui::SidePanel::left("wok_nav_panel"),
        NavSide::Right => egui::SidePanel::right("wok_nav_panel"),
    };
    panel
        .resizable(true)
        .default_width(NAV_PANEL_WIDTH)
        .min_width(NAV_PANEL_MIN_WIDTH)
        .max_width(NAV_PANEL_MAX_WIDTH)
        .frame(flush_panel(ctx))
        .show(ctx, |ui| {
            // The header (top) and icon bar (foot) are nested panels claiming opposite edges; the body
            // then fills what remains between them. The header claims the top first so it sits at the
            // same y as the tab bar (see nav_header). The icon bar is the only region that emits
            // actions this slice; the header and body read the active view to label themselves.
            nav_header(ui, model);
            icon_bar(ui, model, actions);
            nav_body(ui, model);
        });
}

/// The panel header: a single row naming the active view (the handoff's text_bright, weight-600
/// title) with the view's one contextual control on the right - a dim placeholder here, inert until
/// the views are built. Its height is exactly the tab-bar height (driven off `TAB_BAR_HEIGHT`, not a
/// separate value), so the header and the tab bar read as one band across the top with flush bottom
/// edges; a bottom hairline at the header's foot lands on that shared edge.
fn nav_header(ui: &mut egui::Ui, model: &Model) {
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
                ui.label(egui::RichText::new(model.shell.active_nav().title()).color(p.text_bright).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(ROW_PAD);
                    // The contextual-control slot (for Instances, a group-by / sort toggle). A dim
                    // placeholder for now; the real control lands with the view.
                    ui.label(egui::RichText::new("A-Z").color(p.text_dim).small());
                });
            });
        });
}

/// The panel body: wholly the active view. The project-scoped views (Scenes, Prefabs, Lighting) list
/// the open project's content of that kind; Instances keeps a placeholder until it has an open scene to
/// read (a later slice). The body names the active view either way, so switching is visible here as
/// well as in the header and the icon accent.
fn nav_body(ui: &mut egui::Ui, model: &Model) {
    match model.shell.active_nav() {
        NavView::Scenes => content_list(ui, model, "No scenes yet", ContentLayout::scene_names),
        NavView::Prefabs => content_list(ui, model, "No prefabs yet", ContentLayout::prefab_slugs),
        NavView::Lighting => content_list(ui, model, "No lighting states yet", ContentLayout::lighting_names),
        // Instances lists the OPEN SCENE's placements, which this slice has no concept of yet; the
        // original placeholder stands in until the scene-tab slice gives it a scene to read.
        NavView::Instances => nav_placeholder(ui, model),
    }
}

/// List the open project's content for a project-scoped view, one tight row per name. `scan` is the
/// `ContentLayout` discovery method for this view's kind - `scene_names`, `prefab_slugs`, or
/// `lighting_names` - run per frame against the open project's root. The per-frame scan is simple and
/// self-refreshing: a file added on disk shows on the next frame with no cache to invalidate. (An
/// app-side cache or a file-watch is a deferred optimization, for if the scan ever costs too much; it
/// does not for a folder listing.) Display-only this slice: each row is a label, not a control -
/// opening a scene as a tab and selecting it are later slices. Two empty states read dim and italic:
/// no project open at all, or a project open with nothing of this kind yet (`empty`).
fn content_list(ui: &mut egui::Ui, model: &Model, empty: &str, scan: fn(&ContentLayout) -> Vec<String>) {
    ui.add_space(4.0);
    let Some(project) = model.project.as_ref() else {
        empty_note(ui, "No project open");
        return;
    };
    let names = scan(&ContentLayout::new(project.root()));
    if names.is_empty() {
        empty_note(ui, empty);
        return;
    }
    // A scroll area takes over when the list outgrows the body, so a content-heavy project can reach
    // every name rather than losing the ones past the fold (the icon bar claims its strip first, so an
    // un-scrolled overflow would simply be clipped). It draws no scrollbar until the content overflows.
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        // A tight file-list: rows sit close, parted by a hairline of spacing, not the form spacing the
        // panel uses elsewhere.
        ui.spacing_mut().item_spacing.y = 2.0;
        for name in &names {
            content_row(ui, name);
        }
    });
}

/// One content row: the name in primary text, inset by [`ROW_PAD`] from the flush panel edge and
/// centred in a fixed-height cell ([`NAV_ROW_HEIGHT`]). Display-only this slice - no hover, no click,
/// no selection highlight; those land when a row opens a tab. The fixed cell (not the label's own
/// height) keeps the rows even and gives the full-bleed selection highlight to come a rect to fill.
fn content_row(ui: &mut egui::Ui, name: &str) {
    let text = theme::palette(ui.ctx()).text;
    let (rect, _response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), NAV_ROW_HEIGHT), egui::Sense::hover());
    let pos = egui::pos2(rect.left() + ROW_PAD, rect.center().y);
    let font = egui::TextStyle::Body.resolve(ui.style());
    ui.painter().text(pos, egui::Align2::LEFT_CENTER, name, font, text);
}

/// A dim, italic note filling the body in place of a list: the empty states for the project-scoped
/// views (no project open, or none of this kind yet) and the Instances placeholder. Inset by
/// [`ROW_PAD`] like the header and the rows, so the text lines up with them.
fn empty_note(ui: &mut egui::Ui, text: &str) {
    let dim = theme::palette(ui.ctx()).text_dim;
    ui.horizontal(|ui| {
        ui.add_space(ROW_PAD);
        ui.label(egui::RichText::new(text).color(dim).italics());
    });
}

/// The Instances placeholder: the view names itself but has no open scene to list yet. Kept as the
/// original placeholder line (its own slice replaces it with the instances tree), so this slice only
/// changes the project-scoped views, not Instances.
fn nav_placeholder(ui: &mut egui::Ui, model: &Model) {
    let title = model.shell.active_nav().title();
    ui.add_space(8.0);
    empty_note(ui, &format!("{title} - view content lands here"));
}

/// The bottom icon bar at the panel foot (handoff view 2): a Zed-style row of view icons, split by a
/// vertical divider into the project group (Scenes, Prefabs) and the this-scene group (Instances,
/// Lighting). The active view's icon carries a 2px accent top-line and an accent tint; the rest sit
/// dim. A top hairline separates the bar from the body above. A click on a cell emits
/// `Action::SelectNavView`, switching the active view through the action seam.
fn icon_bar(ui: &mut egui::Ui, model: &Model, actions: &mut Vec<Action>) {
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
                icon_cell(ui, model, NavView::Scenes, height, actions);
                icon_cell(ui, model, NavView::Prefabs, height, actions);
                divider(ui, height);
                icon_cell(ui, model, NavView::Instances, height, actions);
                icon_cell(ui, model, NavView::Lighting, height, actions);
            });
        });
}

/// One icon cell: the view's Nerd Font glyph centred in a full-height cell. The active view (per
/// `model.shell.active_nav`) gets an accent tint behind it and a 2px accent top-line at the bar's top
/// edge, and its glyph is the accent colour; the rest sit dim. A click emits
/// `Action::SelectNavView(view)` - the one way the active view changes (the view never mutates the
/// model itself).
fn icon_cell(ui: &mut egui::Ui, model: &Model, view: NavView, height: f32, actions: &mut Vec<Action>) {
    let p = theme::palette(ui.ctx());
    let active = view == model.shell.active_nav();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ICON_CELL, height), egui::Sense::click());
    if response.clicked() {
        actions.push(Action::SelectNavView(view));
    }
    if active {
        let tint = egui::Color32::from_rgba_unmultiplied(p.accent.r(), p.accent.g(), p.accent.b(), 0x33);
        ui.painter().rect_filled(rect, 0.0, tint);
        ui.painter().hline(rect.x_range(), rect.top(), egui::Stroke::new(2.0, p.accent));
    }
    let color = if active { p.accent } else { p.text_dim };
    icons::paint(ui.painter(), rect, nav_icon(view), color);
}

/// The vertical divider between the project group and the this-scene group: a short 1px rule, inset
/// from the bar's top and bottom so it reads as a separator rather than a full-height line.
fn divider(ui: &mut egui::Ui, height: f32) {
    let border = theme::palette(ui.ctx()).border;
    let (rect, _response) = ui.allocate_exact_size(egui::vec2(9.0, height), egui::Sense::hover());
    let inset = height * 0.28;
    ui.painter().vline(rect.center().x, rect.top() + inset..=rect.bottom() - inset, egui::Stroke::new(1.0, border));
}

/// The tab bar over the view column: the app-menu hamburger at the left (which opens the File / View /
/// Run / Help menu), then one placeholder tab. Hand-drawn (egui has no tab widget). The tab does not
/// switch or close; opening, closing, and switching tabs is a later slice. The single tab is rendered
/// active to exercise the active-tab styling: the editor-surface fill (so it reads continuous with the
/// well below) and the one accent as a top line.
pub fn tab_bar(ctx: &egui::Context, model: &Model, actions: &mut Vec<Action>) {
    egui::TopBottomPanel::top("wok_tab_bar").exact_height(TAB_BAR_HEIGHT).show(ctx, |ui| {
        ui.horizontal_centered(|ui| {
            menu::hamburger(ui, model, actions);
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
