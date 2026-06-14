//! Camera constants the build decides: the spring-arm probe, the arm clamp-in speed, the framing
//! offsets, the occlusion fade, the look-input device policy (inversion and deadzone), and the
//! orbit/projection limits.
//!
//! The camera's FEEL - the boom length, the tracking and arm half-lives, the look-ahead lead, and
//! the per-device look sensitivities - moved to `crate::tuning`, the hot-reloadable feel record,
//! because those are what a framing verdict retunes live. What stays here is the rest of the camera
//! the build fixes: the probe radius and terrain margin (collision geometry), the occlusion fade
//! (presentation policy), the inversion pairs and stick deadzone (device policy, not feel - flipping
//! an axis or resizing the dead region is a controller decision, not a tuning slider), and the
//! orbit pitch range and projection. The liveness sanity these values used to assert against the
//! moved ones now lives in `Tuning::validate`; the tests kept here pin only the constants that
//! remain.

// ---- camera ----

/// Radius of the sphere the spring arm sweeps along the boom. It doubles as the standoff: the
/// camera rides the sphere's centre, so it stops this far in front of whatever the sweep hits.
pub const CAMERA_PROBE_RADIUS: f32 = 0.3;

/// The look target sits this far above the capsule centre, just under the bean's crown (centre is
/// height/2 over the ground at rest), so the camera frames the player rather than its waist. Must
/// stay inside the body: a target floating over the head reads as the camera staring at nothing.
pub const CAMERA_TARGET_LIFT: f32 = 0.35;

/// Minimum height the camera keeps above the terrain surface under it, in metres.
pub const CAMERA_TERRAIN_MARGIN: f32 = 0.4;

/// Half-life of the arm pulling IN toward an obstruction's clearance, in seconds: short, so backing
/// the camera into a prefab reads as a swift move rather than a hard snap, yet still eased over a
/// few frames (the occlusion fade covers the brief overlap while the arm closes). Recovery back OUT
/// uses the slow `Tuning::camera_arm_recover` instead - an obstruction is a fact to clear quickly,
/// the boom drifting back out is the gentle part. Build-fixed: it shapes how the clamp resolves, not
/// a framing verdict, so it stays a constant.
pub const CAMERA_ARM_CLAMP: f32 = 0.08;

/// Opacity an occluding prefab fades toward while it crosses the eye-to-anchor segment: low enough
/// that the player reads through it, high enough that the prefab still reads as present rather
/// than vanishing (the fade is presentation; the collider underneath is unchanged).
pub const OPACITY_OCCLUDED: f32 = 0.35;

/// Time, in seconds, for a full fade from opaque to `OPACITY_OCCLUDED` (and the same rate back).
/// Around 100ms: fast enough that the player is never hidden for a readable moment, slow enough
/// that an item grazing the segment for one frame does not strobe.
pub const OCCLUSION_FADE_S: f32 = 0.1;

/// Look inversion toggles, one pair per device: the play-test verdicts came back different for
/// the mouse and the stick, so the inversion is policy per device, not shared. The base mapping
/// (all false) turns the view with the motion: rightward input turns the view right (the boom
/// swings the opposite way around the player), and pushing forward raises the camera to look down.
/// Device policy, not feel tuning - a flip is a controller decision, so it stays a constant.
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
/// rather than jumping to the deadzone's edge value. Device policy, not feel: it shapes the dead
/// region a controller needs, so it stays a constant alongside the inversion pairs.
pub const STICK_DEADZONE: f32 = 0.15;

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
// Asserting on constants is the point here, exactly as in the movement domain's tests. (Liveness
// relationships involving the moved feel values - the boom outreaching its probe, the look rates -
// are pinned by `Tuning::validate` instead.)
#[allow(clippy::assertions_on_constants, clippy::float_cmp)]
mod tests {
    use super::*;

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
        // complement, so it must also stay strictly below 1. (The stick and mouse look rates that
        // used to be pinned alongside it here are feel tuning now, checked by `Tuning::validate`.)
        assert!((0.0..1.0).contains(&STICK_DEADZONE));
    }

    #[test]
    fn the_probe_radius_is_a_real_standoff() {
        // The probe must be a real sphere for the spring arm to ride; that the boom outreaches it
        // is a feel relationship pinned by `Tuning::validate` now.
        assert!(CAMERA_PROBE_RADIUS > 0.0);
        assert!(CAMERA_TERRAIN_MARGIN > 0.0);
    }

    #[test]
    fn the_arm_clamp_in_is_a_real_half_life() {
        // A live, positive half-life so the clamp-in eases over a few frames rather than snapping
        // (zero) or never resolving.
        assert!(CAMERA_ARM_CLAMP > 0.0);
    }
}
