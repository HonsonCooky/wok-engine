//! Painter-drawn glyphs: the page toggles and the tree's kind icons.
//!
//! Drawn with egui's painter from strokes and circles rather than icon-font codepoints: the ASCII
//! source rule rules out unicode glyph literals, an icon font would be a new dependency, and the
//! handful of marks needed here (a list, a box, a magnifier, five primitive silhouettes) read
//! fine at 16px as line art. Every glyph draws inside the rect it is given and takes its color
//! from the caller (the widget's interact visuals), so theming flows through untouched.

use egui::epaint::StrokeKind;
use egui::{Painter, Pos2, Rect, Stroke, pos2, vec2};
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

/// A placement row's kind glyph: the dominant primitive's silhouette, or a small dot when the
/// kind is unknown (missing prefab, mesh-only state).
pub fn kind(painter: &Painter, rect: Rect, kind: Option<Primitive>, color: egui::Color32) {
    let stroke = Stroke::new(WIDTH, color);
    let r = rect.shrink(rect.width() * 0.22);
    match kind {
        Some(Primitive::Cube) => {
            painter.rect_stroke(r, 1.0, stroke, StrokeKind::Inside);
        }
        Some(Primitive::Ellipsoid) => {
            painter.circle_stroke(r.center(), r.width() * 0.5, stroke);
        }
        Some(Primitive::Cylinder) => {
            // A tin: the wall, with the top cap hinted by a chord.
            let wall = Rect::from_center_size(r.center(), vec2(r.width() * 0.7, r.height()));
            painter.rect_stroke(wall, 2.0, stroke, StrokeKind::Inside);
            let y = wall.top() + wall.height() * 0.28;
            painter.line_segment([pos2(wall.left(), y), pos2(wall.right(), y)], stroke);
        }
        Some(Primitive::Capsule) => {
            // A stadium: the wall with fully rounded ends.
            let pill = Rect::from_center_size(r.center(), vec2(r.width() * 0.6, r.height()));
            painter.rect_stroke(pill, pill.width() * 0.5, stroke, StrokeKind::Inside);
        }
        Some(Primitive::Plane) => {
            // A ground quad seen at a slant: a flat parallelogram.
            let y = r.center().y;
            let lean = r.width() * 0.18;
            let quad: [Pos2; 4] = [
                pos2(r.left() + lean, y - lean),
                pos2(r.right(), y - lean),
                pos2(r.right() - lean, y + lean),
                pos2(r.left(), y + lean),
            ];
            for i in 0..4 {
                painter.line_segment([quad[i], quad[(i + 1) % 4]], stroke);
            }
        }
        None => {
            painter.circle_filled(r.center(), WIDTH, color);
        }
    }
}
