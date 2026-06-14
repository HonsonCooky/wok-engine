//! The editor's egui shell: one paged left panel, a bottom status bar, the floating details
//! window, and the viewport context menu.
//!
//! This module is the coordinator: it owns the per-frame UI state ([`UiState`]), the action
//! vocabulary the surfaces emit ([`Action`]), and the composition order; the surfaces themselves
//! live in their own modules (`crate::status`, `crate::tree`, `crate::library`,
//! `crate::details`). Panels read the model and emit actions, never mutating model state
//! directly, so the frame loop stays the single writer and the UI cannot race an edit against a
//! reload. `UiState` is the one thing the UI mutates in place - page choice, the rename buffer,
//! scroll and menu flags - all presentation, never authored data.

use std::collections::HashSet;

use glam::{Quat, Vec3};
use wok_scene::{ChunkCoord, PrefabRef, Transform};

use crate::details;
use crate::drag::PlacementDrag;
use crate::input::Marquee;
use crate::library;
use crate::model::{EditorModel, Selection};
use crate::pages::{Page, PageState};
pub use crate::status::Stats;
use crate::status;
use crate::tree;

/// UI-only state that persists across frames but is never saved.
#[derive(Default)]
pub struct UiState {
    /// Which page the left panel shows.
    pub pages: PageState,
    /// Place mode: the prefab the next viewport click places.
    pub placing: Option<PrefabRef>,
    /// An inline rename in progress, if any.
    pub renaming: Option<Rename>,
    /// Chunks folded shut in the scene tree; absence means open (chunks default open).
    pub collapsed: HashSet<ChunkCoord>,
    /// Scroll the tree to the selected row this frame: set when selection arrives from the
    /// viewport, consumed by the tree build.
    pub scroll_to_selection: bool,
    /// The viewport context menu's anchor, in physical pixels, while it is open.
    pub context_menu: Option<(f32, f32)>,
    /// Raw mouse motion accumulated while the right button is held, in pixels: how the input
    /// routing tells a look-drag from a context click on release.
    pub right_drag_px: f32,
    /// A left-button drag on the selected placement in progress, if any; the same 4px rule tells
    /// it from a click. Lives here because it is interaction state, never authored data.
    pub drag: Option<PlacementDrag>,
    /// A left-button area marquee in progress, if any: a press on empty or unselected space that
    /// becomes a box past the slop. Interaction state like `drag`; the UI draws it, the input
    /// routing arms and resolves it.
    pub marquee: Option<Marquee>,
}

/// An inline rename in progress: the row being renamed and the live text.
pub struct Rename {
    pub sel: Selection,
    pub buffer: String,
    /// Give the text field keyboard focus on its first frame, then leave focus alone.
    pub take_focus: bool,
}

/// What the user asked for this frame, from the UI surfaces or the viewport input routing, applied
/// by the frame loop so model state has a single writer.
#[derive(Debug, PartialEq)]
pub enum Action {
    /// Replace the whole selection with this placement, or clear it (`None`). The plain click.
    Select(Option<Selection>),
    /// Toggle one placement in or out of the selection set, last-in becoming primary. The Ctrl+click.
    ToggleSelect(Selection),
    /// Select several placements at once - the marquee box result. `add` extends the current set
    /// (Ctrl held at release); otherwise it replaces. The model applies it as `selection.extend`,
    /// so a plain box over empty space clears and a Ctrl box over empty space leaves the set.
    SelectMany { items: Vec<Selection>, add: bool },
    Edit { sel: Selection, transform: Transform, state: Option<String> },
    /// Move every selected placement rigidly by a uniform delta - the viewport group-drag and the
    /// inspector's multi position edit, one undo step per drag (transform verbs coalesce in
    /// `crate::history`).
    MoveSelection { delta: Vec3 },
    /// Rotate every selected placement in place by `delta` (rotation = delta * rotation) - the
    /// inspector's multi rotation edit. Coalesces with the other transform verbs into one undo step.
    RotateSelection { delta: Quat },
    /// Scale every selected placement by `factor` (scale *= factor) - the inspector's multi scale
    /// edit. Coalesces with the other transform verbs into one undo step.
    ScaleSelection { factor: f32 },
    /// Set every selected placement's state - the inspector's multi state combo. A discrete mutating
    /// action: one checkpoint, like a set delete.
    SetStateSelection { state: Option<String> },
    ArmPlace(PrefabRef),
    DisarmPlace,
    /// Place the armed prefab at a viewport-resolved terrain point; the model selects the result.
    Place { prefab: PrefabRef, point: Vec3 },
    /// Duplicate every selected placement and reselect the copies (one undo step).
    Duplicate,
    /// Commit a rename with the raw edited text; the model normalizes (trims, empty clears).
    Rename { sel: Selection, name: String },
    /// Delete every selected placement (one undo step).
    Delete,
    /// Frame the fly camera on the placement's bounds.
    Frame(Selection),
    /// Write every dirty chunk and the manifest to disk through `crate::sync`.
    Save,
    /// Step the model back one checkpoint, or forward one, through `crate::history`.
    Undo,
    Redo,
}

/// Build the whole UI for one frame: status bar first (it spans the window bottom), then the
/// paged left panel, the floating details window, and the viewport context menu when open.
pub fn ui(
    ctx: &egui::Context,
    model: &EditorModel,
    ui_state: &mut UiState,
    stats: &Stats,
    actions: &mut Vec<Action>,
) {
    status::bar(ctx, &mut ui_state.pages, stats);
    // Marginless flat frame: the tree's rows paint their own hover and selection fills edge to
    // edge, so the panel must not inset them; pages that want padding add their own. The fill is
    // the same panel_fill the status bar uses, so the shell reads as one flat surface, and the
    // panel's default separator line stays the single hairline against the viewport.
    let frame = egui::Frame::new().fill(ctx.style().visuals.panel_fill);
    egui::SidePanel::left("wok_side_panel")
        .resizable(true)
        .default_width(300.0)
        .frame(frame)
        .show(ctx, |ui| match ui_state.pages.current() {
            Page::Scene => tree::page(ui, model, ui_state, actions),
            Page::Prefabs => library::page(ui, model, ui_state, actions),
            // Unreachable while the slot is disabled; harmless if it ever shows.
            Page::Scan => {
                ui.weak("content scan: not built yet");
            }
        });
    details::window(ctx, model, actions);
    viewport_menu(ctx, model, ui_state, actions);
    marquee_overlay(ctx, ui_state);
}

/// Draw the active marquee as a foreground overlay: a faint fill and a thin outline from the press
/// corner to the current cursor, so the selection box is visible as it is dragged. Only the
/// active box (past the slop) draws; the enclosed set is computed on release by
/// `crate::input::marquee`, not here. The marquee corners are physical pixels; egui paints in
/// points, so they divide by `pixels_per_point` exactly as the viewport menu's anchor does.
fn marquee_overlay(ctx: &egui::Context, ui_state: &UiState) {
    let Some(marquee) = ui_state.marquee.as_ref().filter(|m| m.active) else { return };
    let ppp = ctx.pixels_per_point();
    let rect = egui::Rect::from_two_pos(
        egui::pos2(marquee.start_px.x / ppp, marquee.start_px.y / ppp),
        egui::pos2(marquee.current_px.x / ppp, marquee.current_px.y / ppp),
    );
    let selection = ctx.style().visuals.selection;
    let fill = egui::Color32::from_rgba_unmultiplied(
        selection.bg_fill.r(),
        selection.bg_fill.g(),
        selection.bg_fill.b(),
        40,
    );
    let stroke = egui::Stroke::new(1.0, selection.stroke.color);
    let painter =
        ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("wok_marquee")));
    painter.rect(rect, 0.0, fill, stroke, egui::StrokeKind::Inside);
}

/// The viewport's context menu: the same items as a tree row's, floated at the right-click
/// position over the selected placement. Closed by choosing an item, clicking elsewhere, or the
/// selection vanishing.
fn viewport_menu(ctx: &egui::Context, model: &EditorModel, ui_state: &mut UiState, actions: &mut Vec<Action>) {
    let Some((px, py)) = ui_state.context_menu else { return };
    let Some(sel) = model.selection.primary() else {
        ui_state.context_menu = None;
        return;
    };
    let ppp = ctx.pixels_per_point();
    let pos = egui::pos2(px / ppp, py / ppp);
    let mut close = false;
    let area = egui::Area::new(egui::Id::new("wok_viewport_menu"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos)
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(110.0);
                close = tree::placement_menu(ui, sel, model, ui_state, actions);
            });
        });
    if close || area.response.clicked_elsewhere() {
        ui_state.context_menu = None;
    }
}
