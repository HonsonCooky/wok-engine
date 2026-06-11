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

use wok_scene::{PrefabRef, Transform};

use crate::details;
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
    /// Scroll the tree to the selected row this frame: set when selection arrives from the
    /// viewport, consumed by the tree build.
    pub scroll_to_selection: bool,
    /// The viewport context menu's anchor, in physical pixels, while it is open.
    pub context_menu: Option<(f32, f32)>,
    /// Raw mouse motion accumulated while the right button is held, in pixels: how the input
    /// routing tells a look-drag from a context click on release.
    pub right_drag_px: f32,
}

/// An inline rename in progress: the row being renamed and the live text.
pub struct Rename {
    pub sel: Selection,
    pub buffer: String,
    /// Give the text field keyboard focus on its first frame, then leave focus alone.
    pub take_focus: bool,
}

/// What the user asked for this frame, applied by the frame loop after the UI runs.
pub enum Action {
    Select(Option<Selection>),
    Edit { sel: Selection, transform: Transform, state: Option<String> },
    ArmPlace(PrefabRef),
    DisarmPlace,
    Duplicate(Selection),
    /// Commit a rename with the raw edited text; the model normalizes (trims, empty clears).
    Rename { sel: Selection, name: String },
    Delete(Selection),
    /// Frame the fly camera on the placement's bounds.
    Frame(Selection),
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
    egui::SidePanel::left("wok_side_panel")
        .resizable(true)
        .default_width(300.0)
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
}

/// The viewport's context menu: the same items as a tree row's, floated at the right-click
/// position over the selected placement. Closed by choosing an item, clicking elsewhere, or the
/// selection vanishing.
fn viewport_menu(ctx: &egui::Context, model: &EditorModel, ui_state: &mut UiState, actions: &mut Vec<Action>) {
    let Some((px, py)) = ui_state.context_menu else { return };
    let Some(sel) = model.selection else {
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
