//! Camera tuning: the follow camera's geometry and easing, the look-ahead framing, the occlusion
//! fade, and look input for both devices.

// ---- camera ----

/// Unobstructed boom length from the look target out to the camera, in metres. Tuned by play
/// verdicts in both directions: the original 6m read the then-1.1m character as a speck, the 5m
/// answer to that read too close once the bean grew to 1.5m and the view led ahead, so 6.5 is the
/// verdict for this body and framing, not a return to the old default.
pub const CAMERA_DISTANCE: f32 = 6.5;

/// Radius of the sphere the spring arm sweeps along the boom. It doubles as the standoff: the
/// camera rides the sphere's centre, so it stops this far in front of whatever the sweep hits.
pub const CAMERA_PROBE_RADIUS: f32 = 0.3;

/// The look target sits this far above the capsule centre, just under the bean's crown (centre is
/// height/2 over the ground at rest), so the camera frames the player rather than its waist. Must
/// stay inside the body: a target floating over the head reads as the camera staring at nothing.
pub const CAMERA_TARGET_LIFT: f32 = 0.35;

/// Minimum height the camera keeps above the terrain surface under it, in metres.
pub const CAMERA_TERRAIN_MARGIN: f32 = 0.4;

/// How far past the anchor, along the camera's horizontal forward, the look-at point sits at
/// level pitch, in metres. Looking at the anchor itself centres the player and wastes the frame's
/// lower half on ground already travelled; leading the view drops the player to low-centre and
/// spends the frame on where they are going. The live lead scales by cos(pitch)
/// (`FollowCamera::look_target`): a fixed lead under a steep downward pitch pushes the player off
/// the screen's bottom edge, so a vertical view aims back at the anchor and centres the player.
/// Eye, orbit, and arm math are untouched: this only re-aims the view.
pub const LOOK_AHEAD_M: f32 = 4.0;

/// Vertical trim on the look-at point, in metres, for fine framing on top of the lead. Zero until
/// a play-test asks otherwise.
pub const LOOK_AHEAD_LIFT_M: f32 = 0.0;

/// Half-life of the anchor's tracking smooth, in seconds: the one lag anywhere in the camera,
/// applied to the point the boom hangs from. Vertical included, so jumps and falls track instead
/// of the player drifting off-frame. Orbit angles are never smoothed; this is follow lag only.
pub const CAMERA_TRACK_SMOOTH: f32 = 0.10;

/// Half-life of the arm's recovery toward the desired boom once an obstruction clears, in seconds.
/// Obstruction clamps the arm inward instantly (a wall is a hard fact, and easing into it would
/// show the camera inside geometry); recovery is slow so the boom drifts back out rather than
/// whipping, and grazing a corner does not pump the camera in and out.
pub const CAMERA_ARM_RECOVER: f32 = 0.40;

/// Opacity an occluding prefab fades toward while it crosses the eye-to-anchor segment: low enough
/// that the player reads through it, high enough that the prefab still reads as present rather
/// than vanishing (the fade is presentation; the collider underneath is unchanged).
pub const OPACITY_OCCLUDED: f32 = 0.35;

/// Time, in seconds, for a full fade from opaque to `OPACITY_OCCLUDED` (and the same rate back).
/// Around 100ms: fast enough that the player is never hidden for a readable moment, slow enough
/// that an item grazing the segment for one frame does not strobe.
pub const OCCLUSION_FADE_S: f32 = 0.1;

/// Mouse-look sensitivity, radians of orbit per pixel of raw motion. Mouse only: the stick is a
/// rate device with its own STICK_LOOK_RATE. Raised 1.8x from the first playable's 0.0035 on the
/// mouse verdict: turning around took too much desk.
pub const MOUSE_LOOK_SENSITIVITY: f32 = 0.0063;

/// Look inversion toggles, one pair per device: the play-test verdicts came back different for
/// the mouse and the stick, so the inversion is policy per device, not shared. The base mapping
/// (all false) turns the view with the motion: rightward input turns the view right (the boom
/// swings the opposite way around the player), and pushing forward raises the camera to look down.
///
/// Mouse verdict, after a second pass: vertical flipped (dragging down raises the camera),
/// horizontal base (dragging right turns the view right - the full both-axis flip overcorrected).
/// Stick verdict, after its own second pass: vertical flipped too (pushing forward lowers the
/// camera), horizontal base. The two devices landed on the same shape, but by separate verdicts;
/// the pairs stay per device so the next pass can move one without dragging the other.
pub const MOUSE_INVERT_X: bool = false;
pub const MOUSE_INVERT_Y: bool = true;
pub const STICK_INVERT_X: bool = false;
pub const STICK_INVERT_Y: bool = true;

// ---- gamepad ----

/// Radial stick deadzone, as a fraction of full deflection. Below it a stick reads zero (resting
/// sticks drift); past it the magnitude rescales from zero so analog control stays continuous
/// rather than jumping to the deadzone's edge value.
pub const STICK_DEADZONE: f32 = 0.15;

/// Orbit turn rate at full right-stick deflection, radians per second. A stick is a rate device
/// (deflection held over time), unlike the mouse (a displacement device), so it gets its own
/// sensitivity in rate units and is integrated by the frame dt.
pub const STICK_LOOK_RATE: f32 = 2.5;

// ---- orbit limits and projection ----

/// Orbit pitch limits, radians. Positive pitch raises the camera (wok-physics boom convention);
/// the floor allows a slight under-shoulder look and the ceiling stops short of straight overhead.
pub const PITCH_MIN: f32 = -0.20;
pub const PITCH_MAX: f32 = 1.35;

/// Starting orbit pitch: a little above the shoulder, looking gently down.
pub const PITCH_DEFAULT: f32 = 0.35;

/// Vertical field of view and near plane for the projection. The far plane is per-frame data (fog
/// distance sets render distance, per the HLD), so it is a parameter, not a constant.
pub const FOV_Y_RADIANS: f32 = std::f32::consts::FRAC_PI_3;
pub const NEAR_PLANE: f32 = 0.1;

#[cfg(test)]
// Asserting on constants is the point here, exactly as in the movement domain's tests.
#[allow(clippy::assertions_on_constants, clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn the_boom_outreaches_its_probe_and_the_easing_is_live() {
        assert!(CAMERA_DISTANCE > CAMERA_PROBE_RADIUS, "a boom shorter than its probe never extends");
        assert!(CAMERA_PROBE_RADIUS > 0.0);
        assert!(CAMERA_TRACK_SMOOTH > 0.0 && CAMERA_ARM_RECOVER > 0.0);
        // Tracking must settle faster than the arm recovers: the player is framed again while the
        // boom is still drifting back out, never the other way around.
        assert!(CAMERA_TRACK_SMOOTH < CAMERA_ARM_RECOVER);
    }

    #[test]
    fn the_occlusion_fade_is_a_real_partial_fade() {
        // The fade must land strictly between invisible and opaque: 0 would make occluders vanish
        // (and cutout discard everything), 1 would make the fade a no-op. The rate must be live.
        assert!(OPACITY_OCCLUDED > 0.0 && OPACITY_OCCLUDED < 1.0);
        assert!(OCCLUSION_FADE_S > 0.0);
    }

    #[test]
    fn the_pitch_range_contains_its_default() {
        assert!(PITCH_MIN < PITCH_MAX);
        assert!((PITCH_MIN..=PITCH_MAX).contains(&PITCH_DEFAULT));
    }

    #[test]
    fn the_stick_deadzone_leaves_a_live_range() {
        // A deadzone of 1.0 or more silences the stick entirely; the rescale divides by its
        // complement, so it must also stay strictly below 1. Both look rates must be live (the
        // mouse's sensitivity lives here with the stick's: both are look-input liveness).
        assert!((0.0..1.0).contains(&STICK_DEADZONE));
        assert!(STICK_LOOK_RATE > 0.0);
        assert!(MOUSE_LOOK_SENSITIVITY > 0.0);
    }
}
