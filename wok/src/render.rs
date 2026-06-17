//! The editor's render pass: GPU residency and the per-frame draw, split from `crate::app` to keep
//! that file at the frame loop's altitude.
//!
//! Composition only, per the HLD's application layer: the authored model flows through wok-content's
//! store into runtime arrays (`crate::scene`), wok-mesh uploads them, wok-render draws exactly the
//! list it is handed, and the chunk-origin composition happens here because the render contract makes
//! it caller policy. The scene draws into the editor-area rect: wok-render's viewport is set to that
//! rect (the egui CentralPanel rect in physical pixels) and the camera's aspect comes from it, so
//! the 3D sits centred and undistorted inside the panel rather than filling the window and reading
//! off-centre behind the chrome. The egui chrome then paints last into the same encoder, framing the
//! viewport. When no project is open there is no scene, so the frame clears to the editor surface -
//! the empty viewport - full-window (the opaque chrome panels paint over the margins).
//!
//! [`Gpu`] is the residency created once a device exists: the renderer, one mesh per unit primitive
//! (shared by every placement), and one terrain mesh per loaded chunk, rebuilt when content loads or
//! a chunk hot-reloads.

use std::collections::BTreeMap;

use glam::{Mat4, Vec3};
use wok_content::ChunkStore;
use wok_mesh::{MeshGpu, primitive_mesh};
use wok_platform::{FrameCtx, Platform, gfx};
use wok_render::{Camera, RenderItem, Renderer, ViewportRect};
use wok_scene::{ChunkCoord, Primitive, SurfaceTag, VisibleItem};

use crate::app::EditorApp;
use crate::scene::chunk_origin;
use crate::theme;

const TERRAIN_COLOR: Vec3 = Vec3::new(0.40, 0.60, 0.35);

/// Draw order of the primitive mesh cache; `primitive_index` must match.
const PRIMITIVES: [Primitive; 5] =
    [Primitive::Cube, Primitive::Ellipsoid, Primitive::Cylinder, Primitive::Capsule, Primitive::Plane];

fn primitive_index(primitive: Primitive) -> usize {
    match primitive {
        Primitive::Cube => 0,
        Primitive::Ellipsoid => 1,
        Primitive::Cylinder => 2,
        Primitive::Capsule => 3,
        Primitive::Plane => 4,
    }
}

/// Flat base color for a placeholder by its surface tag; editor presentation policy, not engine
/// data (the engine only carries the tag). The same palette as taste, so authored content reads the
/// same in both.
fn surface_color(surface: Option<&SurfaceTag>) -> Vec3 {
    match surface.map(SurfaceTag::as_str) {
        Some("grass") => Vec3::new(0.40, 0.60, 0.35),
        Some("wood") => Vec3::new(0.60, 0.42, 0.24),
        Some("stone") => Vec3::new(0.55, 0.55, 0.58),
        Some("metal") => Vec3::new(0.80, 0.45, 0.25),
        _ => Vec3::new(0.70, 0.70, 0.70),
    }
}

/// The editor-area rect as a wok-render viewport, in physical pixels, or `None` (full target) when
/// the rect is not a usable positive box. `rect` is in egui points (the CentralPanel rect captured
/// from the chrome, `EditorApp::editor_rect`); physical pixels are points times `pixels_per_point`.
/// The point-space rect is the shared source - cursor-to-ray picking (3b) maps the cursor against
/// the same `editor_rect` - so this is the one place the points-to-pixels scaling lives.
fn editor_viewport(rect: egui::Rect, pixels_per_point: f32) -> Option<ViewportRect> {
    if !rect.is_finite() || rect.width() <= 0.0 || rect.height() <= 0.0 {
        return None;
    }
    // Round the pixel rect outward - floor the min corner, ceil the max corner - so the integer
    // viewport never falls a rounding pixel short of the panel and lets wok-render's clear colour
    // show as a seam where the 3D meets the chrome. wok-render's scissor floors each edge, so
    // feeding it whole-pixel edges reproduces exactly this rect, and any sub-pixel overshoot is
    // painted over by the opaque chrome (the nav panel, tab bar, status bar) drawn after the 3D.
    let min_x = (rect.min.x * pixels_per_point).floor();
    let min_y = (rect.min.y * pixels_per_point).floor();
    let max_x = (rect.max.x * pixels_per_point).ceil();
    let max_y = (rect.max.y * pixels_per_point).ceil();
    Some(ViewportRect { x: min_x, y: min_y, width: max_x - min_x, height: max_y - min_y })
}

/// GPU residency, created in `init` once a device exists: the renderer sized to the surface, one
/// uploaded mesh per unit primitive (shared by every placement), and one terrain mesh per loaded
/// chunk (empty until a project's content loads).
pub(crate) struct Gpu {
    pub(crate) renderer: Renderer,
    primitives: Vec<MeshGpu>,
    terrain: BTreeMap<ChunkCoord, MeshGpu>,
}

impl Gpu {
    /// Build the scene-independent residency: the renderer sized to the surface and one mesh per
    /// unit primitive. Terrain starts empty; [`set_terrain`](Gpu::set_terrain) fills it when content
    /// loads.
    pub(crate) fn new(platform: &Platform) -> Gpu {
        let config = &platform.surface_config;
        let renderer = Renderer::new(&platform.device, config.format, config.width, config.height);
        let primitives = PRIMITIVES
            .iter()
            .map(|&p| MeshGpu::upload(&platform.device, &primitive_mesh(p)))
            .collect();
        Gpu { renderer, primitives, terrain: BTreeMap::new() }
    }

    /// Rebuild the terrain meshes from the store: one upload per loaded chunk that has terrain.
    /// Called when a project's content loads and after a chunk hot-reloads (the heightmap may have
    /// changed); cheap to rebuild wholesale at this scale.
    pub(crate) fn set_terrain(&mut self, platform: &Platform, store: &ChunkStore) {
        self.terrain.clear();
        for (coord, runtime) in store.iter_loaded() {
            if let Some(mesh) = runtime.terrain_mesh.as_ref() {
                self.terrain.insert(coord, MeshGpu::upload(&platform.device, mesh));
            }
        }
    }

    /// Drop every terrain mesh - the project closed, so there is nothing to draw.
    pub(crate) fn clear_terrain(&mut self) {
        self.terrain.clear();
    }
}

impl EditorApp {
    /// Draw the frame: the open scene (terrain, placeholder prefab shapes, and the scene's lighting),
    /// or the cleared editor surface when no project is open, with the egui chrome painted over it
    /// into the same encoder.
    pub(crate) fn render(&mut self, ctx: &mut FrameCtx, ui_output: Option<egui::FullOutput>) {
        let camera_state = self.camera;
        let size = self.size;
        let editor_rect = self.editor_rect;
        // Scale the editor rect (egui points) to physical pixels with the same pixels-per-point egui
        // paints the chrome with, so the viewport lines up with the panel exactly.
        let pixels_per_point = ui_output.as_ref().map_or(1.0, |o| o.pixels_per_point);
        let Some(gpu) = self.gpu.as_mut() else { return };
        let Some(mut frame) = gfx::begin_frame(ctx.platform) else { return };

        match self.scene.as_ref() {
            Some(scene) => {
                let Gpu { renderer, primitives, terrain } = gpu;
                // Confine the 3D to the editor-area rect and take the camera's aspect from it, so the
                // view sits centred and undistorted in the panel instead of stretched to the window.
                // A degenerate rect (before the first chrome, or a collapsed panel) falls back to the
                // full window.
                let viewport = editor_viewport(editor_rect, pixels_per_point);
                renderer.set_viewport(viewport);
                let aspect = match viewport {
                    Some(vp) => vp.width / vp.height,
                    None => size.0 as f32 / size.1.max(1) as f32,
                };
                let camera = Camera {
                    view_proj: camera_state.view_proj(aspect, scene.far_plane()),
                    eye: camera_state.position,
                };
                let mut items: Vec<RenderItem> = Vec::new();
                for (coord, runtime) in scene.store.iter_loaded() {
                    let origin = Mat4::from_translation(chunk_origin(coord));
                    if let Some(mesh) = terrain.get(&coord) {
                        items.push(RenderItem { transform: origin, mesh, color: TERRAIN_COLOR, opacity: 1.0 });
                    }
                    for item in &runtime.visible {
                        match item {
                            VisibleItem::Primitive { primitive, transform, surface } => {
                                items.push(RenderItem {
                                    transform: origin * *transform,
                                    mesh: &primitives[primitive_index(*primitive)],
                                    color: surface_color(surface.as_ref()),
                                    opacity: 1.0,
                                });
                            }
                            // Named replacement meshes need the glTF loader (wok-mesh, later); their
                            // placements simply do not draw yet, the same as taste.
                            VisibleItem::Mesh { .. } => {}
                        }
                    }
                }
                renderer.render(
                    &ctx.platform.device,
                    &ctx.platform.queue,
                    &mut frame.encoder,
                    &frame.view,
                    &camera,
                    &scene.light,
                    scene.scene_bounds(),
                    &items,
                );
            }
            None => {
                // No project: clear to the active theme's editor surface (the empty viewport). The
                // surface is sRGB and wgpu reads the clear value as linear, so decode through Rgba.
                let editor_bg = self.gui.as_ref().map_or(egui::Color32::BLACK, |g| theme::palette(&g.ctx).editor_bg);
                let clear = egui::Rgba::from(editor_bg);
                frame.clear(clear.r().into(), clear.g().into(), clear.b().into());
            }
        }

        if let (Some(gui), Some(output)) = (self.gui.as_mut(), ui_output) {
            gui.paint(ctx.platform, &mut frame.encoder, &frame.view, output, size);
        }
        frame.finish(ctx.platform);
    }
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::editor_viewport;

    #[test]
    fn a_degenerate_rect_has_no_viewport() {
        assert!(editor_viewport(egui::Rect::NOTHING, 1.5).is_none());
        let zero_height = egui::Rect::from_min_max(egui::pos2(0.0, 10.0), egui::pos2(100.0, 10.0));
        assert!(editor_viewport(zero_height, 1.0).is_none());
    }

    #[test]
    fn a_whole_pixel_rect_passes_through_at_unit_scale() {
        let rect = egui::Rect::from_min_max(egui::pos2(216.0, 34.0), egui::pos2(1100.0, 680.0));
        let vp = editor_viewport(rect, 1.0).unwrap();
        assert_eq!((vp.x, vp.y, vp.width, vp.height), (216.0, 34.0, 884.0, 646.0));
    }

    #[test]
    fn a_subpixel_rect_rounds_outward_to_cover_the_panel() {
        // At 1.5x a panel at points (144.3 .. 480.7) x (20.2 .. 360.9) spans pixels
        // (216.45 .. 721.05) x (30.3 .. 541.35). Outward rounding floors the min corner (216, 30)
        // and ceils the max corner (722, 542), so the integer viewport fully covers the panel
        // instead of falling a fraction short and showing a clear-colour seam.
        let rect = egui::Rect::from_min_max(egui::pos2(144.3, 20.2), egui::pos2(480.7, 360.9));
        let vp = editor_viewport(rect, 1.5).unwrap();
        assert_eq!((vp.x, vp.y), (216.0, 30.0));
        assert_eq!((vp.width, vp.height), (722.0 - 216.0, 542.0 - 30.0));
    }
}
