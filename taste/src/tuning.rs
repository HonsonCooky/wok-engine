//! Live feel tuning: the handful of movement and camera numbers a play-test verdict moves, lifted
//! out of compiled constants into a hot-reloadable file so the human iterates feel without rebuilds.
//!
//! [`Tuning`] holds the simple movement model's knobs - one run speed, one gravity, one jump
//! velocity, and the jump count - plus the follow camera's feel and the look sensitivities. It does
//! NOT hold the body
//! dimensions, the simulation rate, the debug toggles, the inversion flags, or the stick deadzone:
//! changing the player's size or the step rate mid-play is a different game, not a different feel, so
//! those stay compiled constants in `crate::constants`. The split is the contract: `Tuning` is what
//! a designer edits live; the constants are what the build decides.
//!
//! **Defaults are the shipped truth; the file is the experiment surface.** `Tuning::default()` is
//! today's shipped feel, value for value. The tracked `taste/tuning.json` is loaded at startup and
//! may diverge from the defaults as the human experiments, so the tests pin that the defaults
//! validate clean (the shipped truth must be sane) and, separately, that the tracked file parses and
//! validates (the experiment must not be broken) - never that the two are equal.
//!
//! **Validation warns, it never panics.** [`Tuning::validate`] reports nonsensical values (a
//! non-positive run speed, a jump that cannot leave the ground, a silenced look device) as warnings;
//! play continues on the values as given. A parse failure on load or reload keeps the previous
//! values and says so. Nothing here can crash a play session.
//!
//! **Determinism.** Live tuning is a dev/authoring hook, like content hot reload: it feeds the
//! authored -> runtime numbers and is deliberately OUTSIDE the determinism contract. Every sim test
//! and the replay harness construct `Tuning::default()`, so the deterministic gameplay they pin is
//! unaffected by a file that changes under a play session.

use std::error::Error;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::constants::CAMERA_PROBE_RADIUS;

/// The hot-reloadable feel record. Serde round-trips it to `taste/tuning.json`; `#[serde(default)]`
/// fills any field a hand-edited file omits from [`Tuning::default`], so a partial file is valid
/// and changes only the fields it names.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tuning {
    // ---- movement ----
    /// Horizontal run speed in m/s. This is the whole of horizontal locomotion: the player's
    /// horizontal velocity is the move intent (length at most one) times this, applied identically
    /// on the ground and in the air - no separate acceleration, friction, or air-control knob to
    /// keep in sync. State multipliers (ice, mud, a sprint) will scale this one number.
    pub move_speed: f32,

    /// Downward acceleration in m/s^2: the one gravity for the whole arc, with no rise-versus-fall
    /// split. A jump of `jump_velocity` peaks at jump_velocity^2 / (2 * gravity) metres,
    /// jump_velocity / gravity seconds after launch.
    pub gravity: f32,

    /// Upward velocity a jump grants, in m/s. Every jump - the first off the ground and every air
    /// jump alike - sets the vertical velocity to exactly this, so a second jump acts like the
    /// first.
    pub jump_velocity: f32,

    /// How many jumps are available before the body must refill them. Two is the jump plus the
    /// double jump.
    pub max_jumps: u32,

    // ---- camera feel ----
    /// Unobstructed boom length from the look target out to the camera, in metres. Tuned by play
    /// verdicts in both directions: the original 6m read the then-1.1m character as a speck, the 5m
    /// answer to that read too close once the bean grew to 1.5m and the view led ahead, so 6.5 is
    /// the verdict for this body and framing, not a return to the old default.
    pub camera_distance: f32,

    /// Half-life of the anchor's tracking smooth, in seconds: the one lag anywhere in the camera,
    /// applied to the point the boom hangs from. Vertical included, so jumps and falls track instead
    /// of the player drifting off-frame. Orbit angles are never smoothed; this is follow lag only.
    pub camera_track_smooth: f32,

    /// Half-life of the arm's recovery toward the desired boom once an obstruction clears, in
    /// seconds. Obstruction clamps the arm inward instantly (a wall is a hard fact, and easing into
    /// it would show the camera inside geometry); recovery is slow so the boom drifts back out
    /// rather than whipping, and grazing a corner does not pump the camera in and out.
    pub camera_arm_recover: f32,

    /// How far past the anchor, along the camera's horizontal forward, the look-at point sits at
    /// level pitch, in metres. Looking at the anchor itself centres the player and wastes the
    /// frame's lower half on ground already travelled; leading the view drops the player to
    /// low-centre and spends the frame on where they are going. The live lead scales by cos(pitch)
    /// (`FollowCamera::look_target`): a fixed lead under a steep downward pitch pushes the player off
    /// the screen's bottom edge, so a vertical view aims back at the anchor and centres the player.
    pub look_ahead_m: f32,

    /// Vertical trim on the look-at point, in metres, for fine framing on top of the lead. Zero
    /// until a play-test asks otherwise.
    pub look_ahead_lift_m: f32,

    /// Mouse-look sensitivity, radians of orbit per pixel of raw motion. Mouse only: the stick is a
    /// rate device with its own stick_look_rate. Raised 1.8x from the first playable's 0.0035 on the
    /// mouse verdict: turning around took too much desk.
    pub mouse_look_sensitivity: f32,

    /// Orbit turn rate at full right-stick deflection, radians per second. A stick is a rate device
    /// (deflection held over time), unlike the mouse (a displacement device), so it gets its own
    /// sensitivity in rate units and is integrated by the frame dt.
    pub stick_look_rate: f32,
}

impl Default for Tuning {
    /// Today's shipped feel, value for value. This is the truth the tests validate clean; the
    /// tracked `tuning.json` is free to diverge from it.
    fn default() -> Self {
        Tuning {
            move_speed: 7.5,
            gravity: 30.0,
            jump_velocity: 11.0,
            max_jumps: 2,
            camera_distance: 6.5,
            camera_track_smooth: 0.10,
            camera_arm_recover: 0.40,
            look_ahead_m: 4.0,
            look_ahead_lift_m: 0.0,
            mouse_look_sensitivity: 0.0063,
            stick_look_rate: 2.5,
        }
    }
}

impl Tuning {
    /// Report nonsensical feel values as warnings, never panicking: play continues on the values as
    /// given. A retune that breaks an assumption says so on load (and on reload) instead of failing
    /// only in `cargo test`. An empty result means every value is sane.
    pub fn validate(&self) -> Vec<String> {
        let mut w = Vec::new();

        // Movement: a real run speed, a real downward gravity.
        if self.move_speed <= 0.0 {
            w.push(format!("move_speed {} is not a real run speed", self.move_speed));
        }
        if self.gravity <= 0.0 {
            w.push(format!("gravity {} must pull the body down (> 0)", self.gravity));
        }

        // The jump: a positive launch that clears something worth leaving the ground for.
        if self.jump_velocity <= 0.0 {
            w.push(format!("jump_velocity {} must launch upward (> 0)", self.jump_velocity));
        }
        if self.gravity > 0.0 && self.jump_velocity > 0.0 {
            let apex = self.jump_velocity * self.jump_velocity / (2.0 * self.gravity);
            if apex < 0.5 {
                w.push(format!(
                    "jump apex {apex:.3}m barely leaves the ground: raise jump_velocity or lower gravity"
                ));
            }
        }
        if self.max_jumps == 0 {
            w.push("max_jumps is 0: the player cannot jump at all".into());
        }

        // The camera: a boom that outreaches its probe, live easing, tracking that settles faster
        // than the arm recovers, and live look rates on both devices.
        if self.camera_distance <= CAMERA_PROBE_RADIUS {
            w.push(format!(
                "camera_distance {} is no longer than the probe radius {CAMERA_PROBE_RADIUS}: the boom never extends",
                self.camera_distance
            ));
        }
        if self.camera_track_smooth <= 0.0 || self.camera_arm_recover <= 0.0 {
            w.push("camera_track_smooth and camera_arm_recover must both be live (> 0)".into());
        } else if self.camera_track_smooth >= self.camera_arm_recover {
            w.push(format!(
                "camera_track_smooth {} should settle faster than camera_arm_recover {}",
                self.camera_track_smooth, self.camera_arm_recover
            ));
        }
        if self.mouse_look_sensitivity <= 0.0 {
            w.push(format!("mouse_look_sensitivity {} silences the mouse", self.mouse_look_sensitivity));
        }
        if self.stick_look_rate <= 0.0 {
            w.push(format!("stick_look_rate {} silences the stick", self.stick_look_rate));
        }

        w
    }
}

// ---- load and save ----

/// Read and parse a tuning file. The error is `Box<dyn Error>` per the wok precedent: the only
/// thing the caller does with it is decide between "use these values" and "say so and keep the
/// previous ones", never distinguish a missing file from malformed JSON programmatically.
pub fn load(path: impl AsRef<Path>) -> Result<Tuning, Box<dyn Error>> {
    let text = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&text)?)
}

/// Write a tuning out as pretty JSON (a trailing newline, so the tracked file ends clean). Used to
/// lay down the defaults the first time the file is missing.
pub fn save(path: impl AsRef<Path>, tuning: &Tuning) -> Result<(), Box<dyn Error>> {
    let mut text = serde_json::to_string_pretty(tuning)?;
    text.push('\n');
    std::fs::write(path, text)?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_validate_clean() {
        // The shipped truth must be sane on its own terms: every check the validator makes holds for
        // the defaults, so a warning on load always means the FILE diverged, never that the shipped
        // feel was broken.
        let warnings = Tuning::default().validate();
        assert!(warnings.is_empty(), "the shipped defaults must validate clean, got: {warnings:?}");
    }

    #[test]
    fn the_tracked_tuning_file_parses_and_validates() {
        // The experiment surface must not be broken. This reads the tracked taste/tuning.json (via
        // the manifest dir, so it is found regardless of the test's working directory) and pins that
        // it parses and validates - deliberately NOT that it equals the defaults: the file is free
        // to diverge as the human tunes feel, the defaults are the shipped baseline.
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tuning.json");
        let tuning = load(&path).expect("the tracked tuning.json should parse");
        let warnings = tuning.validate();
        assert!(warnings.is_empty(), "the tracked tuning.json should validate clean, got: {warnings:?}");
    }

    #[test]
    fn validate_catches_a_dead_run_speed() {
        let t = Tuning { move_speed: 0.0, ..Tuning::default() };
        assert!(t.validate().iter().any(|m| m.contains("move_speed")), "should warn on a zero run speed");
    }

    #[test]
    fn validate_catches_a_non_falling_gravity() {
        let t = Tuning { gravity: 0.0, ..Tuning::default() };
        assert!(t.validate().iter().any(|m| m.contains("gravity")), "should warn on a gravity that does not pull down");
    }

    #[test]
    fn validate_catches_a_jump_that_barely_leaves_the_ground() {
        // A launch too weak for the gravity peaks below half a metre: jumping buys nothing.
        let t = Tuning { jump_velocity: 1.0, gravity: 30.0, ..Tuning::default() };
        assert!(t.validate().iter().any(|m| m.contains("jump apex")), "should warn on a jump that clears nothing");
    }

    #[test]
    fn validate_catches_no_jumps_at_all() {
        let t = Tuning { max_jumps: 0, ..Tuning::default() };
        assert!(t.validate().iter().any(|m| m.contains("max_jumps")), "should warn when the player cannot jump");
    }

    #[test]
    fn validate_catches_a_camera_boom_shorter_than_its_probe() {
        let t = Tuning { camera_distance: 0.1, ..Tuning::default() };
        assert!(t.validate().iter().any(|m| m.contains("camera_distance")), "should warn on a boom inside its probe");
    }

    #[test]
    fn validate_catches_tracking_slower_than_arm_recovery() {
        // The player must be framed again while the boom is still drifting out, never the reverse.
        let t = Tuning { camera_track_smooth: 0.6, camera_arm_recover: 0.4, ..Tuning::default() };
        assert!(
            t.validate().iter().any(|m| m.contains("camera_track_smooth")),
            "should warn when tracking lags the arm recovery"
        );
    }

    #[test]
    fn validate_catches_a_silenced_look_device() {
        let mouse = Tuning { mouse_look_sensitivity: 0.0, ..Tuning::default() };
        assert!(mouse.validate().iter().any(|m| m.contains("mouse_look_sensitivity")), "should warn on a dead mouse");
        let stick = Tuning { stick_look_rate: 0.0, ..Tuning::default() };
        assert!(stick.validate().iter().any(|m| m.contains("stick_look_rate")), "should warn on a dead stick");
    }

    #[test]
    fn a_partial_file_fills_missing_fields_from_defaults() {
        // #[serde(default)]: a hand-edited file that names only one field is valid and changes only
        // that field, so the human can drop a single number in without restating the whole record.
        let t: Tuning = serde_json::from_str(r#"{ "move_speed": 9.0 }"#).expect("a partial file should parse");
        assert_eq!(t.move_speed, 9.0, "the named field takes the file's value");
        assert_eq!(t.gravity, Tuning::default().gravity, "an omitted field falls back to the default");
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = std::env::temp_dir().join(format!("taste-tuning-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tuning.json");
        let original = Tuning::default();
        save(&path, &original).expect("save should write");
        let loaded = load(&path).expect("load should read back");
        assert_eq!(loaded, original, "a saved tuning must load back identical");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
