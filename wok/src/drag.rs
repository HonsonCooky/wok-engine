//! Drag-to-move math: where a viewport drag puts the selected placement.
//!
//! Two modes, chosen by whether Shift is held on each frame of the drag. Terrain mode follows the
//! cursor's terrain-ray hit: the hit drives x and z (offset by where on the shape the user
//! grabbed, so the placement never jumps to put its origin under the cursor), and the shape's
//! rest policy re-derives y from the terrain under the new spot every frame - the same
//! `crate::place` policy that placed it. Vertical mode moves y only: the placement slides along
//! its own vertical line to the point on that line nearest the cursor ray, so it tracks the
//! cursor on screen without x/z ever moving.
//!
//! The dragged placement stays in its authored chunk regardless of where it ends up - exactly the
//! rule the details panel's position fields already follow (chunk-local position is
//! unconstrained) - but the terrain that derives y is sampled by world position, so dragging past
//! a chunk edge rests on the neighbour's heightmap, not on a clamped edge of the home chunk.
//!
//! Everything here is pure (rays and transforms in, a translation out). `crate::input` owns the
//! 4px click-or-drag rule, the anchoring lifecycle, and the commit through the model.

use std::collections::BTreeMap;

use glam::{Vec2, Vec3};
use wok_scene::{ChunkCoord, Heightmap, Prefab, Transform};

use crate::model::{Selection, chunk_at, chunk_origin};
use crate::pick;
use crate::place;

/// How a drag frame maps the cursor to movement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragMode {
    /// The cursor's terrain hit drives x/z; the rest policy re-derives y.
    Terrain,
    /// Shift: y only, along the placement's vertical line.
    Vertical,
}

/// A left-button drag on the selected placement, from press to release. Owned by `UiState`
/// (presentation state, like the right button's look-or-click accumulator) and driven by
/// `crate::input` each frame the button stays down.
pub struct PlacementDrag {
    /// The placement the press landed on - always the current selection at press time.
    pub sel: Selection,
    /// Cursor position at press, physical pixels: the 4px click-or-drag rule measures from here.
    pub press_px: Vec2,
    /// The drag has crossed the slop and is moving the placement; until then a release is a click.
    pub active: bool,
    /// The current mode and its grab offset, captured when the mode is entered (and re-captured
    /// when Shift toggles mid-drag). `None` until a frame's cursor gives a valid anchor.
    pub anchor: Option<(DragMode, Vec3)>,
}

/// The grab offset captured when a mode is entered: what keeps the placement from jumping on the
/// first moved frame. Terrain mode: the x/z vector from the cursor's terrain hit to the
/// placement, so the grabbed spot stays under the cursor. Vertical mode: the y distance from the
/// ray's nearest point on the placement's vertical line to the placement. `None` when the cursor
/// gives no anchor this frame (no terrain under it, or a vertical ray); the caller retries next
/// frame.
pub fn drag_offset(
    mode: DragMode,
    current: &Transform,
    owning_origin: Vec3,
    heightmaps: &BTreeMap<ChunkCoord, Heightmap>,
    eye: Vec3,
    dir: Vec3,
    far: f32,
) -> Option<Vec3> {
    let world = current.translation + owning_origin;
    match mode {
        DragMode::Terrain => {
            let hit = pick::terrain_hit(heightmaps, eye, dir, far)?;
            Some(Vec3::new(world.x - hit.point.x, 0.0, world.z - hit.point.z))
        }
        DragMode::Vertical => {
            let s = vertical_param(Vec2::new(world.x, world.z), eye, dir)?;
            Some(Vec3::new(0.0, world.y - s, 0.0))
        }
    }
}

/// One frame of an active drag: the new chunk-local translation (relative to the owning chunk),
/// or `None` when this frame's cursor gives nothing to track (no terrain under it, a vertical
/// ray, or the nearest point behind the eye), in which case the placement holds still.
pub fn dragged_translation(
    mode: DragMode,
    offset: Vec3,
    current: &Transform,
    owning_origin: Vec3,
    prefab: &Prefab,
    heightmaps: &BTreeMap<ChunkCoord, Heightmap>,
    eye: Vec3,
    dir: Vec3,
    far: f32,
) -> Option<Vec3> {
    let world = current.translation + owning_origin;
    match mode {
        DragMode::Terrain => {
            let hit = pick::terrain_hit(heightmaps, eye, dir, far)?;
            let target = Vec3::new(hit.point.x + offset.x, world.y, hit.point.z + offset.z);
            // Rest on the terrain under the target, in that chunk's local frame (chunk origins
            // have no y component, so the rested y is world y either way).
            let coord = chunk_at(target);
            let origin = chunk_origin(coord);
            let floating = Transform { translation: target - origin, ..*current };
            let rested = match heightmaps.get(&coord) {
                Some(hm) => place::rest_on_terrain(prefab, place::rest_for_prefab(prefab), floating, hm),
                // No terrain under the new spot: x/z still follow, y keeps its current height.
                None => floating,
            };
            Some(rested.translation + origin - owning_origin)
        }
        DragMode::Vertical => {
            let s = vertical_param(Vec2::new(world.x, world.z), eye, dir)?;
            Some(Vec3::new(current.translation.x, s + offset.y - owning_origin.y, current.translation.z))
        }
    }
}

/// The y at which the vertical line through `line_xz` passes nearest the ray: the closest-point
/// solution between two lines, solved for the vertical line's parameter, which for a line through
/// y = 0 is the y itself. `None` when the ray is vertical too (parallel lines have no single
/// nearest point) or when the nearest point lies behind the eye (the user is aiming away).
pub fn vertical_param(line_xz: Vec2, eye: Vec3, dir: Vec3) -> Option<f32> {
    // Lines P(s) = P0 + s*Y and Q(t) = eye + t*dir, w0 = P0 - eye. With both directions unit
    // length the classic closest-point system reduces to b = dir.y, denom = 1 - b^2.
    let w0 = Vec3::new(line_xz.x, 0.0, line_xz.y) - eye;
    let b = dir.y;
    let denom = 1.0 - b * b;
    if denom <= 1e-6 {
        return None;
    }
    let d = w0.y;
    let e = dir.dot(w0);
    let t = (e - b * d) / denom;
    if t <= 0.0 {
        return None;
    }
    Some((b * e - d) / denom)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wok_scene::{CHUNK_GRID_LEN, PrefabState, Primitive, Shape, SurfaceTag};

    fn flat_heightmap(height_m: f32) -> Heightmap {
        let raw = Heightmap::meters_to_raw(height_m);
        Heightmap::new(vec![raw; CHUNK_GRID_LEN], vec![SurfaceTag::new("grass")], vec![0; CHUNK_GRID_LEN])
            .expect("flat grid has the right length")
    }

    fn flat_world(height_m: f32) -> BTreeMap<ChunkCoord, Heightmap> {
        let mut heightmaps = BTreeMap::new();
        heightmaps.insert(ChunkCoord::new(0, 0), flat_heightmap(height_m));
        heightmaps
    }

    /// A 2m cube prefab: corner rest, so on flat ground at 5m its centre rests at y = 6.
    fn cube_prefab() -> Prefab {
        Prefab {
            states: vec![PrefabState {
                name: "default".to_string(),
                shapes: vec![Shape {
                    primitive: Primitive::Cube,
                    transform: Transform { scale: Vec3::splat(2.0), ..Transform::IDENTITY },
                    surface: None,
                    is_hitbox: true,
                    is_visible: true,
                }],
                mesh: None,
            }],
            default_state: "default".to_string(),
        }
    }

    fn at(translation: Vec3) -> Transform {
        Transform { translation, ..Transform::IDENTITY }
    }

    // ---- terrain-projected drag ----

    #[test]
    fn terrain_drag_moves_xz_to_the_hit_and_rests_y_on_the_terrain() {
        let heightmaps = flat_world(5.0);
        let current = at(Vec3::new(30.0, 6.0, 30.0));
        // Straight down over (40, 35): the terrain hit is exactly there, at ground height 5.
        let eye = Vec3::new(40.0, 30.0, 35.0);
        let moved = dragged_translation(
            DragMode::Terrain,
            Vec3::ZERO,
            &current,
            Vec3::ZERO,
            &cube_prefab(),
            &heightmaps,
            eye,
            Vec3::NEG_Y,
            100.0,
        )
        .expect("the ray hits terrain");
        assert!((moved.x - 40.0).abs() < 1e-3 && (moved.z - 35.0).abs() < 1e-3, "xz follows: {moved:?}");
        assert!((moved.y - 6.0).abs() < 1e-2, "the 2m cube rests its bottom on the 5m ground: {moved:?}");
    }

    #[test]
    fn the_grab_offset_makes_the_first_dragged_frame_a_no_op() {
        // The anti-jump property: capturing the offset and immediately stepping with the same
        // cursor must land exactly where the placement already is.
        let heightmaps = flat_world(5.0);
        let current = at(Vec3::new(30.0, 6.0, 30.0));
        let eye = Vec3::new(40.0, 30.0, 35.0);
        let offset = drag_offset(
            DragMode::Terrain, &current, Vec3::ZERO, &heightmaps, eye, Vec3::NEG_Y, 100.0,
        )
        .expect("anchored");
        assert!((offset - Vec3::new(-10.0, 0.0, -5.0)).length() < 1e-3, "offset {offset:?}");
        let moved = dragged_translation(
            DragMode::Terrain, offset, &current, Vec3::ZERO, &cube_prefab(), &heightmaps, eye,
            Vec3::NEG_Y, 100.0,
        )
        .expect("still over terrain");
        assert!((moved.x - 30.0).abs() < 1e-3 && (moved.z - 30.0).abs() < 1e-3, "no jump: {moved:?}");
    }

    #[test]
    fn terrain_drag_with_no_terrain_under_the_cursor_holds_still() {
        let heightmaps = flat_world(5.0);
        let current = at(Vec3::new(30.0, 6.0, 30.0));
        // A ray into the sky never crosses terrain: the frame reports nothing to track.
        let moved = dragged_translation(
            DragMode::Terrain, Vec3::ZERO, &current, Vec3::ZERO, &cube_prefab(), &heightmaps,
            Vec3::new(30.0, 10.0, 40.0), Vec3::Y, 100.0,
        );
        assert_eq!(moved, None);
    }

    // ---- Shift-vertical drag ----

    #[test]
    fn vertical_drag_tracks_the_cursor_ray_in_y_only() {
        let heightmaps = flat_world(5.0);
        let current = at(Vec3::new(30.0, 6.0, 30.0));
        // A ray aimed exactly at (30, 8, 30) passes through the placement's vertical line at
        // y = 8: the placement rises to meet the cursor, x/z untouched.
        let eye = Vec3::new(30.0, 10.0, 40.0);
        let dir = (Vec3::new(30.0, 8.0, 30.0) - eye).normalize();
        let moved = dragged_translation(
            DragMode::Vertical, Vec3::ZERO, &current, Vec3::ZERO, &cube_prefab(), &heightmaps,
            eye, dir, 100.0,
        )
        .expect("the ray crosses the line");
        assert!((moved.y - 8.0).abs() < 1e-3, "y follows the ray: {moved:?}");
        assert!((moved.x - 30.0).abs() < 1e-6 && (moved.z - 30.0).abs() < 1e-6, "xz frozen: {moved:?}");
    }

    #[test]
    fn the_vertical_grab_offset_also_makes_the_first_frame_a_no_op() {
        let current = at(Vec3::new(30.0, 6.0, 30.0));
        let eye = Vec3::new(30.0, 10.0, 40.0);
        let dir = (Vec3::new(30.0, 8.0, 30.0) - eye).normalize();
        let offset = drag_offset(
            DragMode::Vertical, &current, Vec3::ZERO, &BTreeMap::new(), eye, dir, 100.0,
        )
        .expect("anchored");
        assert!((offset.y - (-2.0)).abs() < 1e-3, "grabbed 2m above the nearest point: {offset:?}");
        let moved = dragged_translation(
            DragMode::Vertical, offset, &current, Vec3::ZERO, &cube_prefab(), &BTreeMap::new(),
            eye, dir, 100.0,
        )
        .expect("same ray still anchors");
        assert!((moved - current.translation).length() < 1e-3, "no jump: {moved:?}");
    }

    #[test]
    fn a_vertical_ray_cannot_drive_a_vertical_drag() {
        // Looking straight down, the ray and the placement's line are parallel: no single
        // nearest point, so the frame reports nothing rather than a wild y.
        assert_eq!(vertical_param(Vec2::new(30.0, 30.0), Vec3::new(31.0, 50.0, 30.0), Vec3::NEG_Y), None);
    }

    #[test]
    fn a_ray_aimed_away_from_the_line_does_not_anchor() {
        // The nearest point on the line sits behind the eye: aiming away must not yank the
        // placement backwards through the camera.
        let eye = Vec3::new(30.0, 10.0, 40.0);
        let away = (eye - Vec3::new(30.0, 8.0, 30.0)).normalize();
        assert_eq!(vertical_param(Vec2::new(30.0, 30.0), eye, away), None);
    }
}
