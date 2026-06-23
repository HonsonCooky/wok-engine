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
//! views (Scenes, Prefabs, Lighting) through `wok_scene::ContentLayout` discovery, scanned per frame; a
//! Scenes row opens that scene as a tab (`Action::OpenScene`), while Prefabs and Lighting rows stay
//! display-only (opening those contexts is a later bite). The this-scene Instances view lists the active
//! scene tab's placements (from the reconciled `LoadedScene` threaded in) two ways, on the header's sort
//! toggle: grouped under their prefab as a collapsible tree (the default), or a flat A-Z list. Clicking
//! an instance row emits `Action::Select`; the selected row carries the full-bleed accent highlight, and
//! the floating inspector (`crate::inspector`) shows it. The panel docks to either side and toggles
//! through the View menu
//! (`crate::menu`) - the composition root shows `nav_panel` only when visible, on the model's chosen
//! side, and the menu drives both - and it resizes by dragging its inner edge, with egui owning the live
//! width (so there is no Shell state for it). The tab bar renders the model's open tabs: a click selects
//! a tab, its close glyph closes it, and the editor well names the active tab (the per-context surface -
//! the 3D viewport, the data views - lands in later bites). Every colour is read through
//! `theme::palette`, so the chrome follows the OS light/dark.

use wok_scene::{ContentLayout, InstanceId, Placement};

use crate::action::Action;
use crate::icons;
use crate::loaded::LoadedScene;
use crate::menu;
use crate::model::{InstanceSort, Model, NavSide, NavView};
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

/// The column width reserved for a tree row's glyph - the group chevron and folder, the instance cube -
/// in points. A touch over the icon [`icons::SIZE`] so the ~12px glyph has a little room in its column
/// and the glyph columns line up down the tree.
const TREE_GLYPH: f32 = 16.0;

/// The gap in points between a tree row's last glyph and the text that follows it.
const TREE_GAP: f32 = 4.0;

/// An instance row's indent in points (handoff view 2: instance rows indented ~30px): one glyph-column
/// past the group's folder ([`ROW_PAD`] + [`TREE_GLYPH`] + [`TREE_GAP`] = 30), so the instance cube
/// lands under the group's prefab name and its label one step further in. The empty column to the left
/// is what reads as the nesting now that the disclosure chevron is gone (the folder glyph is the
/// disclosure).
const INSTANCE_INDENT: f32 = ROW_PAD + TREE_GLYPH + TREE_GAP;

/// The alpha of the selected row's full-bleed highlight: the accent at ~30% (handoff view 2: an
/// accent-at-30% fill spanning the full panel width, no inset or rounded pill). 0x4d/0xff is 30%.
const SELECTION_ALPHA: u8 = 0x4d;

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
            content_list(ui, model, "No prefabs yet", ContentLayout::prefab_slugs, icons::CUBE, false);
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
            instances_list(ui, model.shell.instance_sort(), model.shell.selection(), loaded_scene, actions);
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
/// and centred in a fixed-height cell ([`NAV_ROW_HEIGHT`]). The glyph sits in the same [`TREE_GLYPH`]-wide
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

/// The Instances view: the active scene tab's placements, laid out per the header's sort toggle -
/// grouped under their prefab as a collapsible tree ([`instances_tree`], the default), or a flat A-Z
/// list ([`instances_flat`]). A row click emits [`Action::Select`]; the row matching `selection` carries
/// the full-bleed highlight. Three resting states with nothing to list: no scene tab active
/// (`loaded_scene` is `None`) reads "No scene open"; a scene that failed to load reads "Scene failed to
/// load" (the detail is noted on the residency for a later bite to surface); and a loaded scene with no
/// placements reads "No instances".
fn instances_list(
    ui: &mut egui::Ui,
    sort: InstanceSort,
    selection: Option<InstanceId>,
    loaded_scene: Option<&LoadedScene>,
    actions: &mut Vec<Action>,
) {
    ui.add_space(4.0);
    let Some(loaded) = loaded_scene else {
        empty_note(ui, "No scene open");
        return;
    };
    // A failed load is empty too, but it is not "no instances" - say so rather than misreport it.
    if loaded.error().is_some() {
        empty_note(ui, "Scene failed to load");
        return;
    }
    let placements = loaded.placements();
    if placements.is_empty() {
        empty_note(ui, "No instances");
        return;
    }
    // A scroll area takes over when the list outgrows the body, the same as the project-scoped lists,
    // so a placement-heavy scene reaches every row rather than clipping past the fold. The rows sit
    // flush against each other (no inter-row spacing) so the tree reads tight and the full-bleed
    // selection highlight is one continuous band, the Zed file-tree look.
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        match sort {
            InstanceSort::Group => instances_tree(ui, loaded.name(), placements, selection, actions),
            InstanceSort::Flat => instances_flat(ui, placements, selection, actions),
        }
    });
}

/// The Instances view's flat layout: every placement as one selectable row at the base indent, sorted
/// A-Z by its display label. The order is on the label the row reads, so a named placement sorts by its
/// name and an unnamed one by its `{prefab} #{id}` fallback - not by the underlying instance id. A click
/// emits [`Action::Select`]; the row whose id is `selection` carries the highlight.
fn instances_flat(
    ui: &mut egui::Ui,
    placements: &[Placement],
    selection: Option<InstanceId>,
    actions: &mut Vec<Action>,
) {
    let mut sorted: Vec<&Placement> = placements.iter().collect();
    sorted.sort_by_key(|p| placement_label(p));
    for placement in sorted {
        if instance_row(ui, placement, ROW_PAD, selection == Some(placement.instance_id)) {
            actions.push(Action::Select(placement.instance_id));
        }
    }
}

/// The Instances view's grouped layout: one collapsible group row per prefab (sorted by prefab name)
/// carrying its instance count, and under an open group the indented, selectable instance rows (in
/// instance-id order, the residency's order). The group open state is egui-managed transient view
/// memory, not model state - collapsing a group is a browsing affordance, never an authored edit. A
/// click on an instance row emits [`Action::Select`]; the row whose id is `selection` carries the
/// highlight.
fn instances_tree(
    ui: &mut egui::Ui,
    scene: &str,
    placements: &[Placement],
    selection: Option<InstanceId>,
    actions: &mut Vec<Action>,
) {
    for (prefab, members) in group_by_prefab(placements) {
        if group_row(ui, scene, prefab, members.len()) {
            for placement in members {
                if instance_row(ui, placement, INSTANCE_INDENT, selection == Some(placement.instance_id)) {
                    actions.push(Action::Select(placement.instance_id));
                }
            }
        }
    }
}

/// Bucket placements by their prefab, returning one entry per prefab - its name and its members - with
/// the entries sorted by prefab name. Members keep the input's order, which the residency sorts by
/// instance id, so each bucket comes out id-ordered without a second sort. Pure (no egui), so the
/// grouping is unit tested directly. A linear scan per placement is ample at editor scene scale.
fn group_by_prefab(placements: &[Placement]) -> Vec<(&str, Vec<&Placement>)> {
    let mut groups: Vec<(&str, Vec<&Placement>)> = Vec::new();
    for placement in placements {
        let prefab = placement.prefab.as_str();
        match groups.iter_mut().find(|(name, _)| *name == prefab) {
            Some((_, members)) => members.push(placement),
            None => groups.push((prefab, vec![placement])),
        }
    }
    groups.sort_by(|a, b| a.0.cmp(b.0));
    groups
}

/// One prefab group row: a folder glyph (open or closed, the disclosure - the chevron was dropped), the
/// prefab name, and the instance count on the right, in a fixed-height cell. The whole row senses a
/// click that toggles the group's open state - egui transient memory keyed per scene and prefab
/// ([`instance_group_id`]), so it outlives a frame but is never authored. Returns whether the group is
/// open, so the caller renders its instance rows. Hovering lights the row.
fn group_row(ui: &mut egui::Ui, scene: &str, prefab: &str, count: usize) -> bool {
    let p = theme::palette(ui.ctx());
    let id = instance_group_id(scene, prefab);
    let mut open = ui.data_mut(|d| d.get_temp::<bool>(id).unwrap_or(true));
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), NAV_ROW_HEIGHT), egui::Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if response.clicked() {
        open = !open;
        ui.data_mut(|d| d.insert_temp(id, open));
    }
    let hovered = response.hovered();
    if hovered {
        ui.painter().rect_filled(rect, 0.0, p.hover);
    }
    let ink = if hovered { p.text_bright } else { p.text };
    // [pad][folder][prefab name .......][count][pad] - the open/closed folder is the disclosure.
    let folder = if open { icons::FOLDER_OPEN } else { icons::FOLDER };
    let folder_rect = glyph_cell(rect, ROW_PAD);
    icons::paint(ui.painter(), folder_rect, folder, ink);
    let font = egui::TextStyle::Body.resolve(ui.style());
    let name_pos = egui::pos2(folder_rect.right() + TREE_GAP, rect.center().y);
    ui.painter().text(name_pos, egui::Align2::LEFT_CENTER, prefab, font.clone(), ink);
    let count_pos = egui::pos2(rect.right() - ROW_PAD, rect.center().y);
    ui.painter().text(count_pos, egui::Align2::RIGHT_CENTER, count.to_string(), font, p.text_dim);
    open
}

/// One selectable instance row: a cube glyph at `indent` and the placement's label, filling the row.
/// `selected` paints the full-bleed selection highlight (the accent at [`SELECTION_ALPHA`] across the
/// whole panel width); otherwise the row lights on hover. Returns whether it was clicked this frame, for
/// the caller to emit [`Action::Select`]. Shared by the grouped tree (indented under its group, at
/// [`INSTANCE_INDENT`]) and the flat list (at the base [`ROW_PAD`]).
fn instance_row(ui: &mut egui::Ui, placement: &Placement, indent: f32, selected: bool) -> bool {
    let p = theme::palette(ui.ctx());
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), NAV_ROW_HEIGHT), egui::Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let hovered = response.hovered();
    if selected {
        let fill = egui::Color32::from_rgba_unmultiplied(p.accent.r(), p.accent.g(), p.accent.b(), SELECTION_ALPHA);
        ui.painter().rect_filled(rect, 0.0, fill);
    } else if hovered {
        ui.painter().rect_filled(rect, 0.0, p.hover);
    }
    let ink = if selected || hovered { p.text_bright } else { p.text };
    let cube_rect = glyph_cell(rect, indent);
    icons::paint(ui.painter(), cube_rect, icons::CUBE, ink);
    let font = egui::TextStyle::Body.resolve(ui.style());
    let pos = egui::pos2(cube_rect.right() + TREE_GAP, rect.center().y);
    ui.painter().text(pos, egui::Align2::LEFT_CENTER, placement_label(placement), font, ink);
    response.clicked()
}

/// A [`TREE_GLYPH`]-wide glyph cell at `left`, spanning the row's full height, for [`icons::paint`] to
/// centre a tree glyph in. Sharing it keeps the Instances tree's folder and cube columns aligned, and
/// the project-list rows' leading type glyph ([`content_row`]) in that same column.
fn glyph_cell(row: egui::Rect, left: f32) -> egui::Rect {
    egui::Rect::from_min_size(egui::pos2(left, row.top()), egui::vec2(TREE_GLYPH, row.height()))
}

/// The egui-memory id a prefab group's open state is stored under, keyed per scene and prefab so
/// collapsing a group in one scene leaves a same-named prefab's group in another scene at its default.
/// `pub(crate)` so the snapshot test can drive a group collapsed by seeding the very id the view reads
/// (the open state is transient view memory, not model state a test could build directly).
pub(crate) fn instance_group_id(scene: &str, prefab: &str) -> egui::Id {
    egui::Id::new(("wok_instances_group", scene, prefab))
}

/// One placement's display label: its author-given name when set, else the `{prefab} #{id}` fallback
/// (e.g. `oak_tree #3`) - enough to tell two instances of the same prefab apart by their stable
/// instance id. Display formatting, so it lives view-side with the rows that show it, not on the data;
/// both the flat list and the grouped tree's instance rows read from it.
fn placement_label(placement: &Placement) -> String {
    match &placement.name {
        Some(name) => name.clone(),
        None => format!("{} #{}", placement.prefab.as_str(), placement.instance_id.0),
    }
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
/// Run / Help menu), then one cell per open tab (`model.shell.tabs`). With no tab open the bar is just
/// the hamburger. Hand-drawn (egui has no tab widget). A click on a tab selects it and its close glyph
/// closes it (`tab_cell`); the active tab (`model.shell.active_tab`) carries the active styling.
pub fn tab_bar(ctx: &egui::Context, model: &Model, actions: &mut Vec<Action>) {
    egui::TopBottomPanel::top("wok_tab_bar").exact_height(TAB_BAR_HEIGHT).show(ctx, |ui| {
        ui.horizontal_centered(|ui| {
            menu::hamburger(ui, model, actions);
            ui.add_space(8.0);
            // Tabs nearly touch, as in Zed, with the active fill the only thing parting them.
            ui.spacing_mut().item_spacing.x = 1.0;
            let active = model.shell.active_tab();
            for (i, tab) in model.shell.tabs().iter().enumerate() {
                tab_cell(ui, tab.title(), active == Some(i), i, actions);
            }
        });
    });
}

/// One tab cell: the title and a close glyph over the tab fill. The active tab borrows the editor
/// surface (so it reads continuous with the well below) and carries the accent as a top line; an
/// inactive tab sits flat and dim. The whole cell senses one click: a click landing on the close glyph
/// emits `CloseTab` (decided by hit-testing the glyph's rect, so there are no overlapping click widgets
/// racing for the press), any other click on the cell emits `SelectTab`.
fn tab_cell(ui: &mut egui::Ui, title: &str, active: bool, index: usize, actions: &mut Vec<Action>) {
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
            // Non-selectable so the labels sense nothing - no click to fight the cell's, and no
            // text/IBeam cursor on hover (the cell below owns the interaction and sets the cursor).
            ui.add(egui::Label::new(title).selectable(false));
            // The close affordance, the same Nerd Font family as the rest of the chrome, sized small so
            // it sits quietly beside the title. Its rect is returned so a click landing on it closes the
            // tab rather than selecting it.
            let close = egui::RichText::new(icons::CLOSE).size(10.0).color(p.text_dim);
            ui.add(egui::Label::new(close).selectable(false)).rect
        })
        .inner
    });
    let close_rect = inner.inner;
    // One click-sensing region over the whole cell (the labels sense nothing), so close-vs-select is
    // decided by where the press landed, not by overlapping widgets fighting for it. The whole tab shows
    // the pointing-hand cursor (it is clickable), including over the close glyph.
    let cell = inner.response.interact(egui::Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    if cell.clicked() {
        let on_close = cell.interact_pointer_pos().is_some_and(|pos| close_rect.contains(pos));
        actions.push(if on_close { Action::CloseTab(index) } else { Action::SelectTab(index) });
    }
    if active {
        let rect = inner.response.rect;
        ui.painter().hline(rect.x_range(), rect.top(), egui::Stroke::new(2.0, p.accent));
    }
}

/// The editor area: the active tab's placeholder, or an empty well when no tab is open. A transparent
/// panel over the editor-background backdrop (the GPU clear in the live app, the snapshot harness's
/// background fill in the test), so the well reads as `editor_bg`. With a tab open it names the open
/// scene, dim and centred - the stand-in for the per-context surface (the 3D viewport, the data views),
/// which lands in later bites, drawn into this same transparent panel; with no tab open it is the bare
/// well.
///
/// A click on the empty well clears the selection (editor-design.md: a click on empty space deselects;
/// viewport picking that selects on a hit lands with the 3D, a later bite). The floating inspector is on
/// a higher layer, so a click landing on it does not reach here - egui assigns the click to the topmost
/// area, so the well's `clicked()` is false under the window.
pub fn editor_area(ctx: &egui::Context, model: &Model, actions: &mut Vec<Action>) {
    egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
        let well = ui.interact(ui.max_rect(), ui.id().with("editor_well"), egui::Sense::click());
        if well.clicked() && model.shell.selection().is_some() {
            actions.push(Action::Deselect);
        }
        let Some(tab) = model.shell.active_tab().and_then(|i| model.shell.tabs().get(i)) else {
            return;
        };
        let p = theme::palette(ui.ctx());
        let center = ui.max_rect().center();
        // The open scene's name over a one-line hint that the real surface is still to come, both dim
        // and centred on the well. Painted directly (like the nav rows) rather than laid out, so the
        // block sits at the centre regardless of the panel size; the name sits just above the centre
        // line and the hint just below.
        let name = egui::FontId::proportional(20.0);
        let hint = egui::FontId::proportional(12.0);
        let painter = ui.painter();
        painter.text(center, egui::Align2::CENTER_BOTTOM, tab.title(), name, p.text_dim);
        painter.text(center + egui::vec2(0.0, 6.0), egui::Align2::CENTER_TOP, "viewport lands here", hint, p.text_dim);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use wok_scene::{InstanceId, PrefabRef, Transform};

    fn placement(prefab: &str, id: u32) -> Placement {
        Placement {
            prefab: PrefabRef::new(prefab),
            instance_id: InstanceId(id),
            name: None,
            transform: Transform::IDENTITY,
            state: None,
        }
    }

    #[test]
    fn group_by_prefab_buckets_sorted_by_name_with_id_ordered_members() {
        // Input in instance-id order (the residency's order) with the prefabs interleaved and out of
        // name order, so a pass proves the bucketing and the name sort rather than incidental input
        // order: well #0, oak_tree #1, well #2, oak_tree #3, oak_tree #4.
        let placements = vec![
            placement("well", 0),
            placement("oak_tree", 1),
            placement("well", 2),
            placement("oak_tree", 3),
            placement("oak_tree", 4),
        ];
        let groups = group_by_prefab(&placements);

        // One entry per distinct prefab, sorted by prefab name (oak_tree before well), counts correct.
        let summary: Vec<(&str, usize)> = groups.iter().map(|(name, members)| (*name, members.len())).collect();
        assert_eq!(summary, vec![("oak_tree", 3), ("well", 2)]);

        // Members keep instance-id order within each group (the id-ordered input, bucketed in place).
        let oak_ids: Vec<u32> = groups[0].1.iter().map(|p| p.instance_id.0).collect();
        assert_eq!(oak_ids, vec![1, 3, 4]);
        let well_ids: Vec<u32> = groups[1].1.iter().map(|p| p.instance_id.0).collect();
        assert_eq!(well_ids, vec![0, 2]);
    }
}
