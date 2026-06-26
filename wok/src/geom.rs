//! Pure spatial helpers for the editor's transform grammar, kept across the interaction demolition
//! (designs/movement-camera-design.md, "What survives, what is thrown out"). The held-key gizmo that
//! drove these is gone; the rebuilt grammar (brief 2: drag-and-drop move, the keyboard rotate)
//! composes the same primitives, so they survive here unchanged: grid snap, resting an instance's
//! bottom on a surface, the gimbal-free world-axis rotate step, and the ray-vs-ground-plane hit. Pure
//! math - no egui, no input, no camera - so each is unit tested below with no window.
//!
//! Parked, not dead: nothing calls these until brief 2 wires the new grammar, the same
//! kept-for-the-rebuild treatment `crate::camera`'s framing math carries. The module-wide
//! `#![allow(dead_code)]` says so in one place; brief 2 narrows it as the callers land.
#![allow(dead_code)]

use glam::{Quat, Vec3};
use wok_scene::Transform;

/// Snap `v` to the nearest multiple of `step`; a non-positive `step` passes through (no snap). Generic
/// over the step so the caller picks the grid (the 1m world grid is the canon default).
pub fn snap(v: f32, step: f32) -> f32 {
    if step <= 0.0 { v } else { (v / step).round() * step }
}

/// The translation Y that rests an item's BOTTOM at world height `floor`: `floor` plus the item's
/// pivot-to-bottom offset (`base_y - aabb_min_y`, the height of the origin above the item's lowest point
/// at the live rotation and scale). A centre-pivoted prefab would otherwise sink half-in; this lifts it
/// so the lowest point sits at `floor`. The offset cancels `base_y`, so it is invariant under where the
/// item currently sits and depends only on the item's shape. A drop snaps the resulting pivot to the
/// grid (like X/Z), leaving the base within +/-0.5m of the surface (penetration is allowed).
pub fn rest_y(floor: f32, base_y: f32, aabb_min_y: f32) -> f32 {
    floor + (base_y - aabb_min_y)
}

/// Spin `base` by `degrees` about the world `axis`, pre-multiplied so the spin is about the world axis
/// regardless of the placement's current heading. A relative quaternion compose, so it is
/// gimbal-lock-free - successive steps keep turning past 90deg rather than sticking. The non-rotation
/// fields pass through. Pure so the step is unit tested.
pub fn rotate_step(base: Transform, axis: Vec3, degrees: f32) -> Transform {
    let rotation = Quat::from_axis_angle(axis, degrees.to_radians()) * base.rotation;
    Transform { rotation, ..base }
}

/// The world point where the ray `origin + t*dir` meets the horizontal plane `y = height`, or `None`
/// when the ray runs parallel to the plane (no crossing) or the crossing is behind the eye (`t <= 0`).
/// The free move casts the cursor ray at the selection's height.
pub fn ray_vs_ground_plane(origin: Vec3, dir: Vec3, height: f32) -> Option<Vec3> {
    if dir.y.abs() < 1e-5 {
        return None;
    }
    let t = (height - origin.y) / dir.y;
    if t <= 0.0 {
        return None;
    }
    Some(origin + dir * t)
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-4;

    #[test]
    fn snap_rounds_to_the_nearest_multiple_and_a_nonpositive_step_passes_through() {
        // The 1m grid rounds to the nearest cell; a non-positive step is the no-snap pass-through.
        assert_eq!(snap(0.4, 1.0), 0.0);
        assert_eq!(snap(0.6, 1.0), 1.0);
        assert_eq!(snap(-1.4, 1.0), -1.0);
        assert_eq!(snap(7.0, 5.0), 5.0, "rounds to the nearest multiple of any step");
        assert_eq!(snap(3.3, 0.0), 3.3, "a non-positive step is a pass-through");
    }

    #[test]
    fn rotate_step_spins_about_the_world_axis_premultiplied_and_gimbal_free() {
        // A 5deg step spins about world X / Y / Z, positive one way and negative the other; pre-multiplied
        // so the spin is about the world axis regardless of the placement's heading (gimbal-lock-free).
        let v = Vec3::new(0.3, 0.5, 0.8);
        let yaw = rotate_step(Transform::IDENTITY, Vec3::Y, 5.0).rotation;
        assert!((yaw * v - Quat::from_rotation_y(5.0_f32.to_radians()) * v).length() < EPS, "yaws +5 about Y");
        let pitch = rotate_step(Transform::IDENTITY, Vec3::X, -5.0).rotation;
        assert!((pitch * v - Quat::from_rotation_x((-5.0_f32).to_radians()) * v).length() < EPS, "pitches -5 about X");
        // Pre-multiplied: from a turned base, the world-axis spin composes on the world (left) side.
        let base = Transform { rotation: Quat::from_rotation_y(1.0), ..Transform::IDENTITY };
        let rolled = rotate_step(base, Vec3::Z, 5.0).rotation;
        let world = Quat::from_rotation_z(5.0_f32.to_radians()) * base.rotation;
        assert!((rolled * v - world * v).length() < EPS, "roll pre-multiplies about world Z");
        // Successive steps keep turning past 90deg (gimbal-lock-free): 19 pitch steps compose to a clean
        // 95deg about world X, where the old Euler decompose would stick or bounce near 90.
        let mut t = Transform::IDENTITY;
        for _ in 0..19 {
            t = rotate_step(t, Vec3::X, 5.0);
        }
        let ninety_five = Quat::from_rotation_x(95.0_f32.to_radians());
        assert!((t.rotation * v - ninety_five * v).length() < EPS, "19 x 5deg = a clean 95deg about X, no stick");
        // The non-rotation fields pass through.
        let scaled = Transform { scale: Vec3::splat(2.0), ..Transform::IDENTITY };
        assert_eq!(rotate_step(scaled, Vec3::Y, 5.0).scale, Vec3::splat(2.0), "scale untouched");
    }

    #[test]
    fn rest_y_lifts_the_bottom_to_the_floor_and_a_snapped_pivot_is_grid_whole() {
        // rest_y places an item's bottom at `floor`: a centred 2m box (AABB min.y = base - 1) rests its
        // centre at 1.0 on flat ground (floor 0) and at 2.0 on a 1m floor. The pivot-to-bottom offset
        // cancels the current height, so the result is invariant under where the item sits now.
        assert_eq!(rest_y(0.0, 0.0, -1.0), 1.0, "on flat ground the 2m box's centre lifts to 1.0");
        assert_eq!(rest_y(1.0, 0.0, -1.0), 2.0, "a 1m floor rests it at 2.0");
        assert_eq!(rest_y(0.0, 5.0, 4.0), 1.0, "the same 1m offset, measured from a different height");
        // Snapping the resulting PIVOT to the grid keeps translation.y grid-whole even when the
        // half-height is non-integer: a unit cube (AABB min.y = base - 0.5) aimed at a surface y = 3.4 has
        // a flush pivot 3.9 that snaps to a whole 4.0 (the base then sits within +/-0.5m of the surface).
        let pivot = snap(rest_y(3.4, 0.0, -0.5), 1.0);
        assert_eq!(pivot, 4.0, "the snapped pivot is whole");
        assert_eq!(pivot.fract(), 0.0, "translation.y is grid-whole, not X.5");
    }

    #[test]
    fn ray_vs_ground_plane_hits_at_the_plane_height_under_the_cursor() {
        // A ray dropping straight down from (3, 5, 2) meets the plane y = 1 at (3, 1, 2) - the free move
        // places the selection under the cursor at its height.
        let hit = ray_vs_ground_plane(Vec3::new(3.0, 5.0, 2.0), Vec3::NEG_Y, 1.0).unwrap();
        assert!((hit - Vec3::new(3.0, 1.0, 2.0)).length() < EPS, "got {hit:?}");
    }

    #[test]
    fn ray_vs_ground_plane_is_none_when_parallel_or_behind() {
        // Parallel to the plane: no crossing. Pointing up away from a plane below the eye: the crossing is
        // behind, so None - the move holds rather than flinging the selection.
        assert_eq!(ray_vs_ground_plane(Vec3::new(0.0, 5.0, 0.0), Vec3::X, 1.0), None);
        assert_eq!(ray_vs_ground_plane(Vec3::new(0.0, 5.0, 0.0), Vec3::Y, 1.0), None);
    }
}
