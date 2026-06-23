//! The navigation panel: the full-height left (or right) strip with a header naming the active view,
//! the body listing that view's content, and the bottom icon bar that switches views.
//!
//! The regions nest so egui lays them out correctly (see the composition root, `crate::view`): the
//! panel claims the full-height strip on its docked side, the header claims its top (at the same y as
//! the tab bar), the icon bar claims its foot, and the body fills between them. The body reads the
//! active view and shows it: the project-scoped views (Scenes, Prefabs, Lighting) list the open
//! project's content of that kind through `wok_scene::ContentLayout` discovery, scanned per frame - a
//! Scenes row opens that scene as a tab (`Action::OpenScene`) while Prefabs and Lighting rows stay
//! display-only - and the this-scene Instances view defers to [`super::instances`] for the placement
//! tree. The icon bar emits `Action::SelectNavView` on a click, switching the active view through the
//! action seam (`crate::action::handle`); the header label and the icon accent track it too. The panel
//! docks to either side (the composition root shows it on the model's chosen side) and resizes by
//! dragging its inner edge, with egui owning the live width (so there is no Shell state for it).

use wok_scene::ContentLayout;

use crate::action::Action;
use crate::icons;
use crate::loaded::LoadedScene;
use crate::model::{InstanceSort, Model, NavSide, NavView};
use crate::theme;

use super::instances;
use super::{
    ICON_BAR_HEIGHT, ICON_CELL, NAV_PANEL_MAX_WIDTH, NAV_PANEL_MIN_WIDTH, NAV_PANEL_WIDTH, NAV_ROW_HEIGHT, ROW_PAD,
    TAB_BAR_HEIGHT, TREE_GAP, empty_note, glyph_cell,
};

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
pub fn nav_panel(ctx: &egui::Context, model: &Model, loaded_scene: Option<&LoadedScene>, actions: &mut Vec<Action>) {
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
            // same y as the tab bar (see nav_header). The header's Instances sort toggle and the icon
            // bar both emit actions; the body reads the active view (and, for Instances, the sort mode).
            nav_header(ui, model, actions);
            icon_bar(ui, model, actions);
            nav_body(ui, model, loaded_scene, actions);
        });
}

/// The panel header: a single row naming the active view (the handoff's text_bright, weight-600
/// title) with the view's one contextual control on the right. Only the Instances view has one this
/// bite - the group-by-prefab / flat A-Z sort toggle ([`sort_toggle`]); the other views' header right
/// side is empty. Its height is exactly the tab-bar height (driven off `TAB_BAR_HEIGHT`, not a separate
/// value), so the header and the tab bar read as one band across the top with flush bottom edges; a
/// bottom hairline at the header's foot lands on that shared edge.
fn nav_header(ui: &mut egui::Ui, model: &Model, actions: &mut Vec<Action>) {
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
                    // The view's one contextual control. Only Instances has one this bite; the rest
                    // leave the right side empty.
                    if model.shell.active_nav() == NavView::Instances {
                        sort_toggle(ui, model.shell.instance_sort(), actions);
                    }
                });
            });
        });
}

/// The Instances view's contextual control: a compact two-segment toggle - "Group" and "A-Z" - marking
/// the active [`InstanceSort`] in the accent and emitting [`Action::SetInstanceSort`] for the other on a
/// click. The header lays its right side out right-to-left, so the segments are added right ("A-Z")
/// before left ("Group") to read "Group | A-Z" across.
fn sort_toggle(ui: &mut egui::Ui, sort: InstanceSort, actions: &mut Vec<Action>) {
    let dim = theme::palette(ui.ctx()).text_dim;
    if sort_seg(ui, "A-Z", sort == InstanceSort::Flat) {
        actions.push(Action::SetInstanceSort(InstanceSort::Flat));
    }
    ui.label(egui::RichText::new("|").color(dim).small());
    if sort_seg(ui, "Group", sort == InstanceSort::Group) {
        actions.push(Action::SetInstanceSort(InstanceSort::Group));
    }
}

/// One segment of the sort toggle: a small clickable label, the accent colour when it is the active
/// mode and dim otherwise, showing the hand cursor. Returns whether it was clicked this frame. Not
/// selectable, so it senses the click rather than offering text selection.
fn sort_seg(ui: &mut egui::Ui, text: &str, active: bool) -> bool {
    let p = theme::palette(ui.ctx());
    let color = if active { p.accent } else { p.text_dim };
    let label =
        egui::Label::new(egui::RichText::new(text).small().color(color)).selectable(false).sense(egui::Sense::click());
    ui.add(label).on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// The panel body: wholly the active view. The project-scoped views (Scenes, Prefabs, Lighting) list
/// the open project's content of that kind; Instances lists the active scene tab's placements (from
/// `loaded_scene`), grouped or flat per the header's sort toggle. A Scenes row opens that scene as a
/// tab; Prefabs and Lighting rows are display-only (opening those contexts is a later bite), and the
/// Instances rows are display-only too (selection and the floating inspector are a later bite). The
/// body names the active view either way, so switching is visible here as well as in the header and the
/// icon accent.
fn nav_body(ui: &mut egui::Ui, model: &Model, loaded_scene: Option<&LoadedScene>, actions: &mut Vec<Action>) {
    match model.shell.active_nav() {
        NavView::Scenes => {
            // The one clickable list this bite: a click opens that scene as a tab. Its rows lead with
            // the layers mark, the same glyph the Scenes nav-bar icon uses.
            if let Some(name) =
                content_list(ui, model, "No scenes yet", ContentLayout::scene_names, icons::LAYERS, true)
            {
                actions.push(Action::OpenScene(name));
            }
        }
        NavView::Prefabs => {
            content_list(ui, model, "No prefabs yet", ContentLayout::prefab_slugs, icons::CUBE_OUTLINE, false);
        }
        NavView::Lighting => {
            content_list(
                ui, model, "No lighting states yet", ContentLayout::lighting_names, icons::WEATHER_SUNNY, false,
            );
        }
        // The this-scene view: the active scene tab's placements, grouped or flat per the sort toggle,
        // clickable to select. No state dots / hidden styling / visibility toggles - per-instance
        // physical state is not authored here (editor-design.md placement boundary); the game owns it by
        // id at runtime.
        NavView::Instances => {
            instances::instances_list(ui, model.shell.instance_sort(), model.shell.selection(), loaded_scene, actions);
        }
    }
}

/// List the open project's content for a project-scoped view, one tight row per name, returning the
/// name of the row clicked this frame (only ever `Some` when `clickable`; the caller maps it to an
/// action). `scan` is the `ContentLayout` discovery method for this view's kind - `scene_names`,
/// `prefab_slugs`, or `lighting_names` - run per frame against the open project's root. The per-frame
/// scan is simple and self-refreshing: a file added on disk shows on the next frame with no cache to
/// invalidate. (An app-side cache or a file-watch is a deferred optimization, for if the scan ever
/// costs too much; it does not for a folder listing.) `glyph` is the view's leading type glyph, painted
/// in the Instances rows' glyph column so a project-scoped row reads as one set with the tree below it
/// (Scenes a layers mark, Prefabs a cube, Lighting a sun). `clickable` marks the rows as openable
/// (Scenes); the display-only lists (Prefabs, Lighting) pass `false` and render inert. Two empty states
/// read dim and italic: no project open at all, or a project open with nothing of this kind yet (`empty`).
fn content_list(
    ui: &mut egui::Ui,
    model: &Model,
    empty: &str,
    scan: fn(&ContentLayout) -> Vec<String>,
    glyph: char,
    clickable: bool,
) -> Option<String> {
    ui.add_space(4.0);
    let Some(project) = model.project.as_ref() else {
        empty_note(ui, "No project open");
        return None;
    };
    let names = scan(&ContentLayout::new(project.root()));
    if names.is_empty() {
        empty_note(ui, empty);
        return None;
    }
    // A scroll area takes over when the list outgrows the body, so a content-heavy project can reach
    // every name rather than losing the ones past the fold (the icon bar claims its strip first, so an
    // un-scrolled overflow would simply be clipped). It draws no scrollbar until the content overflows.
    let mut clicked = None;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        // A tight file-list: rows sit close, parted by a hairline of spacing, not the form spacing the
        // panel uses elsewhere.
        ui.spacing_mut().item_spacing.y = 2.0;
        for name in &names {
            if content_row(ui, name, glyph, clickable) {
                clicked = Some(name.clone());
            }
        }
    });
    clicked
}

/// One content row: a leading type `glyph` then the name, inset by [`ROW_PAD`] from the flush panel edge
/// and centred in a fixed-height cell ([`NAV_ROW_HEIGHT`]). The glyph sits in the same `TREE_GLYPH`-wide
/// column the Instances tree's rows use ([`glyph_cell`] at [`ROW_PAD`], [`TREE_GAP`] before the name), so
/// the project-scoped lists read as one set with that tree. Returns whether it was clicked this frame
/// (always `false` for a display-only row). A `clickable` row lights up under the pointer (a hover fill
/// and brighter glyph + text) and shows the hand cursor; a display-only row stays inert. Both render at
/// the primary ink with no fill at rest. The fixed cell (not the label's own height) keeps the rows even
/// and gives the full-bleed selection highlight to come a rect to fill.
fn content_row(ui: &mut egui::Ui, name: &str, glyph: char, clickable: bool) -> bool {
    let p = theme::palette(ui.ctx());
    let sense = if clickable { egui::Sense::click() } else { egui::Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), NAV_ROW_HEIGHT), sense);
    let response = if clickable { response.on_hover_cursor(egui::CursorIcon::PointingHand) } else { response };
    let hovered = clickable && response.hovered();
    if hovered {
        ui.painter().rect_filled(rect, 0.0, p.hover);
    }
    let color = if hovered { p.text_bright } else { p.text };
    // [pad][type glyph][gap][name] - the glyph in the Instances rows' glyph column, the name one gap on.
    let glyph_rect = glyph_cell(rect, ROW_PAD);
    icons::paint(ui.painter(), glyph_rect, glyph, color);
    let pos = egui::pos2(glyph_rect.right() + TREE_GAP, rect.center().y);
    let font = egui::TextStyle::Body.resolve(ui.style());
    ui.painter().text(pos, egui::Align2::LEFT_CENTER, name, font, color);
    response.clicked()
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
