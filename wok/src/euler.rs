//! The editor's Euler-angle convention for rotations, shared by the inspector's Rot fields and the
//! gizmo's W / E / R rotate taps so both decompose and recompose a quaternion the same way.
//!
//! A placement stores its rotation as a [`Quat`]; the editor edits it as per-axis degrees. The
//! convention is `YXZ` (lifted from the prior editor's inspector): `Quat::to_euler(YXZ)` yields
//! `(yaw, pitch, roll)` - rotation about (Y, X, Z) - which [`euler_xyz_degrees`] reorders to the
//! `[X, Y, Z]` triplet the UI shows, and [`quat_from_euler_xyz_degrees`] feeds back exactly inverted.
//! Sharing one decompose/recompose keeps a rotate tap editing the same numbers the inspector displays,
//! so a tap nudges one dial and the readout stays clean multiples of the step - rather than smearing
//! across all three axes, which composing world-axis quaternions and reading the canonical Euler back
//! would do once more than one axis is involved.
//!
//! A quaternion has many Euler decompositions; recomposing then re-decomposing only agrees within the
//! principal range (pitch in `(-90, 90)`). At gimbal lock (pitch = +/-90deg) the YXZ split is singular
//! and Y/Z can flip - the inspector and the taps share this limitation.

use glam::{EulerRot, Quat};

/// A rotation as `[X, Y, Z]` Euler degrees in the editor's `YXZ` order: `to_euler(YXZ)` gives
/// `(yaw, pitch, roll)` = rotation about (Y, X, Z), reordered here to the X/Y/Z the UI shows and
/// converted to degrees. Pure, so the axis mapping is unit tested.
pub(crate) fn euler_xyz_degrees(rotation: Quat) -> [f32; 3] {
    let (yaw, pitch, roll) = rotation.to_euler(EulerRot::YXZ);
    [pitch.to_degrees(), yaw.to_degrees(), roll.to_degrees()]
}

/// The inverse of [`euler_xyz_degrees`]: rebuild a quaternion from `[X, Y, Z]` Euler degrees in the
/// editor's `YXZ` order. The triplet is `[pitch, yaw, roll]` (about X, Y, Z), fed back as
/// `from_euler(YXZ, yaw, pitch, roll)` in radians - exactly undoing the display decomposition. Pure, so
/// the round trip is unit tested.
pub(crate) fn quat_from_euler_xyz_degrees(xyz: [f32; 3]) -> Quat {
    Quat::from_euler(EulerRot::YXZ, xyz[1].to_radians(), xyz[0].to_radians(), xyz[2].to_radians())
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

    #[test]
    fn quat_from_euler_degrees_inverts_the_display_decomposition() {
        // Recomposing the held degrees must be the exact inverse of the Quat -> Euler display, or even a
        // no-op edit would drift the rotation. Holds for each single axis and a combined rotation,
        // within the principal YXZ range (pitch in (-90, 90)).
        for e in [[0.0, 0.0, 0.0], [30.0, 0.0, 0.0], [0.0, 45.0, 0.0], [0.0, 0.0, 60.0], [10.0, 20.0, 30.0]] {
            approx(euler_xyz_degrees(quat_from_euler_xyz_degrees(e)), e);
        }
    }

    #[test]
    fn editing_one_euler_axis_leaves_the_others_intact() {
        // Changing one axis, recomposing to a quaternion, then re-decomposing returns the other two axes
        // unscrambled - what lets a rotate tap (and the inspector's scratch) keep an edit local to its
        // axis. Each axis in turn is set to a fresh value over a non-trivial starting rotation.
        let start = [10.0, 20.0, 30.0];
        for axis in 0..3 {
            let mut edited = start;
            edited[axis] = 55.0;
            approx(euler_xyz_degrees(quat_from_euler_xyz_degrees(edited)), edited);
        }
    }
}
