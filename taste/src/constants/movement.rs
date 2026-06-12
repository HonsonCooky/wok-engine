//! Movement tuning: the fixed step, the wall policies, the player body, locomotion, and the jump.

use glam::Vec3;

// ---- simulation ----

/// The fixed simulation timestep: 60 steps per second of game time, never a wall-clock delta. The
/// fixed dt is the day-one decision behind deterministic scripted-input replay.
pub const SIM_DT: f32 = 1.0 / 60.0;

/// Most fixed steps one rendered frame may consume. A long stall (a debugger pause, a window drag)
/// otherwise turns into a catch-up burst that itself takes too long, which accumulates more debt: the
/// spiral of death. Past the clamp the leftover time is dropped and the game slows down instead.
pub const MAX_STEPS_PER_FRAME: u32 = 8;

/// cos(60 degrees): the steepest slope that still counts as walkable ground, passed to both the
/// slide and the terrain rest so the two grounded signals agree. Raised from 45 degrees on the
/// play verdict: steep hillsides should ground, rest, and walk normally. With the flat-bottomed
/// cylinder the limit applies uniformly: tilted collider faces stand to the same 60 degrees as
/// terrain (the capsule-era ~15-degree shed is retired with the support tolerance it came from).
pub const WALKABLE_COS: f32 = 0.5;

/// Contacts at least this upright (by normal.y) grade as ground for the slide's flat-resolve
/// policy (`crate::slide`): when the player is genuinely supported at the contact, such a contact
/// resolves as flat ground, so gravity dies in the contact instead of leaking sideways - the
/// edge-drift fix. Set a hair under WALKABLE_COS so every contact the grounded signal accepts
/// also qualifies; wall-grade contacts and all unsupported resolution keep their true normals.
/// Moves with WALKABLE_COS (the relationship test pins the pairing), so the walkable retune to
/// 60 degrees carried it from 0.7 down with the threshold.
pub const WALKABLE_NORMAL_Y: f32 = 0.49;

/// The wall stop's incidence window, in degrees from head-on. A wall-grade contact whose
/// horizontal motion points within this angle of straight into the wall kills its tangential
/// redirect: the player stops at the wall instead of skating along it (the play verdict: running
/// at a wall should read as a stop, not a deflection). Beyond the window, glancing contacts slide
/// as the engine resolves them (less the wall friction below); vertical motion (gravity along a
/// wall) never enters the test. Narrowed from 45 on the follow-up verdict: the stop cone was too
/// wide, and a quarter-angle approach should slide. Policy in `crate::slide`.
pub const WALL_STOP_DEG: f32 = 30.0;

/// Tangential deceleration while in wall contact, in m/s^2: the horizontal speed of a wall-grade
/// sliding contact decays at this rate, applied only on steps that actually touch a wall, so a
/// wall slide scrubs speed instead of feeling like glass. Vertical motion is exempt (gravity
/// still slides a body down a wall, as in the stop), and ground or airborne non-contact motion
/// never sees it. A full-speed slide scrubs out in MOVE_SPEED / WALL_FRICTION seconds (0.3s);
/// one step's scrub (~0.42 m/s) is a sliver, so a brief graze barely dents the run. Policy in
/// `crate::slide`.
pub const WALL_FRICTION: f32 = 25.0;

// ---- player ----

/// Player body total height and radius, in metres - shared by the collider and the visual. The
/// COLLIDER is a flat-bottomed vertical cylinder of exactly these dimensions
/// (`Cylinder::upright`): the flat bottom is what stands on tilted faces, overhangs ledges, and
/// does not roll off edges. The VISUAL stays the bean (the capsule mesh at the same height and
/// radius; the mismatch is documented at the draw site in `crate::app`). 1.5 over 0.9 wide is the
/// bean silhouette the play-tests settled on.
pub const PLAYER_HEIGHT: f32 = 1.5;
pub const PLAYER_RADIUS: f32 = 0.45;

/// The drawn capsule's wall (cylinder segment) length the height and radius imply - what the bean
/// mesh (`capsule_mesh`) sizes its straight section from. Visual only since the collider became
/// the cylinder: the collider's straight wall is the full PLAYER_HEIGHT.
pub const PLAYER_SEGMENT: f32 = PLAYER_HEIGHT - 2.0 * PLAYER_RADIUS;

/// Step-up height, in metres: a grounded walk blocked by a wall-grade contact no taller than this
/// above the foot climbs it (lift-move-drop in `crate::slide`) instead of stopping. The flat
/// bottom needs the policy where the capsule's rounded bottom glided up small lips for free; 0.3m
/// is shin height - kerbs and stair treads climb, crates (0.5m and up) are walls.
pub const STEP_HEIGHT: f32 = 0.3;

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

/// How fast airborne input rotates the horizontal velocity's direction toward the stick, in
/// radians per second - the ONLY airborne control. Air is pure momentum (policy in `crate::air`):
/// a jump's horizontal speed is set at launch and never changes until landing, so the stick turns
/// the heading but can never stretch or shrink the jump's reach. Redirection rather than
/// acceleration-through-zero is the BFBB / Ratchet & Clank air authority: reversing heading
/// mid-jump turns the moving velocity around (a half circle in ~0.52s, most of a jump's hang
/// time) instead of braking through a dead stop. The speed-magnitude approach this rate used to
/// pair with (AIR_ACCEL, 12 m/s^2) retired on the pure-momentum verdict; friction never applies
/// airborne either - with no input the velocity is ballistic (air friction pinned bodies onto
/// crate corners, the last mid-air halt).
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

/// Apex height of a ground jump, in metres: the ~1.9m the raised-jump verdict landed on, restated
/// as the parameter instead of a consequence. Every jump flies this full arc - height never
/// depends on how long the control is held (the verdict that removed the jump cut).
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
    fn the_coyote_window_is_grace_not_flight() {
        // The window must survive quantization (at least a couple of fixed steps, or a single
        // dropped frame eats it) and stay well inside the jump's own rise, or hovering off ledges
        // stops reading as forgiveness and starts reading as a mechanic.
        assert!(COYOTE_S >= 2.0 * SIM_DT, "a window under two steps is luck, not grace");
        assert!(COYOTE_S <= JUMP_TIME_TO_APEX * 0.5, "the grace must stay small against the jump itself");
    }

    #[test]
    fn the_player_body_is_taller_than_it_is_wide_and_the_bean_keeps_its_wall() {
        // The drawn capsule needs height > 2 * radius or its straight section vanishes (the
        // mesh's silhouette); the cylinder collider shares the numbers, so this also keeps the
        // body a standing shape rather than a coin.
        assert!(PLAYER_HEIGHT > 2.0 * PLAYER_RADIUS);
        assert!(PLAYER_RADIUS > 0.0);
    }

    #[test]
    fn the_step_height_is_a_shin_not_a_climb() {
        // Zero would retire the policy; at or above half the body the "step" would swallow the
        // crates the demo treats as obstacles, and a step should never substitute for a jump.
        assert!(STEP_HEIGHT > 0.0);
        assert!(STEP_HEIGHT < PLAYER_HEIGHT * 0.5, "a step is climbed by the feet, not the body");
        assert!(STEP_HEIGHT < JUMP_APEX_HEIGHT, "anything jump-worthy must still need the jump");
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
        // must stay a hair under the threshold, not a regime apart, or near-vertical walls would
        // start flat-resolving and stop pushing back. The pairing is the contract: a walkable
        // retune moves both together (45 -> 60 degrees carried 0.7 -> 0.49).
        assert!(WALKABLE_NORMAL_Y <= WALKABLE_COS);
        assert!(WALKABLE_NORMAL_Y > WALKABLE_COS - 0.05, "the flat-resolve grade must move with the walkable limit");
    }

    #[test]
    fn the_wall_stop_window_is_a_real_window() {
        // 0 would make the stop unreachable (only an exact head-on hit, measure-zero in float);
        // 90 or more would stop every wall contact and walls would have no slide at all.
        assert!(WALL_STOP_DEG > 0.0 && WALL_STOP_DEG < 90.0);
    }

    #[test]
    fn acceleration_starts_fast_and_friction_stops_harder() {
        // The precision-kit crispness bar: from rest to top speed within two tenths of a second
        // (the keyboard-tap responsiveness verdict), and stopping at least as hard as starting so
        // releasing input never reads slippery.
        assert!(MOVE_SPEED / GROUND_ACCEL < 0.2, "too slow to top speed: reads as ice");
        assert!(GROUND_FRICTION >= GROUND_ACCEL);
    }

    #[test]
    fn the_wall_scrub_is_a_sliver_per_step_but_real_over_a_slide() {
        // The friction's two felt claims: a brief graze barely dents the run (one contact step
        // scrubs well under a tenth of top speed), while a held slide scrubs out fast (a
        // full-speed slide dies within a second of contact).
        assert!(WALL_FRICTION > 0.0);
        assert!(WALL_FRICTION * SIM_DT < 0.1 * MOVE_SPEED, "one step's scrub should be a sliver of the run");
        assert!(MOVE_SPEED / WALL_FRICTION < 1.0, "a held wall slide should scrub out within a second");
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
    fn the_snap_distance_covers_a_full_speed_step_down_the_steepest_walkable_slope() {
        // One step of full-speed walking descends at most MOVE_SPEED * SIM_DT * tan(max slope);
        // with WALKABLE_COS = cos(60 deg) that gradient is tan(60) = sqrt(3), so the worst
        // per-step descent is 7.5 / 60 * 1.732, about 0.22m - up from 0.125m at the old 45-degree
        // limit, still inside the 0.3m glue, and that headroom is exactly what this assertion
        // pins. The glue must cover it, or a fast downhill walk outruns the snap and flickers
        // airborne - the exact bug the glue removes.
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
    }
}
