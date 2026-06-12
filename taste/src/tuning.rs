//! Live feel tuning: the gameplay and camera numbers a play-test verdict actually talks about,
//! lifted out of the compiled constants into a hot-reloadable file so the human iterates feel
//! without rebuilds.
//!
//! [`Tuning`] holds exactly the numbers a retune verdict moves: locomotion, the parameterized
//! jump, air steering, the wall policies, the walkable ground limit, the downhill glue, and the
//! follow camera's feel. It does NOT hold the body, the clock, the debug toggles, the inversion
//! flags, or the stick deadzone - changing the player's dimensions or the simulation rate mid-play
//! is not feel tuning, it is a different game, so those stay compiled constants in
//! `crate::constants`. The split is the contract: `Tuning` is what a designer edits live; the
//! constants are what the build decides.
//!
//! **Defaults are the shipped truth; the file is the experiment surface.** `Tuning::default()` is
//! today's shipped feel, value for value, with every constant's doc comment carried over so the
//! rationale travels with the number. The tracked `taste/tuning.json` is the authored feel record:
//! it is loaded at startup and may diverge from the defaults as the human experiments, so the
//! tests pin that the defaults validate clean (the shipped truth must be sane) and, separately,
//! that the tracked file parses and validates (the experiment must not be broken) - never that the
//! two are equal.
//!
//! **Derived values are methods, not fields.** The jump is parameterized the way it is judged
//! (apex height and time to apex); the ascent gravity, launch velocity, and fall gravity follow by
//! the kinematics ([`Tuning::ascent_gravity`], [`Tuning::jump_velocity`], [`Tuning::fall_gravity`]).
//! The walkable ground limit is one angle in degrees; the walkable cosine and the slide's
//! flat-resolve normal threshold both derive from it ([`Tuning::walkable_cos`],
//! [`Tuning::walkable_normal_y`]) so the pairing the slide and the terrain rest depend on cannot be
//! detuned apart.
//!
//! **Validation warns, it never panics.** [`Tuning::validate`] reports broken relationships
//! (friction weaker than acceleration, a snap-down too short for the steepest walkable step, a stop
//! distance outside the precision band, and so on) as warnings; play continues on the values as
//! given. A parse failure on load or reload keeps the previous values and says so. Nothing here can
//! crash a play session.
//!
//! **Determinism.** Live tuning is a dev/authoring hook, like content hot reload: it feeds the
//! authored -> runtime numbers and is deliberately OUTSIDE the determinism contract. Every sim test
//! and the replay harness construct `Tuning::default()`, so the deterministic gameplay they pin is
//! unaffected by a file that changes under a play session.

use std::error::Error;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::constants::{CAMERA_PROBE_RADIUS, PLAYER_HEIGHT, SIM_DT, STEP_HEIGHT};

/// How far below the walkable cosine the slide's flat-resolve normal threshold sits: a hair, so
/// every contact the grounded signal accepts also qualifies for the supported flat resolve, while
/// wall-grade contacts keep their true normals. Derived from the walkable limit rather than tuned
/// (see [`Tuning::walkable_normal_y`]), so the pairing the edge-drift fix depends on cannot drift.
const WALKABLE_NORMAL_MARGIN: f32 = 0.01;

/// The hot-reloadable feel record. Serde round-trips it to `taste/tuning.json`; `#[serde(default)]`
/// fills any field a hand-edited file omits from [`Tuning::default`], so a partial file is valid
/// and changes only the fields it names.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tuning {
    // ---- movement ----
    /// Horizontal locomotion speed in m/s, a brisk run. Raised from 6.0 on the feel-pass verdict:
    /// crossing the demo's spaces at 6 read as too slow.
    pub move_speed: f32,

    /// Horizontal acceleration toward the intended velocity, in m/s^2, grounded with input. From
    /// rest to top speed in move_speed / ground_accel seconds (0.083s, five fixed steps): the
    /// precision-kit verdict against the old 40 (0.19s), which read slidey under keyboard taps - a
    /// digital key asks for full speed now, and a long ramp-up smears every small correction.
    pub ground_accel: f32,

    /// Horizontal deceleration toward rest, in m/s^2, grounded with no input. Stronger than the
    /// acceleration so stopping reads planted: top speed to rest in 0.05s, sliding
    /// move_speed^2 / (2 * ground_friction) metres (~0.19m; the old 50 slid ~0.56m, most of a tile,
    /// which is what made precision landings overshoot).
    pub ground_friction: f32,

    // ---- the parameterized jump ----
    //
    // The jump is parameterized the way it is judged in play (Pittman's "tuning a jump" model): a
    // designer retunes WHERE the apex is and WHEN it arrives, and the physics follow. Under constant
    // ascent gravity g, a launch velocity v rises for t = v / g seconds and peaks at h = v^2 / 2g
    // metres. Solving that pair for (g, v) given (h, t): g = 2h / t^2, v = g * t = 2h / t. The
    // derived values are methods ([`Tuning::ascent_gravity`], [`Tuning::jump_velocity`]).
    /// Apex height of a ground jump, in metres: the ~1.9m the raised-jump verdict landed on, stated
    /// as the parameter instead of a consequence. Every jump flies this full arc - height never
    /// depends on how long the control is held (the verdict that removed the jump cut).
    pub jump_apex_height: f32,

    /// Time from launch to that apex, in seconds. Shorter is snappier, longer is floatier; 0.38s is
    /// the old 25-gravity arc's rise time, the keep-the-feel starting point.
    pub jump_time_to_apex: f32,

    /// Gravity multiplier while descending. Symmetric arcs read floaty on the way down: the eye
    /// expects a jump to commit to its landing. Scaling only the descent keeps the tuned apex and
    /// rise untouched while the fall arrives sqrt(fall_gravity_mult) times sooner.
    pub fall_gravity_mult: f32,

    /// Coyote time, in seconds of simulation time: how long after walking off an edge (leaving the
    /// ground without jumping) a ground jump remains available. The player judges the jump against
    /// the drawn body, which is right at the edge when the simulation has already left it; the grace
    /// window makes the late press land the full jump instead of silently spending the air jump. A
    /// jump consumes the window immediately, so it can never stack a free extra jump.
    pub coyote_s: f32,

    /// How long a jump press stays buffered waiting for ground, in seconds of simulation time. A
    /// press up to this long before landing fires on the landing step instead of being swallowed by
    /// an airborne one (`crate::jump`). Around 100ms is the feel-polish standard: long enough that a
    /// press timed against the visible landing always lands, short enough that a stale press never
    /// fires a visibly delayed jump.
    pub jump_buffer_s: f32,

    /// Extra jumps available while airborne, restored on any grounding. One is the double jump.
    pub air_jumps: u32,

    /// The air jump's launch velocity as a fraction of the ground jump's: a touch weaker than the
    /// ground jump so the double jump reads as a recovery, not a free second full jump.
    pub air_jump_scale: f32,

    // ---- air ----
    /// How fast airborne input rotates the horizontal velocity's direction toward the stick, in
    /// radians per second - the ONLY airborne control. Air is pure momentum (policy in `crate::air`):
    /// a jump's horizontal speed is set at launch and never changes until landing, so the stick
    /// turns the heading but can never stretch or shrink the jump's reach. Redirection rather than
    /// acceleration-through-zero is the BFBB / Ratchet & Clank air authority: reversing heading
    /// mid-jump turns the moving velocity around (a half circle in ~0.52s, most of a jump's hang
    /// time) instead of braking through a dead stop. The speed-magnitude approach this rate used to
    /// pair with (air acceleration, 12 m/s^2) retired on the pure-momentum verdict; friction never
    /// applies airborne either - with no input the velocity is ballistic.
    pub air_turn_rate: f32,

    // ---- walls ----
    /// The wall stop's incidence window, in degrees from head-on. A wall-grade contact whose
    /// horizontal motion points within this angle of straight into the wall kills its tangential
    /// redirect: the player stops at the wall instead of skating along it (the play verdict: running
    /// at a wall should read as a stop, not a deflection). Beyond the window, glancing contacts
    /// slide as the engine resolves them (less the wall friction below); vertical motion (gravity
    /// along a wall) never enters the test. Narrowed from 45 on the follow-up verdict: the stop cone
    /// was too wide, and a quarter-angle approach should slide. Policy in `crate::slide`.
    pub wall_stop_deg: f32,

    /// Tangential deceleration while in wall contact, in m/s^2: the horizontal speed of a wall-grade
    /// sliding contact decays at this rate, applied only on steps that actually touch a wall, so a
    /// wall slide scrubs speed instead of feeling like glass. Vertical motion is exempt (gravity
    /// still slides a body down a wall, as in the stop), and ground or airborne non-contact motion
    /// never sees it. A full-speed slide scrubs out in move_speed / wall_friction seconds (0.3s);
    /// one step's scrub (~0.42 m/s) is a sliver, so a brief graze barely dents the run. Policy in
    /// `crate::slide`.
    pub wall_friction: f32,

    // ---- ground ----
    /// The steepest slope that still counts as walkable ground, in degrees. One angle drives both
    /// the slide and the terrain rest (via [`Tuning::walkable_cos`]) so the two grounded signals
    /// agree, and the slide's flat-resolve normal threshold ([`Tuning::walkable_normal_y`]) derives
    /// from it too, so the pairing the edge-drift fix depends on cannot be detuned apart. 60 degrees
    /// on the play verdict: steep hillsides should ground, rest, and walk normally. With the
    /// flat-bottomed cylinder the limit applies uniformly - tilted collider faces stand to the same
    /// limit as terrain.
    pub walkable_degrees: f32,

    /// Ground glue, in metres: walking downhill, the surface falls away faster than one step of
    /// gravity can follow, so without glue a grounded walk flickers airborne every step. If the
    /// player was grounded, did not jump, and the support is within this distance below the foot
    /// after the move, the foot snaps to the support and stays grounded. Genuine drops (a ledge
    /// taller than this) still go airborne, and a jump always leaves the ground. Game policy, not
    /// physics: the engine's terrain rest is lift-only by design.
    pub snap_down_distance: f32,

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
    /// Today's shipped feel, value for value with the constants this file replaced. This is the
    /// truth the tests validate clean; the tracked `tuning.json` is free to diverge from it.
    fn default() -> Self {
        Tuning {
            move_speed: 7.5,
            ground_accel: 90.0,
            ground_friction: 150.0,
            jump_apex_height: 1.9,
            jump_time_to_apex: 0.38,
            fall_gravity_mult: 1.7,
            coyote_s: 0.10,
            jump_buffer_s: 0.10,
            air_jumps: 1,
            air_jump_scale: 0.9,
            air_turn_rate: 6.0,
            wall_stop_deg: 30.0,
            wall_friction: 25.0,
            walkable_degrees: 60.0,
            snap_down_distance: 0.3,
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
    // ---- derived jump kinematics ----

    /// Ascent gravity magnitude in m/s^2, derived: 2h / t^2. Applies while the player is rising.
    pub fn ascent_gravity(&self) -> f32 {
        2.0 * self.jump_apex_height / (self.jump_time_to_apex * self.jump_time_to_apex)
    }

    /// Upward velocity granted by a ground (or coyote) jump, in m/s, derived: 2h / t.
    pub fn jump_velocity(&self) -> f32 {
        2.0 * self.jump_apex_height / self.jump_time_to_apex
    }

    /// Descent gravity magnitude in m/s^2: the ascent gravity under the fall multiplier. The split
    /// is applied by `sim::gravity` on the vertical velocity's sign.
    pub fn fall_gravity(&self) -> f32 {
        self.ascent_gravity() * self.fall_gravity_mult
    }

    // ---- derived ground limit ----

    /// cos(walkable_degrees): the steepest slope that still counts as walkable ground, passed to
    /// both the slide and the terrain rest so the two grounded signals agree.
    pub fn walkable_cos(&self) -> f32 {
        self.walkable_degrees.to_radians().cos()
    }

    /// The slide's flat-resolve normal threshold: contacts at least this upright grade as ground
    /// for the supported flat resolve, so gravity dies in the contact instead of leaking sideways
    /// (the edge-drift fix). Derived a hair under [`Tuning::walkable_cos`] so every contact the
    /// grounded signal accepts also qualifies, while wall-grade contacts keep their true normals.
    /// Deriving it from the same angle is what keeps the pairing from being detuned apart.
    pub fn walkable_normal_y(&self) -> f32 {
        self.walkable_cos() - WALKABLE_NORMAL_MARGIN
    }

    // ---- validation ----

    /// Report broken feel relationships as warnings, never panicking: play continues on the values
    /// as given. This is the relationship-test logic the constants modules used to assert, restated
    /// against the live numbers - a retune that breaks an assumption now says so on load (and on
    /// reload) instead of failing only in `cargo test`. An empty result means every relationship
    /// holds.
    pub fn validate(&self) -> Vec<String> {
        let mut w = Vec::new();

        // Locomotion: positive speed, crisp ramp, friction at least as hard as acceleration.
        if self.move_speed <= 0.0 {
            w.push(format!("move_speed {} is not a real run speed", self.move_speed));
        }
        if self.ground_accel <= 0.0 || self.move_speed / self.ground_accel > 0.2 {
            w.push(format!(
                "too slow to top speed (move_speed / ground_accel = {:.3}s, want < 0.2s): reads as ice",
                self.move_speed / self.ground_accel
            ));
        }
        if self.ground_friction < self.ground_accel {
            w.push(format!(
                "ground_friction {} is weaker than ground_accel {}: releasing input reads slippery",
                self.ground_friction, self.ground_accel
            ));
        }
        // The stop after releasing input covers move^2 / 2f metres: inside a precision landing, but
        // not so abrupt it reads as hitting a wall.
        if self.ground_friction > 0.0 {
            let stop = self.move_speed * self.move_speed / (2.0 * self.ground_friction);
            if stop > 0.25 {
                w.push(format!("stop distance {stop:.3}m overshoots a precision landing (want <= 0.25m)"));
            }
            if stop < 0.05 {
                w.push(format!("stop distance {stop:.3}m is so abrupt it reads as hitting a wall (want > 0.05m)"));
            }
        }

        // The jump: positive parameters, an apex worth leaving the ground for, a committing fall.
        if self.jump_apex_height <= 0.0 || self.jump_time_to_apex <= 0.0 {
            w.push(format!(
                "jump_apex_height {} and jump_time_to_apex {} must both be positive",
                self.jump_apex_height, self.jump_time_to_apex
            ));
        }
        if self.jump_apex_height <= STEP_HEIGHT {
            w.push(format!(
                "jump_apex_height {} does not clear the {STEP_HEIGHT}m step-up: jumping buys nothing",
                self.jump_apex_height
            ));
        }
        if self.fall_gravity_mult <= 1.0 {
            w.push(format!(
                "fall_gravity_mult {} should exceed 1.0 or the descent is no heavier than the rise",
                self.fall_gravity_mult
            ));
        }

        // The forgiveness timers: each must survive quantization (a couple of fixed steps), and the
        // coyote grace must stay small against the jump's own rise.
        if self.coyote_s < 2.0 * SIM_DT {
            w.push(format!("coyote_s {} is under two fixed steps: luck, not grace", self.coyote_s));
        }
        if self.coyote_s > self.jump_time_to_apex * 0.5 {
            w.push(format!(
                "coyote_s {} exceeds half the jump's rise ({:.3}s): the grace reads as a mechanic",
                self.coyote_s,
                self.jump_time_to_apex * 0.5
            ));
        }
        if self.jump_buffer_s < 2.0 * SIM_DT {
            w.push(format!("jump_buffer_s {} is under two fixed steps: it barely catches a press", self.jump_buffer_s));
        }

        // The double jump exists and is a recovery, not a stronger second jump.
        if self.air_jumps == 0 {
            w.push("air_jumps is 0: the double jump is gone".into());
        }
        if self.air_jump_scale <= 0.0 || self.air_jump_scale > 1.0 {
            let s = self.air_jump_scale;
            w.push(format!("air_jump_scale {s} should be in (0, 1]: a recovery, not a stronger jump"));
        }

        // Air steering: a full heading reversal (the worst redirect) must finish inside a jump's
        // hang time, or the do-over the air model sells cannot complete before landing.
        if self.air_turn_rate <= 0.0 {
            w.push(format!("air_turn_rate {} must be positive or the air has no control at all", self.air_turn_rate));
        } else if self.jump_apex_height > 0.0 && self.jump_time_to_apex > 0.0 {
            let hang = self.jump_time_to_apex + (2.0 * self.jump_apex_height / self.fall_gravity()).sqrt();
            let reversal = std::f32::consts::PI / self.air_turn_rate;
            if reversal >= hang {
                w.push(format!("an air reversal ({reversal:.3}s) cannot finish inside the {hang:.3}s hang time"));
            }
        }

        // The walkable limit is a real slope between flat and vertical.
        if self.walkable_degrees <= 0.0 || self.walkable_degrees >= 90.0 {
            w.push(format!("walkable_degrees {} must be strictly between flat and vertical", self.walkable_degrees));
        }

        // The wall stop is a real window, and the wall scrub is a sliver per step but real over a
        // held slide.
        if self.wall_stop_deg <= 0.0 || self.wall_stop_deg >= 90.0 {
            w.push(format!("wall_stop_deg {} must be a window strictly between 0 and 90", self.wall_stop_deg));
        }
        if self.wall_friction <= 0.0 {
            w.push(format!("wall_friction {} must be positive", self.wall_friction));
        } else {
            if self.wall_friction * SIM_DT >= 0.1 * self.move_speed {
                w.push(format!(
                    "one step's wall scrub ({:.3} m/s) is more than a sliver of the run: a graze should barely dent it",
                    self.wall_friction * SIM_DT
                ));
            }
            if self.move_speed / self.wall_friction >= 1.0 {
                w.push(format!(
                    "a full-speed wall slide takes {:.3}s to scrub out (want < 1s)",
                    self.move_speed / self.wall_friction
                ));
            }
        }

        // The downhill glue must cover one full-speed step down the steepest walkable slope, or a
        // fast downhill walk outruns the snap and flickers airborne; and it must stay a glue, well
        // under the body's own height, not a teleport.
        let cos = self.walkable_cos();
        if cos > 0.0 && cos < 1.0 {
            let gradient = (1.0 - cos * cos).sqrt() / cos;
            let step_drop = self.move_speed * SIM_DT * gradient;
            if self.snap_down_distance < step_drop {
                w.push(format!(
                    "snap_down_distance {} cannot cover a full-speed step down the {}-degree slope ({step_drop:.3}m)",
                    self.snap_down_distance, self.walkable_degrees
                ));
            }
        }
        if self.snap_down_distance >= PLAYER_HEIGHT * 0.5 {
            let d = self.snap_down_distance;
            w.push(format!("snap_down_distance {d} is a teleport, not a glue (>= half the body height)"));
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
        // The shipped truth must be sane on its own terms: every relationship the validator checks
        // holds for the defaults, so a warning on load always means the FILE diverged, never that
        // the shipped feel was broken.
        let warnings = Tuning::default().validate();
        assert!(warnings.is_empty(), "the shipped defaults must validate clean, got: {warnings:?}");
    }

    #[test]
    fn the_tracked_tuning_file_parses_and_validates() {
        // The experiment surface must not be broken. This reads the tracked taste/tuning.json (via
        // the manifest dir, so it is found regardless of the test's working directory) and pins
        // that it parses and validates - deliberately NOT that it equals the defaults: the file is
        // free to diverge as the human tunes feel, the defaults are the shipped baseline.
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tuning.json");
        let tuning = load(&path).expect("the tracked tuning.json should parse");
        let warnings = tuning.validate();
        assert!(warnings.is_empty(), "the tracked tuning.json should validate clean, got: {warnings:?}");
    }

    #[test]
    fn the_derived_jump_round_trips_to_the_parameters() {
        // The derivation's whole point: plugging the derived (g, v) back into the kinematics returns
        // the authored parameters. Apex v^2 / 2g = h and rise v / g = t, to float roundoff.
        let t = Tuning::default();
        let apex = t.jump_velocity() * t.jump_velocity() / (2.0 * t.ascent_gravity());
        let rise = t.jump_velocity() / t.ascent_gravity();
        assert!((apex - t.jump_apex_height).abs() < 1e-5, "derived apex {apex} vs parameter {}", t.jump_apex_height);
        assert!((rise - t.jump_time_to_apex).abs() < 1e-6, "derived rise {rise} vs parameter {}", t.jump_time_to_apex);
    }

    #[test]
    fn the_walkable_pairing_is_derived_not_detunable() {
        // One angle drives both grades: the flat-resolve threshold sits a hair under the walkable
        // cosine and within a margin of it, by construction, for any walkable angle. This is what
        // the constants era had to pin as a relationship test (0.49 vs 0.5 by hand); deriving both
        // from one field makes it true for free, and this guards the derivation.
        for degrees in [30.0_f32, 45.0, 60.0, 75.0] {
            let t = Tuning { walkable_degrees: degrees, ..Tuning::default() };
            assert!(t.walkable_normal_y() <= t.walkable_cos(), "{degrees} deg: normal_y exceeded the cosine");
            assert!(
                t.walkable_normal_y() > t.walkable_cos() - 0.05,
                "{degrees} deg: the flat-resolve grade drifted a regime away from the walkable limit"
            );
        }
    }

    #[test]
    fn validate_catches_friction_weaker_than_acceleration() {
        let t = Tuning { ground_friction: 50.0, ground_accel: 90.0, ..Tuning::default() };
        assert!(t.validate().iter().any(|m| m.contains("ground_friction")), "should warn on soft friction");
    }

    #[test]
    fn validate_catches_a_snap_down_too_short_for_the_steepest_walkable_step() {
        // A snap-down that cannot cover one full-speed step down the walkable limit is the exact bug
        // the glue exists to remove: the validator must catch it.
        let t = Tuning { snap_down_distance: 0.05, ..Tuning::default() };
        assert!(t.validate().iter().any(|m| m.contains("snap_down_distance")), "should warn on a short glue");
    }

    #[test]
    fn validate_catches_a_stop_distance_outside_the_precision_band() {
        let slidey = Tuning { ground_friction: 90.0, move_speed: 12.0, ..Tuning::default() };
        assert!(slidey.validate().iter().any(|m| m.contains("stop distance")), "should warn on an overshooting stop");
    }

    #[test]
    fn validate_catches_a_coyote_window_longer_than_half_the_rise() {
        let t = Tuning { coyote_s: 0.3, ..Tuning::default() };
        assert!(t.validate().iter().any(|m| m.contains("coyote_s")), "should warn on an oversized coyote window");
    }

    #[test]
    fn validate_catches_a_symmetric_fall() {
        let t = Tuning { fall_gravity_mult: 1.0, ..Tuning::default() };
        assert!(t.validate().iter().any(|m| m.contains("fall_gravity_mult")), "should warn on a non-committing fall");
    }

    #[test]
    fn validate_catches_an_ice_rink_acceleration() {
        // The crispness bar: too long a ramp to top speed reads as ice. The other half of the brief's
        // friction-vs-accel relationship, pinned alongside it.
        let t = Tuning { ground_accel: 20.0, ..Tuning::default() };
        assert!(t.validate().iter().any(|m| m.contains("ice")), "should warn on a slidey ramp-up");
    }

    #[test]
    fn validate_catches_a_dead_wall_stop_window() {
        let t = Tuning { wall_stop_deg: 0.0, ..Tuning::default() };
        assert!(t.validate().iter().any(|m| m.contains("wall_stop_deg")), "should warn on a degenerate stop window");
    }

    #[test]
    fn validate_catches_an_air_reversal_that_cannot_finish() {
        // Too slow an air turn cannot complete a heading reversal before the jump lands - the do-over
        // the air model sells would never finish.
        let t = Tuning { air_turn_rate: 1.0, ..Tuning::default() };
        assert!(t.validate().iter().any(|m| m.contains("air reversal")), "should warn on an unfinishable reversal");
    }

    #[test]
    fn validate_catches_a_double_jump_stronger_than_the_ground_jump() {
        let t = Tuning { air_jump_scale: 1.5, ..Tuning::default() };
        assert!(t.validate().iter().any(|m| m.contains("air_jump_scale")), "should warn on a too-strong air jump");
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
        assert_eq!(t.ground_accel, Tuning::default().ground_accel, "an omitted field falls back to the default");
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
