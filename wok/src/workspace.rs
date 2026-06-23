//! The editor workspace: the navigation panel, the tab bar, and the editor well - the chrome's middle
//! regions, between the composition root (`crate::view`) above and egui below.
//!
//! The work is split into focused submodules, all reached through this one `workspace::` path - the
//! composition root calls [`nav_panel`], [`tab_bar`], and [`editor_area`], re-exported here:
//! - [`nav`]: the full-height navigation panel - its header (with the Instances sort toggle), the
//!   project-scoped content lists (Scenes, Prefabs, Lighting), the body dispatch, and the bottom icon
//!   bar that switches the active view.
//! - [`instances`]: the this-scene Instances view - the active scene's placements as a group-by-prefab
//!   tree (the default) or a flat A-Z list, each row selectable.
//! - [`tabs`]: the tab bar over the editor well, led by the app-menu hamburger.
//! - [`editor`]: the editor well itself - the active tab's placeholder, or the empty well (a click on
//!   which deselects).
//!
//! This module root holds what those submodules share: the shell's layout constants (one place for the
//! panel widths, the bar heights, the row metrics, and the tree glyph column) and the two cross-cutting
//! paint helpers - [`glyph_cell`] (the aligned tree/content glyph column) and [`empty_note`] (the dim
//! italic body note). Every colour is read through `theme::palette`, so the chrome follows the OS
//! light/dark.

use crate::theme;

mod editor;
mod instances;
mod nav;
mod tabs;

pub use editor::editor_area;
pub use nav::nav_panel;
pub use tabs::tab_bar;
// Exposed at the workspace level only for the snapshot test (see [`instances::instance_group_id`]'s own
// doc): it seeds a group's open state under the very id the view reads. The live view reads that id
// within `instances`, so outside the test there is nothing to re-export.
#[cfg(test)]
pub(crate) use instances::instance_group_id;

// ---- shell layout constants ----

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
/// in points. A touch over the icon [`icons::SIZE`](crate::icons::SIZE) so the ~12px glyph has a little
/// room in its column and the glyph columns line up down the tree.
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

// ---- shared paint helpers ----

/// A [`TREE_GLYPH`]-wide glyph cell at `left`, spanning the row's full height, for
/// [`icons::paint`](crate::icons::paint) to centre a tree glyph in. Sharing it keeps the Instances
/// tree's folder and cube columns aligned, and the project-list rows' leading type glyph
/// ([`content_row`](nav::content_row)) in that same column.
fn glyph_cell(row: egui::Rect, left: f32) -> egui::Rect {
    egui::Rect::from_min_size(egui::pos2(left, row.top()), egui::vec2(TREE_GLYPH, row.height()))
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
