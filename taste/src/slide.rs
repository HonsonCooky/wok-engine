//! The player's collide-and-slide over the cylinder sweeps: flat-bottom support, the wall
//! policies, and the step-up.
//!
//! The player's collider is a flat-bottomed vertical cylinder (`wok_physics::Cylinder`, swept by
//! `sweep_cylinder_colliders`); the drawn bean stays a capsule, a deliberate visual mismatch
//! documented at the draw site (`crate::app`). The slide runs the engine sweep's loop shape (same
//! skin, same iteration cap) and applies taste's resolve policies per contact:
//!
//! **Support is geometric.** A contact is SUPPORT when its normal grades as ground
//! (`normal.y >= WALKABLE_NORMAL_Y`, up to the 60-degree walkable limit) and its contact point
//! lies under the disc footprint (horizontally within `PLAYER_RADIUS` of the axis, plus a small
//! slack for the slide skin). The capsule's vertical-tolerance dance (`SUPPORT_TOLERANCE_M`, the
//! probe that re-derived bearing surfaces per collider shape) retires: a flat bottom bears on
//! exactly what is under it, so the contact the sweep already found IS the support answer. This
//! is what makes tilted faces standable to the walkable limit, lets the body stand with its axis
//! past a ledge while the rim is supported, and keeps corner landings from rolling off.
//!
//! **Supported contacts resolve flat** (the edge-drift fix, carried over): flat (+Y) first, so
//! the vertical part of motion and velocity dies in the contact exactly as on flat ground, then
//! the true plane, so surviving horizontal motion follows the surface. Unsupported resolutions
//! keep true normals: airborne deflections and genuine edge departures are untouched.
//!
//! **Wall-grade contacts** (`|normal.y| < WALKABLE_NORMAL_Y`) carry three policies in priority
//! order:
//!
//! - **Step-up** (new with the flat bottom): the rounded capsule glided up small lips for free;
//!   the flat bottom stops square against them. When the slide entered grounded (the caller's
//!   `can_step_up`), the motion is not rising, and the blocking contact's point is within
//!   `STEP_HEIGHT` of the foot, the slide attempts lift-move-drop: translate up (as far as
//!   headroom allows, at most `STEP_HEIGHT`), retry the horizontal motion, and settle back down
//!   onto whatever is there. The contact point's height stands in for the obstacle's top: the
//!   closest-pair seed clamps to the top of any blocker shorter than the cylinder's centre, while
//!   a real wall yields a mid-height contact point and fails the test before any sweep is spent.
//!   A successful climb is not a wall touch (no stop, no scrub - climbing a step is the policy);
//!   a failed retry falls through to the wall policies. One climb per slide call.
//! - **The incidence stop**: horizontal motion within `WALL_STOP_DEG` of straight into the wall
//!   kills the tangential redirect - the player stops at the wall instead of skating. Vertical
//!   motion is exempt (gravity still slides a body down a wall).
//! - **The friction scrub**: any step that touched a wall (and did not climb it) decays the exit
//!   velocity's horizontal speed by one step of `WALL_FRICTION`. Geometry is untouched; vertical
//!   is exempt; contact-free steps never see it.
//!
//! Deterministic: fixed iteration caps, the engine sweep's slice-order contract, fixed
//! arithmetic; no RNG, no state. The pins live in test-only siblings: `crate::slide_feel` (the
//! wall policies and the support seams) and `crate::landing` (tilted faces, the overhang, and the
//! step-up through the real step).

use glam::Vec3;
use wok_physics::{Collider, Cylinder, SweptHit, sweep_cylinder_colliders};

use crate::constants::{
    PLAYER_RADIUS, SIM_DT, STEP_HEIGHT, WALKABLE_COS, WALKABLE_NORMAL_Y, WALL_FRICTION, WALL_STOP_DEG,
};

/// Small separation kept between the body and surfaces while sliding: the engine slide's value,
/// passed straight to the cylinder sweep's skin parameter.
const SKIN: f32 = 1e-3;

/// Below this squared length the leftover motion is negligible and the slide stops (engine value).
const MIN_MOVE_SQ: f32 = 1e-10;

/// Cap on slide iterations (engine value): floor plus two walls fits, leftover past it is dropped.
const MAX_ITERS: usize = 4;

/// Horizontal slack on the footprint test, beyond `PLAYER_RADIUS`: the sweep reports contact
/// points on the static surface, a skin outside the body, so a rim-bearing contact can sit a
/// fraction of a millimetre past the geometric disc. A centimetre covers it with margin while
/// staying far below anything that could promote a wall graze (wall contacts fail the normal
/// grade long before this slack matters).
const FOOTPRINT_SLACK: f32 = 0.01;

/// The retry sweep after a lift must clear the blocker by more than this fraction of the motion,
/// or the obstacle still stands at the lifted height and the contact is a real wall.
const STEP_PROGRESS_TOI: f32 = 1e-3;

/// The outcome of a player slide: where the body ended up, the policy-resolved velocity, and the
/// two contact signals the landing policy reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerSlide {
    /// Resolved cylinder-centre position.
    pub position: Vec3,
    /// The input velocity through every contact's resolve policy, scrub included.
    pub velocity: Vec3,
    /// True if any contact's normal passed the walkable threshold (`WALKABLE_COS`): the raw
    /// contact grade, support or not.
    pub grounded: bool,
    /// True if any contact was genuine support: ground-grade normal AND contact point under the
    /// disc footprint. This is the landing policy's signal - the flat bottom is standing on
    /// something real, not grazing past it.
    pub supported: bool,
}

/// Move the player cylinder by `displacement` through the static `colliders`, sliding along
/// contacts under the module docs' policies. `can_step_up` gates the step-up: the caller passes
/// "was grounded at step entry and did not jump this step", so airborne motion and jumps never
/// climb.
pub fn slide_player(
    cylinder: Cylinder,
    displacement: Vec3,
    velocity: Vec3,
    colliders: &[Collider],
    can_step_up: bool,
) -> PlayerSlide {
    let mut body = cylinder;
    let mut remaining = displacement;
    let mut velocity = velocity;
    let mut grounded = false;
    let mut supported = false;
    let mut wall_contact = false;
    let mut stepped = false;

    for _ in 0..MAX_ITERS {
        if remaining.length_squared() <= MIN_MOVE_SQ {
            break;
        }
        match sweep_cylinder_colliders(&body, remaining, colliders, SKIN) {
            None => {
                body = body.translated(remaining);
                break;
            }
            Some(hit) => {
                let advance = remaining * hit.toi;
                body = body.translated(advance);
                if hit.normal.y >= WALKABLE_COS {
                    grounded = true;
                }
                let leftover = remaining - advance;
                if hit.normal.y >= WALKABLE_NORMAL_Y && under_footprint(hit.point, body.center) {
                    // Genuine support: flat first (the vertical part dies, as on flat ground),
                    // then the true plane so surviving horizontal motion follows the surface.
                    supported = true;
                    remaining = project_on_plane(project_on_plane(leftover, Vec3::Y), hit.normal);
                    velocity = project_on_plane(project_on_plane(velocity, Vec3::Y), hit.normal);
                } else if hit.normal.y.abs() < WALKABLE_NORMAL_Y {
                    // A wall: try the step-up first - a climbable lip is not a wall touch.
                    let foot = body.center.y - body.half_height;
                    if can_step_up && !stepped && remaining.y <= 0.0 && hit.point.y - foot <= STEP_HEIGHT
                        && let Some(climb) = try_step_up(body, leftover, colliders)
                    {
                        stepped = true;
                        body = climb.body;
                        remaining = climb.leftover;
                        if climb.landing.normal.y >= WALKABLE_COS {
                            grounded = true;
                        }
                        if under_footprint(climb.landing.point, body.center) {
                            supported = true;
                        }
                        velocity = project_on_plane(velocity, climb.landing.normal);
                        continue;
                    }
                    wall_contact = true;
                    if head_on(leftover, hit.normal) {
                        // The wall stop: inside the incidence window the tangential redirect
                        // dies - only the vertical part survives, projected onto the true plane
                        // so a tilted wall is still never pushed into.
                        remaining = project_on_plane(Vec3::new(0.0, leftover.y, 0.0), hit.normal);
                        velocity = project_on_plane(Vec3::new(0.0, velocity.y, 0.0), hit.normal);
                    } else {
                        remaining = project_on_plane(leftover, hit.normal);
                        velocity = project_on_plane(velocity, hit.normal);
                    }
                } else {
                    // Unsupported ground-grade contact or a ceiling: the engine's own projection.
                    remaining = project_on_plane(leftover, hit.normal);
                    velocity = project_on_plane(velocity, hit.normal);
                }
            }
        }
    }

    // The wall scrub: one step of WALL_FRICTION off the horizontal speed, only on steps that
    // touched (and did not climb) a wall, applied to the exit velocity after the resolve so the
    // step's geometry is untouched. Vertical is exempt, as in the stop.
    if wall_contact {
        let speed = Vec3::new(velocity.x, 0.0, velocity.z).length();
        let scrub = WALL_FRICTION * SIM_DT;
        let scale = if speed <= scrub { 0.0 } else { (speed - scrub) / speed };
        velocity.x *= scale;
        velocity.z *= scale;
    }

    PlayerSlide { position: body.center, velocity, grounded, supported }
}

/// Is `point` under the disc footprint of a body centred at `center`: horizontally within the
/// player radius (plus the skin slack). The whole support test - the flat bottom bears on
/// anything under it.
fn under_footprint(point: Vec3, center: Vec3) -> bool {
    let dx = point.x - center.x;
    let dz = point.z - center.z;
    dx * dx + dz * dz <= (PLAYER_RADIUS + FOOTPRINT_SLACK) * (PLAYER_RADIUS + FOOTPRINT_SLACK)
}

/// The result of a successful step-up: the settled body, the horizontal motion still untravelled,
/// and the walkable contact the settle landed on.
struct StepUp {
    body: Cylinder,
    leftover: Vec3,
    landing: SweptHit,
}

/// Lift-move-drop. Lift by `STEP_HEIGHT` (less if headroom stops it), retry the horizontal part
/// of `leftover` at the lifted height, then sweep back down by the lift and settle. A climb
/// counts only when the drop LANDS on a walkable surface: `None` when the retry is still blocked
/// at the lifted height (a real wall), or when the drop finds nothing walkable to stand on -
/// which covers the small-motion case where the step's travel cannot yet carry the disc over the
/// lip (the walk re-accelerates after the wall stop and climbs a step or two later, with the
/// geometry left untouched in the meantime). The vertical part of `leftover` (one step of gravity
/// during a grounded walk) is consumed by the climb.
fn try_step_up(body: Cylinder, leftover: Vec3, colliders: &[Collider]) -> Option<StepUp> {
    let horizontal = Vec3::new(leftover.x, 0.0, leftover.z);
    if horizontal.length_squared() <= MIN_MOVE_SQ {
        return None;
    }
    let lift = match sweep_cylinder_colliders(&body, Vec3::Y * STEP_HEIGHT, colliders, SKIN) {
        None => STEP_HEIGHT,
        Some(h) => STEP_HEIGHT * h.toi,
    };
    let lifted = body.translated(Vec3::Y * lift);

    let (advanced, leftover) = match sweep_cylinder_colliders(&lifted, horizontal, colliders, SKIN) {
        None => (lifted.translated(horizontal), Vec3::ZERO),
        Some(h) if h.toi > STEP_PROGRESS_TOI => {
            (lifted.translated(horizontal * h.toi), horizontal * (1.0 - h.toi))
        }
        Some(_) => return None,
    };

    let landing = sweep_cylinder_colliders(&advanced, Vec3::NEG_Y * lift, colliders, SKIN)?;
    if landing.normal.y < WALKABLE_NORMAL_Y {
        return None;
    }
    Some(StepUp { body: advanced.translated(Vec3::NEG_Y * (lift * landing.toi)), leftover, landing })
}

/// Remove the component of `v` along `normal` (the engine slide's projection). `normal` is unit
/// length (the sweep's normals are, and so is +Y).
fn project_on_plane(v: Vec3, normal: Vec3) -> Vec3 {
    v - normal * v.dot(normal)
}

/// Is this wall contact head-on: does the horizontal part of `motion` point within `WALL_STOP_DEG`
/// of straight into the wall (the wall's inward horizontal direction, `-normal` flattened)? The
/// comparison `-h.dot(w) >= |h| * |w| * cos(window)` is the angle test without normalizing; the
/// strict `> 0.0` guard makes the degenerate cases false: no horizontal motion means no incidence
/// to measure (a pure vertical graze keeps the engine resolve), and no horizontal normal means a
/// flat ceiling or floor, which this policy never touches. Inclusive at the window's edge,
/// matching "within".
fn head_on(motion: Vec3, normal: Vec3) -> bool {
    let h = Vec3::new(motion.x, 0.0, motion.z);
    let w = Vec3::new(normal.x, 0.0, normal.z);
    let into_wall = -h.dot(w);
    into_wall > 0.0 && into_wall >= h.length() * w.length() * WALL_STOP_DEG.to_radians().cos()
}
