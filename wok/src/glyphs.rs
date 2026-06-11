//! Painter-drawn glyphs: the page toggles and the tree's row icons.
//!
//! Drawn with egui's painter rather than icon-font codepoints: the ASCII source rule rules out
//! unicode glyph literals, an icon font would be a new dependency, and the handful of marks
//! needed here read fine when painted directly. The page toggles stay line art (they sit at 18px
//! where strokes read cleanly); the tree icons are filled silhouettes, because at the tree's 11px
//! a hairline outline dissolves into the row while a filled shape still reads as a mark - the
//! same reason Zed's project panel fills its file icons. Every glyph draws inside the rect it is
//! given and takes its color from the caller, so theming flows through untouched.

use egui::epaint::StrokeKind;
use egui::{Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};
use wok_scene::Primitive;

use crate::pages::Page;

/// Stroke width for all glyph line art, in points: hairline-plus, matching egui's widget strokes.
const WIDTH: f32 = 1.2;

/// A page's status-bar glyph, drawn inside `rect`.
pub fn page(painter: &Painter, rect: Rect, page: Page, color: egui::Color32) {
    let stroke = Stroke::new(WIDTH, color);
    let r = rect.shrink(rect.width() * 0.2);
    match page {
        // Scene: an outline/tree - a root line with two indented children.
        Page::Scene => {
            let x0 = r.left();
            let indent = r.width() * 0.35;
            for (i, y) in [r.top(), r.center().y, r.bottom()].into_iter().enumerate() {
                let x = if i == 0 { x0 } else { x0 + indent };
                painter.line_segment([pos2(x, y), pos2(r.right(), y)], stroke);
            }
            // The spine joining the children to the root.
            painter.line_segment([pos2(x0 + indent * 0.5, r.top()), pos2(x0 + indent * 0.5, r.bottom())], stroke);
        }
        // Prefabs: a library - a 2x2 grid of small boxes.
        Page::Prefabs => {
            let cell = vec2(r.width() * 0.42, r.height() * 0.42);
            for (fx, fy) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
                let min = pos2(
                    r.left() + fx * (r.width() - cell.x),
                    r.top() + fy * (r.height() - cell.y),
                );
                painter.rect_stroke(Rect::from_min_size(min, cell), 1.0, stroke, StrokeKind::Inside);
            }
        }
        // Scan: a magnifier - a circle with a handle toward the lower right.
        Page::Scan => {
            let radius = r.width() * 0.32;
            let center = r.center() - vec2(radius * 0.4, radius * 0.4);
            painter.circle_stroke(center, radius, stroke);
            let dir = vec2(1.0, 1.0).normalized();
            painter.line_segment([center + dir * radius, center + dir * (radius * 2.2)], stroke);
        }
    }
}

/// A placement row's kind glyph: the dominant primitive's filled silhouette, or a small dot when
/// the kind is unknown (missing prefab, mesh-only state).
pub fn kind(painter: &Painter, rect: Rect, kind: Option<Primitive>, color: egui::Color32) {
    let r = rect;
    match kind {
        Some(Primitive::Cube) => {
            painter.rect_filled(r.shrink(0.5), 2.0, color);
        }
        Some(Primitive::Ellipsoid) => {
            painter.circle_filled(r.center(), r.width() * 0.46, color);
        }
        Some(Primitive::Cylinder) => {
            // A tin: a tall rounded slab, rounder than the cube but not a pill.
            let wall = Rect::from_center_size(r.center(), vec2(r.width() * 0.72, r.height()));
            painter.rect_filled(wall, wall.width() * 0.38, color);
        }
        Some(Primitive::Capsule) => {
            // A stadium: fully rounded ends.
            let pill = Rect::from_center_size(r.center(), vec2(r.width() * 0.55, r.height()));
            painter.rect_filled(pill, pill.width() * 0.5, color);
        }
        Some(Primitive::Plane) => {
            // A ground quad seen at a slant: a flat parallelogram.
            let y = r.center().y;
            let lean = r.width() * 0.22;
            painter.add(Shape::convex_polygon(
                vec![
                    pos2(r.left() + lean, y - lean),
                    pos2(r.right(), y - lean),
                    pos2(r.right() - lean, y + lean),
                    pos2(r.left(), y + lean),
                ],
                color,
                Stroke::NONE,
            ));
        }
        None => {
            painter.circle_filled(r.center(), r.width() * 0.16, color);
        }
    }
}

/// A chunk row's folder glyph: filled tab-and-body, the project-panel directory mark.
pub fn folder(painter: &Painter, rect: Rect, color: egui::Color32) {
    let r = rect;
    let tab = Rect::from_min_size(r.min, vec2(r.width() * 0.45, r.height() * 0.32));
    painter.rect_filled(tab, 1.0, color);
    let body = Rect::from_min_max(pos2(r.left(), r.top() + r.height() * 0.18), r.max);
    painter.rect_filled(body, 1.5, color);
}

/// A collapsible row's disclosure chevron: a small filled triangle, right when closed, down when
/// open.
pub fn chevron(painter: &Painter, rect: Rect, open: bool, color: egui::Color32) {
    let c = rect.center();
    let s = rect.width() * 0.32;
    let points: [Pos2; 3] = if open {
        [pos2(c.x - s, c.y - s * 0.5), pos2(c.x + s, c.y - s * 0.5), pos2(c.x, c.y + s * 0.7)]
    } else {
        [pos2(c.x - s * 0.5, c.y - s), pos2(c.x + s * 0.7, c.y), pos2(c.x - s * 0.5, c.y + s)]
    };
    painter.add(Shape::convex_polygon(points.to_vec(), color, Stroke::NONE));
}
