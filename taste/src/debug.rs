//! The hitbox overlay: line-cage geometry for the F1 diagnostic, in four cycled modes.
//!
//! Pure geometry building, no GPU: one call per frame turns the simulation's world into the
//! `LineSegment` list the renderer's debug line pass draws. The cages show what the fixed-step
//! loop actually collides with - world-space static colliders in their own true shapes (box cage,
//! sphere rings, cylinder rings-and-verticals) and the player's CYLINDER collider at its exact
//! dimensions - so a play-tester can see collision-vs-visual disagreements directly instead of
//! inferring them from bumps. The player cage is where the deliberate visual mismatch shows: the
//! drawn body is the bean (capsule mesh), the collider is the flat-bottomed cylinder, and this
//! cage is the collider truth - rings at the flat caps, verticals between, the same stroke the
//! static cylinders get. The skeleton must match the truth: a sphere drawn as its box would
//! reintroduce exactly the lie the round colliders removed.
//!
//! The modes answer different questions. Faces draws the drawn-shape cages depth-tested, so only
//! edges on faces the camera can see render: cage and surface compare face by face without x-ray
//! clutter. Visible draws the same set x-ray, the classic see-collision-through-geometry view.
//! All adds what the scene hides - trigger volumes and hitboxes whose placeholder never draws
//! (mesh-replaced states) - in their own color, so the invisible reads as different at a glance.
//! Which colliders count as drawn comes from the world's `statics_visible` (the slicer's
//! `also_visible`, carried through the reduction); the player cage rides every drawing mode.

use std::f32::consts::TAU;

use glam::{Quat, Vec3};
use wok_physics::Collider;
use wok_render::{DepthMode, LineSegment};
use wok_scene::Aabb;

use crate::constants::{PLAYER_HEIGHT, PLAYER_RADIUS};
use crate::world::World;

/// The hitbox overlay's modes, cycled by F1 in declaration order. Starts off: the overlay is a
/// diagnostic, not part of the game's look.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OverlayMode {
    #[default]
    Off,
    /// Drawn-shape collider cages, depth-tested: only edges on faces the camera can see render.
    Faces,
    /// Drawn-shape collider cages, x-ray: collision read through the geometry it describes.
    Visible,
    /// Every collider x-ray - trigger volumes and undrawn hitboxes included, in their own color.
    All,
}

impl OverlayMode {
    /// The next mode in the F1 cycle: Off -> Faces -> Visible -> All -> Off.
    pub fn next(self) -> OverlayMode {
        match self {
            OverlayMode::Off => OverlayMode::Faces,
            OverlayMode::Faces => OverlayMode::Visible,
            OverlayMode::Visible => OverlayMode::All,
            OverlayMode::All => OverlayMode::Off,
        }
    }

    /// The depth policy of a drawing mode: Faces exists to compare cage edges against the drawn
    /// surfaces, so it tests; the other modes read through the very geometry their cages describe,
    /// which is the point of an x-ray.
    pub fn depth(self) -> DepthMode {
        if self == OverlayMode::Faces { DepthMode::Tested } else { DepthMode::XRay }
    }
}

/// Hitbox cages: pure green at full saturation. The overlay draws on top of lit surface of any
/// color and must out-shout all of it; nothing in the scene palette is pure green (the terrain's
/// green is muted).
const HITBOX_COLOR: Vec3 = Vec3::new(0.0, 1.0, 0.0);

/// Cages for colliders nothing draws - trigger volumes and mesh-hidden hitboxes, the All mode's
/// addition: pure cyan, as loud as the hitbox green but unmistakably not it, so a cage with no
/// geometry inside reads as "invisible by design" rather than as a missing mesh.
const INVISIBLE_COLOR: Vec3 = Vec3::new(0.0, 1.0, 1.0);

/// The player collider cage: pure yellow at full saturation, readable over the bean's signal
/// orange even when the x-ray draw puts the cage on top of the body.
const PLAYER_CAGE_COLOR: Vec3 = Vec3::new(1.0, 1.0, 0.0);

/// The look-ahead reticle: a neutral grey, present without shouting (it is framing feedback, not a
/// gameplay crosshair).
const RETICLE_COLOR: Vec3 = Vec3::new(0.75, 0.75, 0.75);

/// Half-length of each reticle arm, in metres: a ~0.1m cross overall.
const RETICLE_HALF: f32 = 0.05;

/// Line segments per debug ring: enough that a 0.45m circle reads round, few enough that the
/// overlay stays obviously diagnostic.
const RING_SEGMENTS: usize = 16;

/// Verticals on the player cage, evenly spaced around the wall.
const CAGE_VERTICALS: usize = 4;

/// The frame's hitbox overlay for `mode`: the mode's collider set as cages in their own shapes,
/// plus the player's cylinder collider at `player_pos` (the interpolated draw position, so the
/// cage tracks the drawn bean, not the raw sim step). Off returns nothing. Missing
/// `statics_visible` entries read as drawn, so hand-built worlds that only fill `statics` still
/// overlay every collider.
pub fn overlay_lines(mode: OverlayMode, world: &World, player_pos: Vec3) -> Vec<LineSegment> {
    let mut lines = Vec::new();
    if mode == OverlayMode::Off {
        return lines;
    }
    let drawn_only = matches!(mode, OverlayMode::Faces | OverlayMode::Visible);
    for (i, collider) in world.statics.iter().enumerate() {
        let drawn = world.statics_visible.get(i).copied().unwrap_or(true);
        if drawn_only && !drawn {
            continue;
        }
        collider_lines(collider, if drawn { HITBOX_COLOR } else { INVISIBLE_COLOR }, &mut lines);
    }
    if mode == OverlayMode::All {
        for collider in &world.trigger_volumes {
            collider_lines(collider, INVISIBLE_COLOR, &mut lines);
        }
    }
    player_cage_lines(player_pos, &mut lines);
    lines
}

/// One collider's cage in its own true shape, in `color`.
fn collider_lines(collider: &Collider, color: Vec3, out: &mut Vec<LineSegment>) {
    match *collider {
        Collider::Aabb(ref aabb) => aabb_lines(aabb, color, out),
        Collider::Sphere { center, radius } => sphere_lines(center, radius, color, out),
        Collider::VertCylinder { center, radius, half_height } => {
            rings_and_verticals(center, radius, half_height, color, out);
        }
        Collider::Obb { center, half_extents, rotation } => {
            obb_lines(center, half_extents, rotation, color, out);
        }
    }
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
fn aabb_lines(aabb: &Aabb, color: Vec3, out: &mut Vec<LineSegment>) {
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
                out.push(LineSegment { start: corner(i), end: corner(i | bit), color });
            }
        }
    }
}

/// The 12 edges of an oriented box: the AABB stroke drawn in the box's own frame, each corner
/// rotated out by the collider's rotation. The cage must turn with the box - an axis-aligned cage
/// around a yawed crate would redraw the conservative margin the Obb collider just removed.
fn obb_lines(center: Vec3, half_extents: Vec3, rotation: Quat, color: Vec3, out: &mut Vec<LineSegment>) {
    let corner = |i: usize| {
        let local = Vec3::new(
            if i & 1 == 0 { -half_extents.x } else { half_extents.x },
            if i & 2 == 0 { -half_extents.y } else { half_extents.y },
            if i & 4 == 0 { -half_extents.z } else { half_extents.z },
        );
        center + rotation * local
    };
    for i in 0..8 {
        for bit in [1, 2, 4] {
            if i & bit == 0 {
                out.push(LineSegment { start: corner(i), end: corner(i | bit), color });
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
fn sphere_lines(center: Vec3, radius: f32, color: Vec3, out: &mut Vec<LineSegment>) {
    circle_lines(center, Vec3::X, Vec3::Z, radius, color, out);
    circle_lines(center, Vec3::X, Vec3::Y, radius, color, out);
    circle_lines(center, Vec3::Z, Vec3::Y, radius, color, out);
}

/// The player's cylinder collider as a cage: a ring at each FLAT cap (half the full height above
/// and below the centre - the collider's true extents, taller than the bean's cap equators) and a
/// few verticals spanning the wall - the same rings-and-verticals stroke every static cylinder
/// gets, because the player collides as exactly that shape now.
fn player_cage_lines(center: Vec3, out: &mut Vec<LineSegment>) {
    rings_and_verticals(center, PLAYER_RADIUS, PLAYER_HEIGHT * 0.5, PLAYER_CAGE_COLOR, out);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Segments in one box cage and in the player cage, restated from the strokes above so the
    /// set-selection tests can count cages instead of pattern-matching geometry.
    const BOX_CAGE: usize = 12;
    const PLAYER_CAGE: usize = 2 * RING_SEGMENTS + CAGE_VERTICALS;

    fn box_at(x: f32) -> Collider {
        Collider::from(Aabb::new(Vec3::new(x, 0.0, 0.0), Vec3::new(x + 1.0, 1.0, 1.0)))
    }

    /// Two statics (one drawn, one mesh-hidden) and one trigger volume: one collider of each
    /// overlay kind.
    fn overlay_world() -> World {
        World {
            statics: vec![box_at(0.0), box_at(10.0)],
            statics_visible: vec![true, false],
            trigger_volumes: vec![box_at(20.0)],
            ..World::default()
        }
    }

    fn count_color(lines: &[LineSegment], color: Vec3) -> usize {
        lines.iter().filter(|l| l.color == color).count()
    }

    #[test]
    fn f1_cycles_off_faces_visible_all_and_around() {
        let mut mode = OverlayMode::Off;
        let cycle: Vec<OverlayMode> = (0..5).map(|_| { mode = mode.next(); mode }).collect();
        assert_eq!(
            cycle,
            vec![OverlayMode::Faces, OverlayMode::Visible, OverlayMode::All, OverlayMode::Off, OverlayMode::Faces]
        );
    }

    #[test]
    fn off_draws_nothing_at_all() {
        assert!(overlay_lines(OverlayMode::Off, &overlay_world(), Vec3::ZERO).is_empty());
    }

    #[test]
    fn faces_and_visible_draw_only_the_drawn_shape_cages() {
        // Both modes select the same set - one drawn box plus the player - and differ only in
        // depth policy: Faces compares against drawn surfaces, Visible reads through them. A
        // hitbox whose placeholder never draws stays out of both: there is no face to compare
        // against and nothing visible to x-ray through.
        let world = overlay_world();
        for mode in [OverlayMode::Faces, OverlayMode::Visible] {
            let lines = overlay_lines(mode, &world, Vec3::ZERO);
            assert_eq!(lines.len(), BOX_CAGE + PLAYER_CAGE, "{mode:?} draws one box and the player");
            assert_eq!(count_color(&lines, INVISIBLE_COLOR), 0, "{mode:?} never shows the invisible");
        }
        assert_eq!(OverlayMode::Faces.depth(), DepthMode::Tested);
        assert_eq!(OverlayMode::Visible.depth(), DepthMode::XRay);
        assert_eq!(OverlayMode::All.depth(), DepthMode::XRay);
    }

    #[test]
    fn all_adds_the_invisible_in_its_own_color() {
        // Every collider cages: the drawn box in hitbox green, the mesh-hidden hitbox and the
        // trigger volume in the invisible color, so what the scene hides reads as different.
        let lines = overlay_lines(OverlayMode::All, &overlay_world(), Vec3::ZERO);
        assert_eq!(lines.len(), 3 * BOX_CAGE + PLAYER_CAGE);
        assert_eq!(count_color(&lines, HITBOX_COLOR), BOX_CAGE);
        assert_eq!(count_color(&lines, INVISIBLE_COLOR), 2 * BOX_CAGE);
        assert_eq!(count_color(&lines, PLAYER_CAGE_COLOR), PLAYER_CAGE);
    }

    #[test]
    fn a_hand_built_world_without_visibility_flags_draws_every_static() {
        // Test worlds fill `statics` and nothing else; missing flags must read as drawn, not
        // silently hide the colliders the overlay exists to show.
        let world = World { statics: vec![box_at(0.0), box_at(5.0)], ..World::default() };
        let lines = overlay_lines(OverlayMode::Visible, &world, Vec3::ZERO);
        assert_eq!(count_color(&lines, HITBOX_COLOR), 2 * BOX_CAGE);
    }
}
