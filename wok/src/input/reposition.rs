//! The viewport reposition drag: one held frame turns the cursor into a uniform move of the whole
//! selection.
//!
//! `crate::drag` owns the placement-space math (the grab anchor, the terrain/vertical resolve);
//! this is the input-side step that grabs one selected member, resolves where the cursor puts it,
//! and emits the group's delta as an [`Action::MoveSelection`] for the frame loop to apply over
//! the whole set. `crate::input::viewport` owns the arming and the press/hold/release lifecycle
//! around it. The grabbed member drives the delta; the rest of the set follows rigidly (the model
//! adds the same delta to every selected placement - no per-item terrain re-rest).

use glam::{Vec2, Vec3};
use wok_platform::input::InputState;
use wok_platform::winit::keyboard::NamedKey;

use crate::drag::{DragMode, PlacementDrag, drag_offset, dragged_translation};
use crate::model::{EditorModel, chunk_origin};
use crate::panels::Action;

use super::CLICK_SLOP_PX;

/// One held frame of a reposition drag: enforce the slop, pick the mode by Shift (re-anchoring
/// whenever the mode is entered, so nothing jumps to the cursor), resolve the grabbed placement's
/// new translation, and emit the group's delta as [`Action::MoveSelection`]. A frame whose cursor
/// gives nothing to track (no terrain under it, a degenerate ray) holds the group still.
pub(crate) fn step(
    input: &InputState,
    eye: Vec3,
    dir: Vec3,
    far: f32,
    model: &EditorModel,
    drag: &mut PlacementDrag,
    cursor: Vec2,
    actions: &mut Vec<Action>,
) {
    if !drag.active {
        if (cursor - drag.press_px).length() < CLICK_SLOP_PX {
            return;
        }
        drag.active = true;
    }
    let Some(placement) = model.placement(drag.sel) else { return };
    let current = placement.transform;
    let Some(prefab) = model.prefabs.get(&placement.prefab) else { return };
    let origin = chunk_origin(drag.sel.coord);

    let mode = if input.key_held(NamedKey::Shift) { DragMode::Vertical } else { DragMode::Terrain };
    if drag.anchor.map(|(m, _)| m) != Some(mode) {
        drag.anchor = drag_offset(mode, &current, origin, &model.heightmaps, eye, dir, far)
            .map(|offset| (mode, offset));
    }
    let Some((_, offset)) = drag.anchor else { return };
    let Some(translation) =
        dragged_translation(mode, offset, &current, origin, prefab, &model.heightmaps, eye, dir, far)
    else {
        return;
    };
    // The grabbed placement drives the delta; the same delta moves the whole set rigidly. A
    // displacement is frame-independent, so it is correct to add to placements in other chunks too.
    let delta = translation - current.translation;
    if delta != Vec3::ZERO {
        actions.push(Action::MoveSelection { delta });
    }
}
