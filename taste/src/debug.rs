//! The hitbox overlay: line-cage geometry for the F1 debug toggle (`DEBUG_HITBOXES` is the
//! default).
//!
//! Pure geometry building, no GPU: one call per frame turns the simulation's world into the
//! `LineSegment` list the renderer's debug line pass draws. The cages show what the fixed-step
//! loop actually collides with - the world-space static AABBs and the player capsule at its exact
//! collider dimensions - so a play-tester can see collision-vs-visual disagreements directly
//! instead of inferring them from bumps.

use std::f32::consts::TAU;

use glam::Vec3;
use wok_render::LineSegment;
use wok_scene::Aabb;

use crate::constants::{PLAYER_RADIUS, PLAYER_SEGMENT};
use crate::world::World;

/// Hitbox cages: a saturated green nothing in the scene palette uses (the terrain's green is muted).
const HITBOX_COLOR: Vec3 = Vec3::new(0.15, 1.0, 0.3);

/// The player capsule cage: bright yellow, readable over the bean's signal orange.
const CAPSULE_COLOR: Vec3 = Vec3::new(1.0, 0.95, 0.2);

/// Line segments per debug ring: enough that a 0.45m circle reads round, few enough that the
/// overlay stays obviously diagnostic.
const RING_SEGMENTS: usize = 16;

/// Verticals on the capsule cage, evenly spaced around the wall.
const CAGE_VERTICALS: usize = 4;

/// The frame's debug overlay: every static hitbox AABB in `world` as a 12-edge cage, plus the
/// player capsule as a line cage at `player_pos` (the interpolated draw position, so the cage
/// tracks the drawn bean, not the raw sim step).
pub fn debug_lines(world: &World, player_pos: Vec3) -> Vec<LineSegment> {
    let mut lines = Vec::with_capacity(world.statics.len() * 12 + 2 * RING_SEGMENTS + CAGE_VERTICALS);
    for aabb in &world.statics {
        aabb_lines(aabb, &mut lines);
    }
    capsule_lines(player_pos, &mut lines);
    lines
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

/// The player capsule as a cage: a ring at each cap equator (where the hemispheres meet the wall,
/// `half-segment` above and below the centre) and a few verticals spanning the wall between them.
/// Enough to read position, radius, and height at a glance; the brief's full silhouette lives in
/// the rendered capsule mesh itself.
fn capsule_lines(center: Vec3, out: &mut Vec<LineSegment>) {
    let half_segment = PLAYER_SEGMENT * 0.5;
    let on_ring = |j: usize, y: f32| {
        let lon = TAU * (j as f32 / RING_SEGMENTS as f32);
        center + Vec3::new(PLAYER_RADIUS * lon.cos(), y, PLAYER_RADIUS * lon.sin())
    };
    for j in 0..RING_SEGMENTS {
        for y in [-half_segment, half_segment] {
            out.push(LineSegment { start: on_ring(j, y), end: on_ring(j + 1, y), color: CAPSULE_COLOR });
        }
    }
    for j in 0..CAGE_VERTICALS {
        let j = j * (RING_SEGMENTS / CAGE_VERTICALS);
        out.push(LineSegment {
            start: on_ring(j, -half_segment),
            end: on_ring(j, half_segment),
            color: CAPSULE_COLOR,
        });
    }
}
