//! The chrome composition root: the one place the editor's regions are ordered, and the single entry
//! both the live frame loop (`crate::main`) and the snapshot test render through. Sharing this seam is
//! what makes the snapshot a real regression guard - the PNG shows exactly what the app draws, because
//! both go through `chrome`.
//!
//! The regions read the [`Model`](crate::model::Model) and emit [`Action`](crate::action::Action)s;
//! they never mutate the model themselves. `chrome` collects the frame's actions into a buffer and
//! returns it, and the caller applies each through `crate::action::handle` - the single writer. The
//! snapshot test renders the same `chrome` over a model it builds directly, so the PNG tracks exactly
//! what the app draws.
//!
//! Region order is layout order and load-bearing (see the README shell layout, sharp-edges 2): the
//! navigation panel is shown first so it claims its full-height strip, then the view column - the
//! status bar at the bottom, the tab bar at the top, the editor well filling the rest - spans only the
//! remaining width, so the status bar never runs under the nav. This holds whichever side the panel
//! docks to: it is added before the view column on either side. When the panel is hidden the view
//! column spans the full width. The status bar reads the open project (its name, or that none is open);
//! the tab bar renders the open tabs and the editor well names the active one (an empty well when none
//! is open); the nav panel, the status bar, the tab bar, and the hamburger's menus read the model and
//! emit actions.
//!
//! Above the well sits the floating layer: the conditional inspector (`crate::inspector`), an
//! `egui::Window` clipped to the editor area and present only when a selection resolves to a placement.
//! It is shown after the well so it layers over it, and Esc (when something is selected) emits a
//! deselect - both read the same `model.shell.selection()` the Instances tree sets. The inspector's
//! Name field commits a rename (`SetInstanceName`), and Ctrl+S (or the status-bar save dot, shown when
//! the loaded scene is dirty) emits `Save` - the editing actions the frame loop drains through the
//! single writer, the same as every other action.

use crate::action::Action;
use crate::gizmo::{self, GizmoView};
use crate::inspector;
use crate::loaded::LoadedScene;
use crate::menu;
use crate::model::Model;
use crate::workspace;

/// Render the full editor chrome for one frame: the navigation panel first (full height on its docked
/// side, and only when visible), then the view column's status bar, tab bar, and editor well, and last
/// the floating inspector over the well. Returns the actions the regions emitted this frame (for the
/// caller to apply through `crate::action::handle`) and the editor-well rect (egui points): the frame
/// loop scopes the 3D viewport to it (`crate::render`), so the rect the inspector clips to and the rect
/// the viewport draws into are one shared source. `Rect::NOTHING` until the first chrome settles, which
/// the render treats as the full target.
///
/// `loaded_scene` is the active scene tab's loaded data (reconciled by the frame loop, `crate::loaded`),
/// which the Instances nav view lists; it is `None` when no scene tab is active. The model alone cannot
/// carry it - it is filesystem residency, not pure model state - so it is threaded in separately.
///
/// `gizmo` is the transform gizmo's draw inputs (the camera and far plane the overlay projects with),
/// threaded in the same way from the frame loop where the camera and render residency live; `None` when
/// no scene is open (the static snapshot tests pass `None`, so the gizmo never enters their PNGs). When
/// present, the world-axis translate gizmo paints over the well beside the inspector, for the same
/// selection both read from `model.shell.selection()`.
pub fn chrome(
    ctx: &egui::Context,
    model: &Model,
    loaded_scene: Option<&LoadedScene>,
    gizmo: Option<GizmoView>,
) -> (Vec<Action>, egui::Rect) {
    let mut actions = Vec::new();
    // Region order is load-bearing (sharp-edges 2): the nav panel is added first on whichever side it
    // docks, so it claims its full-height strip and the view column fills the rest - the status bar
    // never runs under the nav. Hidden, the view column spans the full width.
    if model.shell.nav_visible() {
        workspace::nav_panel(ctx, model, loaded_scene, &mut actions);
    }
    // The status bar shows the save dot when the open scene has unsaved edits (residency state, not
    // model state, so it is read from the loaded scene here and passed in).
    let dirty = loaded_scene.is_some_and(|scene| scene.dirty());
    menu::status_bar(ctx, model.project.as_ref(), dirty, &mut actions);
    workspace::tab_bar(ctx, model, &mut actions);
    // The editor area is the central region left after the three bounding panels; capture it now, before
    // the central panel consumes it, so the floating inspector can anchor to and clip to it. The
    // inspector is shown after the well so it layers over it; it appears only when a selection resolves.
    let editor_rect = ctx.available_rect();
    workspace::editor_area(ctx, &mut actions);
    inspector::floating(ctx, model, loaded_scene, editor_rect, &mut actions);
    // The transform gizmo's overlay, on the same floating layer as the inspector and over the same
    // selection. It draws under the inspector window and the menus (a Background-order layer, clipped to
    // the well); the drag and the hold-key fast path are the frame loop's (`crate::gizmo::update`).
    if let Some(view) = gizmo {
        gizmo::draw(ctx, loaded_scene, model.shell.selection(), editor_rect, &view);
    }
    // Esc clears the selection (editor-design.md: Esc unwinds the selection). Gated on there being one,
    // so it is inert otherwise and never fights for the key when nothing is selected.
    if model.shell.selection().is_some() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        actions.push(Action::Deselect);
    }
    // Ctrl+S saves the open scene (editor-design.md command). Emitted on the chord; handle makes it a
    // no-op when nothing is dirty. A focused Name field still types - Ctrl+S is not a character it
    // consumes - so the save reaches the editor whether or not a field has focus.
    if ctx.input(|i| i.modifiers.command && i.key_pressed(egui::Key::S)) {
        actions.push(Action::Save);
    }
    (actions, editor_rect)
}

#[cfg(test)]
mod tests;
