//! The hitbox overlay: line-cage geometry for the F1 debug toggle (`DEBUG_HITBOXES` is the
//! default).
//!
//! Pure geometry building, no GPU: one call per frame turns the simulation's world into the
//! `LineSegment` list the renderer's debug line pass draws. The cages show what the fixed-step
//! loop actually collides with - every world-space static collider in its own true shape (box
//! cage, sphere rings, cylinder rings-and-verticals) and the player capsule at its exact collider
//! dimensions - so a play-tester can see collision-vs-visual disagreements directly instead of
//! inferring them from bumps. The skeleton must match the truth: a sphere drawn as its box would
//! reintroduce exactly the lie the round colliders removed.

use std::f32::consts::TAU;

use glam::Vec3;
use wok_physics::Collider;
use wok_render::LineSegment;
use wok_scene::Aabb;

use crate::constants::{PLAYER_RADIUS, PLAYER_SEGMENT};
use crate::world::World;

/// Hitbox cages: a saturated green nothing in the scene palette uses (the terrain's green is muted).
const HITBOX_COLOR: Vec3 = Vec3::new(0.15, 1.0, 0.3);

/// The player capsule cage: bright yellow, readable over the bean's signal orange.
const CAPSULE_COLOR: Vec3 = Vec3::new(1.0, 0.95, 0.2);

/// The look-ahead reticle: a neutral grey, present without shouting (it is framing feedback, not a
/// gameplay crosshair).
const RETICLE_COLOR: Vec3 = Vec3::new(0.75, 0.75, 0.75);

/// Half-length of each reticle arm, in metres: a ~0.1m cross overall.
const RETICLE_HALF: f32 = 0.05;

/// Line segments per debug ring: enough that a 0.45m circle reads round, few enough that the
/// overlay stays obviously diagnostic.
const RING_SEGMENTS: usize = 16;

/// Verticals on the capsule cage, evenly spaced around the wall.
const CAGE_VERTICALS: usize = 4;

/// The frame's debug overlay: every static collider in `world` as a cage in its own shape, plus
/// the player capsule as a line cage at `player_pos` (the interpolated draw position, so the cage
/// tracks the drawn bean, not the raw sim step).
pub fn debug_lines(world: &World, player_pos: Vec3) -> Vec<LineSegment> {
    let mut lines = Vec::with_capacity(world.statics.len() * 3 * RING_SEGMENTS + 2 * RING_SEGMENTS + CAGE_VERTICALS);
    for collider in &world.statics {
        match *collider {
            Collider::Aabb(ref aabb) => aabb_lines(aabb, &mut lines),
            Collider::Sphere { center, radius } => sphere_lines(center, radius, &mut lines),
            Collider::VertCylinder { center, radius, half_height } => {
                cylinder_lines(center, radius, half_height, &mut lines);
            }
        }
    }
    capsule_lines(player_pos, &mut lines);
    lines
}

/// A small three-axis cross at `at`: the camera's look-ahead point, drawn so the framing being
/// tuned is visible in play (`SHOW_RETICLE`). Appended rather than returned so it can ride the
/// same line-pass submission as the hitbox cages.
pub fn reticle_lines(at: Vec3, out: &mut Vec<LineSegment>) {
    for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
        out.push(LineSegment { start: at - axis * RETICLE_HALF, end: at + axis * RETICLE_HALF, color: RETICLE_COLOR });
    }
}

/// The 12 edges of an AABB.
fn aabb_lines(aabb: &Aabb, out: &mut Vec<LineSegment>) {
    let (lo, hi) = (aabb.min, aabb.max);
    // Each corner as a bit pattern (x, y, z from lo or hi); an edge joins corners differing in
    // exactly one bit, taken once by only walking toward the hi side.
    let corner = |i: usize| {
        Vec3::new(
            if i & 1 == 0 { lo.x } else { hi.x },
            if i & 2 == 0 { lo.y } else { hi.y },
            if i & 4 == 0 { lo.z } else { hi.z },
        )
    };
    for i in 0..8 {
        for bit in [1, 2, 4] {
            if i & bit == 0 {
                out.push(LineSegment { start: corner(i), end: corner(i | bit), color: HITBOX_COLOR });
            }
        }
    }
}

/// A circle of `RING_SEGMENTS` segments in the plane spanned by the orthonormal `u`, `v` around
/// `center`: the one stroke every round cage is drawn with.
fn circle_lines(center: Vec3, u: Vec3, v: Vec3, radius: f32, color: Vec3, out: &mut Vec<LineSegment>) {
    let at = |j: usize| {
        let angle = TAU * (j as f32 / RING_SEGMENTS as f32);
        center + (u * angle.cos() + v * angle.sin()) * radius
    };
    for j in 0..RING_SEGMENTS {
        out.push(LineSegment { start: at(j), end: at(j + 1), color });
    }
}

/// A sphere collider as three orthogonal great circles: the equator plus two meridians, enough
/// that the cage reads round from any camera angle.
fn sphere_lines(center: Vec3, radius: f32, out: &mut Vec<LineSegment>) {
    circle_lines(center, Vec3::X, Vec3::Z, radius, HITBOX_COLOR, out);
    circle_lines(center, Vec3::X, Vec3::Y, radius, HITBOX_COLOR, out);
    circle_lines(center, Vec3::Z, Vec3::Y, radius, HITBOX_COLOR, out);
}

/// A vertical-cylinder collider as a ring at each cap plus verticals spanning the wall: the same
/// rings-and-verticals stroke as the player capsule, at the collider's exact dimensions.
fn cylinder_lines(center: Vec3, radius: f32, half_height: f32, out: &mut Vec<LineSegment>) {
    rings_and_verticals(center, radius, half_height, HITBOX_COLOR, out);
}

/// The player capsule as a cage: a ring at each cap equator (where the hemispheres meet the wall,
/// `half-segment` above and below the centre) and a few verticals spanning the wall between them.
/// Enough to read position, radius, and height at a glance; the full silhouette lives in the
/// rendered capsule mesh itself.
fn capsule_lines(center: Vec3, out: &mut Vec<LineSegment>) {
    rings_and_verticals(center, PLAYER_RADIUS, PLAYER_SEGMENT * 0.5, CAPSULE_COLOR, out);
}

/// The shared rings-and-verticals stroke: a horizontal ring `half_span` above and below `center`,
/// joined by `CAGE_VERTICALS` evenly spaced wall lines.
fn rings_and_verticals(center: Vec3, radius: f32, half_span: f32, color: Vec3, out: &mut Vec<LineSegment>) {
    for y in [-half_span, half_span] {
        circle_lines(center + Vec3::Y * y, Vec3::X, Vec3::Z, radius, color, out);
    }
    for j in 0..CAGE_VERTICALS {
        let angle = TAU * (j as f32 / CAGE_VERTICALS as f32);
        let on_wall = Vec3::new(radius * angle.cos(), 0.0, radius * angle.sin());
        out.push(LineSegment {
            start: center + on_wall - Vec3::Y * half_span,
            end: center + on_wall + Vec3::Y * half_span,
            color,
        });
    }
}
