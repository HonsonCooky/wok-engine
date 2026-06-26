//! The editor's Euler-angle decomposition for rotations: a placement's quaternion read out as per-axis
//! `[X, Y, Z]` degrees for the inspector's read-only Rot row.
//!
//! The convention is `YXZ` (lifted from the prior editor's inspector): `Quat::to_euler(YXZ)` yields
//! `(yaw, pitch, roll)` - rotation about (Y, X, Z) - which [`euler_xyz_degrees`] reorders to the
//! `[X, Y, Z]` triplet the UI shows and converts to degrees. Rotation is authored by the gizmo's
//! W / E / R taps, which spin the quaternion about a world axis (`crate::gizmo`), not edited through
//! these degrees - so this is a one-way readout: a single-axis spin reads a clean multiple of the step,
//! while a compound quaternion has no clean per-axis Euler (and at gimbal lock, pitch = +/-90deg, the
//! YXZ split is singular and Y/Z fold together), shown as honest orientation feedback rather than a
//! round value. Pure, so the axis mapping is unit tested.

use glam::{EulerRot, Quat};

/// A rotation as `[X, Y, Z]` Euler degrees in the editor's `YXZ` order: `to_euler(YXZ)` gives
/// `(yaw, pitch, roll)` = rotation about (Y, X, Z), reordered here to the X/Y/Z the UI shows and
/// converted to degrees. Pure, so the axis mapping is unit tested.
pub(crate) fn euler_xyz_degrees(rotation: Quat) -> [f32; 3] {
    let (yaw, pitch, roll) = rotation.to_euler(EulerRot::YXZ);
    [pitch.to_degrees(), yaw.to_degrees(), roll.to_degrees()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(actual: [f32; 3], expected: [f32; 3]) {
        for (a, e) in actual.iter().zip(expected.iter()) {
            assert!((a - e).abs() < 1e-3, "got {actual:?}, expected {expected:?}");
        }
    }

    #[test]
    fn euler_xyz_degrees_maps_each_axis_to_its_field() {
        // A pure rotation about one axis lands wholly in that axis's field and nowhere else, which pins
        // the YXZ -> X/Y/Z reordering: X reads pitch, Y reads yaw, Z reads roll.
        approx(euler_xyz_degrees(Quat::from_rotation_x(30.0_f32.to_radians())), [30.0, 0.0, 0.0]);
        approx(euler_xyz_degrees(Quat::from_rotation_y(45.0_f32.to_radians())), [0.0, 45.0, 0.0]);
        approx(euler_xyz_degrees(Quat::from_rotation_z(60.0_f32.to_radians())), [0.0, 0.0, 60.0]);
    }

    #[test]
    fn euler_xyz_degrees_of_identity_is_zero() {
        approx(euler_xyz_degrees(Quat::IDENTITY), [0.0, 0.0, 0.0]);
    }
}
