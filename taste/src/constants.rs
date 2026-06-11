//! Gameplay tuning constants: the numbers that make taste feel like taste.
//!
//! These are gameplay policy, not engine values (HLD principle 5: the engine provides the math, the
//! game owns the numbers). Everything a designer would reach for to retune the demo lives here, in
//! one place: the fixed simulation rate, gravity, locomotion speeds, the player capsule's shape, and
//! the follow camera's geometry and easing rates. The sanity tests at the bottom pin the structural
//! relationships between them (a capsule taller than its own sphere, a jump that clears something, a
//! boom longer than its probe), so a retune that breaks the demo's assumptions fails in `cargo test`
//! rather than in play.

use glam::Vec3;

// ---- simulation ----

/// The fixed simulation timestep: 60 steps per second of game time, never a wall-clock delta. The
/// fixed dt is the day-one decision behind deterministic scripted-input replay.
pub const SIM_DT: f32 = 1.0 / 60.0;

/// Most fixed steps one rendered frame may consume. A long stall (a debugger pause, a window drag)
/// otherwise turns into a catch-up burst that itself takes too long, which accumulates more debt: the
/// spiral of death. Past the clamp the leftover time is dropped and the game slows down instead.
pub const MAX_STEPS_PER_FRAME: u32 = 8;

/// cos(45 degrees): the steepest slope that still counts as walkable ground, passed to both the
/// slide and the terrain rest so the two grounded signals agree.
pub const WALKABLE_COS: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// Contacts at least this upright (by normal.y) grade as ground for the slide's flat-resolve
/// policy (`crate::slide`): when the player is genuinely supported at the contact, such a contact
/// resolves as flat ground, so gravity dies in the contact instead of leaking sideways - the
/// edge-drift fix. Set a hair under WALKABLE_COS so every contact the grounded signal accepts
/// also qualifies; wall-grade contacts and all unsupported resolution keep their true normals.
pub const WALKABLE_NORMAL_Y: f32 = 0.7;

// ---- player ----

/// Player capsule total height (tip to tip) and radius, in metres. The segment length follows from
/// these via `Capsule::upright`: half-segment = height / 2 - radius. A squat, wide bean rather than
/// a tall pill: play-testing read the character better low and round under a third-person camera.
/// 1.5 over 0.9 wide is the bean silhouette - the earlier 1.1 was nearly a sphere, which read fine
/// while the placeholder was an ellipsoid but loses the silhouette now the capsule renders true.
pub const PLAYER_HEIGHT: f32 = 1.5;
pub const PLAYER_RADIUS: f32 = 0.45;

/// The capsule wall (cylinder segment) length the height and radius imply - what `Capsule::upright`
/// derives internally - restated as a constant so the drawn `capsule_mesh` and the debug cage size
/// from exactly the numbers the collider uses.
pub const PLAYER_SEGMENT: f32 = PLAYER_HEIGHT - 2.0 * PLAYER_RADIUS;

/// Horizontal locomotion speed in m/s, a brisk run. Raised from 6.0 on the feel-pass verdict:
/// crossing the demo's spaces at 6 read as too slow.
pub const MOVE_SPEED: f32 = 7.5;

/// Horizontal acceleration toward the intended velocity, in m/s^2, grounded with input. From rest
/// to top speed in MOVE_SPEED / GROUND_ACCEL seconds (0.083s, five fixed steps): the precision-kit
/// verdict against the old 40 (0.19s), which read slidey under keyboard taps - a digital key asks
/// for full speed now, and a long ramp-up smears every small correction.
pub const GROUND_ACCEL: f32 = 90.0;

/// Horizontal deceleration toward rest, in m/s^2, grounded with no input. Stronger than the
/// acceleration so stopping reads planted: top speed to rest in 0.05s, sliding
/// MOVE_SPEED^2 / (2 * GROUND_FRICTION) metres (~0.19m; the old 50 slid ~0.56m, most of a tile,
/// which is what made precision landings overshoot).
pub const GROUND_FRICTION: f32 = 150.0;

/// Airborne multiplier on the speed-change acceleration. Under the redirection model
/// (`crate::air`) this scales how fast airborne speed magnitude approaches the intended speed;
/// direction is AIR_TURN_RATE's job. Friction never applies airborne - with no input the velocity
/// is ballistic (policy in `crate::sim::step`: air friction pinned bodies onto crate corners, the
/// last mid-air halt).
pub const AIR_CONTROL: f32 = 0.55;

/// How fast airborne input rotates the horizontal velocity's direction toward the stick, in
/// radians per second. Redirection rather than acceleration-through-zero is the BFBB / Ratchet &
/// Clank air authority: reversing heading mid-jump turns the moving velocity around (a half
/// circle in ~0.52s, most of a jump's hang time) instead of braking through a dead stop, so a
/// redirect never collapses the speed.
pub const AIR_TURN_RATE: f32 = 6.0;

/// Extra jumps available while airborne, restored on any grounding. One is the double jump.
pub const AIR_JUMPS: u32 = 1;

/// The air jump's launch velocity as a fraction of JUMP_VELOCITY: a touch weaker than the ground
/// jump so the double jump reads as a recovery, not a free second full jump.
pub const AIR_JUMP_SCALE: f32 = 0.9;

// ---- the parameterized jump ----
//
// The jump is parameterized the way it is judged in play (Pittman's "tuning a jump" model): a
// designer retunes WHERE the apex is and WHEN it arrives, and the physics constants follow. Under
// constant ascent gravity g, a launch velocity v rises for t = v / g seconds and peaks at
// h = v^2 / 2g metres. Solving that pair for (g, v) given (h, t):
//
//     g = 2h / t^2        v = g * t = 2h / t
//
// The old hand-picked GRAVITY (25) and JUMP_VELOCITY (9.8) retire into these derived values: at
// the starting parameters they come out at 26.3 and 10.0, the same jump within a few percent, now
// steered by the two numbers a play-test verdict actually talks about.

/// Apex height of a full (held) ground jump, in metres: the ~1.9m the raised-jump verdict landed
/// on, restated as the parameter instead of a consequence.
pub const JUMP_APEX_HEIGHT: f32 = 1.9;

/// Time from launch to that apex, in seconds. Shorter is snappier, longer is floatier; 0.38s is
/// the old 25-gravity arc's rise time, the keep-the-feel starting point.
pub const JUMP_TIME_TO_APEX: f32 = 0.38;

/// Ascent gravity magnitude in m/s^2, derived: 2h / t^2. Applies while the player is rising.
pub const ASCENT_GRAVITY: f32 = 2.0 * JUMP_APEX_HEIGHT / (JUMP_TIME_TO_APEX * JUMP_TIME_TO_APEX);

/// Upward velocity granted by a ground (or coyote) jump, in m/s, derived: 2h / t.
pub const JUMP_VELOCITY: f32 = 2.0 * JUMP_APEX_HEIGHT / JUMP_TIME_TO_APEX;

/// Gravity multiplier while descending. Symmetric arcs read floaty on the way down: the eye
/// expects a jump to commit to its landing. Scaling only the descent keeps the tuned apex and rise
/// untouched while the fall arrives sqrt(FALL_GRAVITY_MULT) times sooner.
pub const FALL_GRAVITY_MULT: f32 = 1.7;

/// Descent gravity magnitude in m/s^2: the ascent gravity under the fall multiplier. The split is
/// applied by `sim::gravity` on the vertical velocity's sign.
pub const FALL_GRAVITY: f32 = ASCENT_GRAVITY * FALL_GRAVITY_MULT;

/// Variable jump height: releasing the jump control while still rising scales the vertical
/// velocity by this, once per jump, so a tap gives a short hop and a hold gives the full apex.
/// Apex scales with velocity squared, so the minimum hop is about JUMP_CUT_FACTOR^2 of the full
/// height (~0.38m at 0.45): real enough to clear a step, short enough to read as a tap.
pub const JUMP_CUT_FACTOR: f32 = 0.45;

/// Coyote time, in seconds of simulation time: how long after walking off an edge (leaving the
/// ground without jumping) a ground jump remains available. The player judges the jump against the
/// drawn body, which is right at the edge when the simulation has already left it; the grace
/// window makes the late press land the full jump instead of silently spending the air jump. A
/// jump consumes the window immediately, so it can never stack a free extra jump.
pub const COYOTE_S: f32 = 0.10;

/// How long a jump press stays buffered waiting for ground, in seconds of simulation time. A
/// press up to this long before landing fires on the landing step instead of being swallowed by
/// an airborne one (`crate::jump`). Around 100ms is the feel-polish standard: long enough that a
/// press timed against the visible landing always lands, short enough that a stale press never
/// fires a visibly delayed jump.
pub const JUMP_BUFFER_S: f32 = 0.10;

/// How far above the terrain surface the player spawns, in metres: high enough that the opening
/// moments show gravity and the landing.
pub const SPAWN_HEIGHT: f32 = 10.0;

/// Ground glue, in metres: walking downhill, the surface falls away faster than one step of
/// gravity can follow, so without glue a grounded walk flickers airborne every step. If the player
/// was grounded, did not jump, and the support is within this distance below the foot after the
/// move, the foot snaps to the support and stays grounded. Genuine drops (a ledge taller than
/// this) still go airborne, and a jump always leaves the ground. Game policy, not physics: the
/// engine's terrain rest is lift-only by design.
pub const SNAP_DOWN_DISTANCE: f32 = 0.3;

/// The placeholder ellipsoid's flat base color (linear RGB): a warm signal orange, distinct from
/// every surface-tag color the terrain and prefabs use.
pub const PLAYER_COLOR: Vec3 = Vec3::new(0.90, 0.35, 0.15);

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

// ---- diagnostics ----

/// Draw the ground-truth marker: a bright quad at the sampled terrain height under the player,
/// composed through the same chunk-origin path as the terrain mesh. For the floating-at-rest
/// diagnosis: if the marker lies on the rendered terrain while the bean floats, the gap is in the
/// rest math; if the marker itself disagrees with the rendered terrain, sampling and mesh disagree.
/// Off by default: the shadow map carries the grounding cue in normal play now, so the marker
/// retires to an opt-in diagnostic.
pub const DEBUG_GROUND_MARKER: bool = false;

/// Default for the hitbox overlay (F1 flips it at runtime): every loaded hitbox collider drawn as
/// a line cage plus the player capsule as rings and verticals, through the renderer's debug line
/// pass. The drawn scene shows visible shapes; this shows what the simulation actually collides
/// with, so visual-only and trigger-only placeholders stop being invisible to a play-tester.
pub const DEBUG_HITBOXES: bool = false;

/// Draw a small cross at the camera's look-at point (the look-ahead target), through the line
/// pass. On while the look-ahead framing is being tuned: it shows exactly where the view leads,
/// which is otherwise invisible in play.
pub const SHOW_RETICLE: bool = true;

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
// Asserting on constants is this module's entire purpose: the tests pin relationships between
// tuning values so a retune that breaks an assumption fails loudly. The lint assumes a constant
// assertion is an accident; here it is the point. Exact float comparison is likewise intended:
// these are declared values, not computed ones.
#[allow(clippy::assertions_on_constants, clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn the_simulation_rate_is_a_real_fixed_step() {
        assert!(SIM_DT > 0.0, "a non-positive dt advances nothing");
        assert!(MAX_STEPS_PER_FRAME >= 1, "a frame must be able to consume at least one step");
        // The clamp must cover at least a couple of ordinary frames of debt, or normal jitter stalls.
        assert!(MAX_STEPS_PER_FRAME as f32 * SIM_DT >= 2.0 / 60.0);
    }

    #[test]
    fn the_derived_jump_constants_round_trip_to_the_parameters() {
        // The derivation's whole point: plugging the derived (g, v) back into the kinematics must
        // return the authored parameters. Apex height v^2 / 2g = h and rise time v / g = t, to
        // float roundoff of the constant arithmetic.
        let apex = JUMP_VELOCITY * JUMP_VELOCITY / (2.0 * ASCENT_GRAVITY);
        let rise = JUMP_VELOCITY / ASCENT_GRAVITY;
        assert!((apex - JUMP_APEX_HEIGHT).abs() < 1e-5, "derived apex {apex} vs parameter {JUMP_APEX_HEIGHT}");
        assert!((rise - JUMP_TIME_TO_APEX).abs() < 1e-6, "derived rise {rise} vs parameter {JUMP_TIME_TO_APEX}");
    }

    #[test]
    fn the_fall_is_heavier_than_the_rise_but_still_a_fall() {
        // The asymmetric arc: descent gravity must exceed ascent gravity (a multiplier of 1 would
        // make the split dead code), and both must be real downward pulls.
        assert!(ASCENT_GRAVITY > 0.0);
        assert!(FALL_GRAVITY_MULT > 1.0, "the fall multiplier must actually shorten the descent");
        assert!(FALL_GRAVITY > ASCENT_GRAVITY);
    }

    #[test]
    fn the_jump_cut_is_a_real_partial_cut() {
        // 0 would kill a tapped jump outright (vertical velocity zeroed at release); 1 would make
        // variable height a no-op. The minimum hop, JUMP_CUT_FACTOR^2 of the apex, must still
        // clear something: a short hop, not a stumble.
        assert!(JUMP_CUT_FACTOR > 0.0 && JUMP_CUT_FACTOR < 1.0);
        let min_hop = JUMP_CUT_FACTOR * JUMP_CUT_FACTOR * JUMP_APEX_HEIGHT;
        assert!(min_hop > 0.2, "minimum hop {min_hop} too small to read as a jump");
    }

    #[test]
    fn the_coyote_window_is_grace_not_flight() {
        // The window must survive quantization (at least a couple of fixed steps, or a single
        // dropped frame eats it) and stay well inside the jump's own rise, or hovering off ledges
        // stops reading as forgiveness and starts reading as a mechanic.
        assert!(COYOTE_S >= 2.0 * SIM_DT, "a window under two steps is luck, not grace");
        assert!(COYOTE_S <= JUMP_TIME_TO_APEX * 0.5, "the grace must stay small against the jump itself");
    }

    #[test]
    fn the_player_capsule_is_taller_than_its_own_sphere() {
        // Below 2 * radius the upright capsule degrades to a sphere and the segment is gone.
        assert!(PLAYER_HEIGHT > 2.0 * PLAYER_RADIUS);
        assert!(PLAYER_RADIUS > 0.0);
    }

    #[test]
    fn a_jump_clears_something_worth_jumping() {
        // The apex is the parameter now, so the demo's obstacle bounds pin it directly: above a
        // man-height crate with room to feel generous, still under the tall prefabs, or they stop
        // being obstacles.
        assert!(JUMP_APEX_HEIGHT > 1.5, "apex {JUMP_APEX_HEIGHT} fell below the raised-jump verdict (~1.9m)");
        assert!(JUMP_APEX_HEIGHT < 4.0, "apex {JUMP_APEX_HEIGHT} clears the demo's tall prefabs");
    }

    #[test]
    fn the_walkable_threshold_is_a_real_slope_limit() {
        // cos of an angle strictly between flat (1.0) and vertical (0.0).
        assert!(WALKABLE_COS > 0.0 && WALKABLE_COS < 1.0);
    }

    #[test]
    fn the_flat_resolve_threshold_spans_ground_but_not_walls() {
        // Every contact the slide grades as ground (normal.y >= WALKABLE_COS) must also qualify
        // for the supported flat resolve, or a grounding contact could still bleed drift; and it
        // must sit well above any wall-grade normal, or walls would stop pushing back.
        assert!(WALKABLE_NORMAL_Y <= WALKABLE_COS);
        assert!(WALKABLE_NORMAL_Y > 0.5);
    }

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
    fn acceleration_starts_fast_and_friction_stops_harder() {
        // The precision-kit crispness bar: from rest to top speed within two tenths of a second
        // (the keyboard-tap responsiveness verdict), and stopping at least as hard as starting so
        // releasing input never reads slippery.
        assert!(MOVE_SPEED / GROUND_ACCEL < 0.2, "too slow to top speed: reads as ice");
        assert!(GROUND_FRICTION >= GROUND_ACCEL);
        // Airborne control is real but weaker than grounded: momentum survives a jump.
        assert!(AIR_CONTROL > 0.0 && AIR_CONTROL < 1.0);
    }

    #[test]
    fn a_full_speed_stop_lands_inside_a_quarter_metre() {
        // The constant-rate decay covers v^2 / 2f metres from top speed to rest: the slide the
        // player feels after releasing the key. A quarter metre is the precision bar (about half a
        // body width); the old friction of 50 slid ~0.56m, the felt cause of overshot landings.
        let stop_distance = MOVE_SPEED * MOVE_SPEED / (2.0 * GROUND_FRICTION);
        assert!(stop_distance <= 0.25, "stop distance {stop_distance} overshoots a precision landing");
        assert!(stop_distance > 0.05, "a stop this abrupt would read as hitting a wall");
    }

    #[test]
    fn an_air_redirect_can_reverse_heading_within_one_jump() {
        // The redirection promise: a full half-circle turn (the worst redirect) at AIR_TURN_RATE
        // must fit inside a jump's hang time, or the do-over the air model sells cannot finish
        // before landing. The arc is asymmetric now: the rise takes JUMP_TIME_TO_APEX and the
        // descent falls the apex height under the heavier fall gravity, sqrt(2h / g_fall).
        let hang_time = JUMP_TIME_TO_APEX + (2.0 * JUMP_APEX_HEIGHT / FALL_GRAVITY).sqrt();
        let reversal = std::f32::consts::PI / AIR_TURN_RATE;
        assert!(reversal < hang_time, "reversal {reversal}s must fit the jump's {hang_time}s");
    }

    #[test]
    fn the_double_jump_is_real_but_weaker_than_the_ground_jump() {
        assert!(AIR_JUMPS >= 1, "the double jump exists");
        assert!(AIR_JUMP_SCALE > 0.0 && AIR_JUMP_SCALE <= 1.0, "a recovery, not a stronger second jump");
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
    fn the_snap_distance_covers_a_full_speed_step_down_the_steepest_walkable_slope() {
        // One step of full-speed walking descends at most MOVE_SPEED * SIM_DT * tan(max slope);
        // with WALKABLE_COS = cos(45 deg) that gradient is 1, so the bound is set by the top speed
        // alone (7.5 m/s puts it at 0.125m) - the crispness retune of accel and friction moves how
        // fast that speed is reached, not the worst per-step descent, so the bound stands. The
        // glue must cover it, or a fast downhill walk outruns the snap and flickers airborne - the
        // exact bug the glue removes.
        let max_walkable_gradient = (1.0 - WALKABLE_COS * WALKABLE_COS).sqrt() / WALKABLE_COS;
        assert!(SNAP_DOWN_DISTANCE >= MOVE_SPEED * SIM_DT * max_walkable_gradient);
        // And it must stay a glue, not a teleport: well under the player's own height.
        assert!(SNAP_DOWN_DISTANCE < PLAYER_HEIGHT * 0.5);
    }

    #[test]
    fn locomotion_and_spawn_are_positive() {
        assert!(MOVE_SPEED > 0.0);
        assert!(JUMP_VELOCITY > 0.0);
        assert!(SPAWN_HEIGHT > 0.0);
        assert!(MOUSE_LOOK_SENSITIVITY > 0.0);
    }

    #[test]
    fn the_stick_deadzone_leaves_a_live_range() {
        // A deadzone of 1.0 or more silences the stick entirely; the rescale divides by its
        // complement, so it must also stay strictly below 1.
        assert!((0.0..1.0).contains(&STICK_DEADZONE));
        assert!(STICK_LOOK_RATE > 0.0);
    }
}
