//! The editor's render pass: the scene-independent GPU residency and the per-frame draw, split from
//! the frame loop (`crate::main`) to keep that file at the frame loop's altitude.
//!
//! Composition only, per the HLD's application layer: the authored scene (the editable placements in
//! `crate::loaded`) is derived into runtime arrays (`crate::render_scene`), wok-mesh uploads them,
//! wok-render draws exactly the list it is handed, and the chunk-origin composition happens here
//! because the render contract makes it caller policy. The scene draws into the editor-well rect:
//! wok-render's viewport is set to that rect (the egui CentralPanel rect in physical pixels) and the
//! camera's aspect comes from it, so the 3D sits centred and undistorted inside the panel rather than
//! filling the window and reading off-centre behind the chrome. The egui chrome then paints last into
//! the same encoder, framing the viewport. When no scene is open there is no render residency, so the
//! frame clears to the editor surface - the empty well - full-window (the opaque chrome panels paint
//! over the margins).
//!
//! [`Gpu`] is the residency created once a device exists: the renderer, one mesh per unit primitive
//! (shared by every placement), and one terrain mesh per loaded chunk, rebuilt when a scene loads.

use std::collections::BTreeMap;

use glam::{Mat4, Vec3};
use wok_content::ChunkStore;
use wok_mesh::{MeshGpu, primitive_mesh};
use wok_platform::{Platform, gfx};
use wok_render::{Camera, RenderItem, Renderer, ViewportRect};
use wok_scene::{ChunkCoord, Primitive, SurfaceTag, VisibleItem};

use crate::camera::FlyCamera;
use crate::gui::Gui;
use crate::render_scene::{RenderScene, chunk_origin};

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

/// The editor-well rect as a wok-render viewport, in physical pixels, or `None` (full target) when
/// the rect is not a usable positive box. `rect` is in egui points (the CentralPanel rect captured
/// from the chrome, `view::chrome`); physical pixels are points times `pixels_per_point`. The
/// point-space rect is the shared source - cursor-to-ray picking (3b) maps the cursor against the
/// same well rect - so this is the one place the points-to-pixels scaling lives.
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

/// Scene-independent GPU residency, created in `init` once a device exists: the renderer sized to the
/// surface, one uploaded mesh per unit primitive (shared by every placement), and one terrain mesh
/// per loaded chunk (empty until a scene loads).
pub struct Gpu {
    pub renderer: Renderer,
    primitives: Vec<MeshGpu>,
    terrain: BTreeMap<ChunkCoord, MeshGpu>,
}

impl Gpu {
    /// Build the scene-independent residency: the renderer sized to the surface and one mesh per unit
    /// primitive. Terrain starts empty; [`set_terrain`](Gpu::set_terrain) fills it when a scene loads.
    pub fn new(platform: &Platform) -> Gpu {
        let config = &platform.surface_config;
        let renderer = Renderer::new(&platform.device, config.format, config.width, config.height);
        let primitives = PRIMITIVES
            .iter()
            .map(|&p| MeshGpu::upload(&platform.device, &primitive_mesh(p)))
            .collect();
        Gpu { renderer, primitives, terrain: BTreeMap::new() }
    }

    /// Rebuild the terrain meshes from a store: one upload per loaded chunk that has terrain. Called
    /// when a scene's content loads; the terrain cannot change without a scene reload in this bite, so
    /// the GPU meshes outlive the cheap per-edit re-derivation of the visible items. Cheap to rebuild
    /// wholesale at this scale.
    pub fn set_terrain(&mut self, platform: &Platform, store: &ChunkStore) {
        self.terrain.clear();
        for (coord, runtime) in store.iter_loaded() {
            if let Some(mesh) = runtime.terrain_mesh.as_ref() {
                self.terrain.insert(coord, MeshGpu::upload(&platform.device, mesh));
            }
        }
    }

    /// Drop every terrain mesh - the scene closed, so there is nothing to draw.
    pub fn clear_terrain(&mut self) {
        self.terrain.clear();
    }
}

/// Draw the frame: the open scene (terrain, placeholder prefab shapes, and the scene's lighting), or
/// the cleared editor surface when no scene is open, with the egui chrome painted over it into the
/// same encoder. Free function (not a method) so the frame loop can hand it disjoint borrows of the
/// app's fields. `editor_bg` is the no-scene clear colour (the active theme's editor surface).
pub fn draw(
    platform: &mut Platform,
    gpu: &mut Gpu,
    scene: Option<&RenderScene>,
    camera: FlyCamera,
    editor_rect: egui::Rect,
    editor_bg: egui::Color32,
    gui: &mut Gui,
    ui_output: egui::FullOutput,
) {
    // Scale the editor rect (egui points) to physical pixels with the same pixels-per-point egui
    // paints the chrome with, so the viewport lines up with the panel exactly.
    let pixels_per_point = ui_output.pixels_per_point;
    let Some(mut frame) = gfx::begin_frame(platform) else { return };

    // The frame's colour target, depth buffer, and viewport are one size, taken from the texture just
    // acquired - never from a separately tracked window size that can race ahead of the surface
    // mid-resize (sharp-edges 1). Size the depth to match before the forward pass binds them together;
    // `resize` is a no-op when the size is unchanged, so this costs nothing in steady state.
    let size = frame.size();
    gpu.renderer.resize(&platform.device, size.0, size.1);

    match scene {
        Some(scene) => {
            let Gpu { renderer, primitives, terrain } = gpu;
            // Confine the 3D to the editor-well rect and take the camera's aspect from it, so the view
            // sits centred and undistorted in the panel instead of stretched to the window. A
            // degenerate rect (before the first chrome, or a collapsed panel) falls back to the full
            // window.
            let viewport = editor_viewport(editor_rect, pixels_per_point);
            renderer.set_viewport(viewport);
            let aspect = match viewport {
                Some(vp) => vp.width / vp.height,
                None => size.0 as f32 / size.1.max(1) as f32,
            };
            let cam = Camera {
                view_proj: camera.view_proj(aspect, scene.far_plane()),
                eye: camera.position,
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
                &platform.device,
                &platform.queue,
                &mut frame.encoder,
                &frame.view,
                &cam,
                &scene.light,
                scene.scene_bounds(),
                &items,
            );
        }
        None => {
            // No scene: clear to the active theme's editor surface (the empty well). The surface is
            // sRGB and wgpu reads the clear value as linear, so decode through Rgba.
            let clear = egui::Rgba::from(editor_bg);
            frame.clear(clear.r().into(), clear.g().into(), clear.b().into());
        }
    }

    gui.paint(platform, &mut frame.encoder, &frame.view, ui_output, size);
    frame.finish(platform);
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
