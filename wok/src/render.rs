//! The editor's render pass: GPU residency and the per-frame draw, split from `crate::app` to keep
//! that file at the frame loop's altitude.
//!
//! Composition only, per the HLD's application layer: the authored model flows through wok-content's
//! store into runtime arrays (`crate::scene`), wok-mesh uploads them, wok-render draws exactly the
//! list it is handed, and the chunk-origin composition happens here because the render contract makes
//! it caller policy. The scene is drawn full-window; the egui chrome paints last into the same
//! encoder, and the editor-area panel is transparent, so the chrome frames the viewport the scene
//! fills behind it. (Confining the 3D pass to the editor-area rect needs a viewport entry point on
//! wok-render, a later cross-cutting change.) When no project is open there is no scene, so the frame
//! clears to the editor surface - the empty viewport.
//!
//! [`Gpu`] is the residency created once a device exists: the renderer, one mesh per unit primitive
//! (shared by every placement), and one terrain mesh per loaded chunk, rebuilt when content loads or
//! a chunk hot-reloads.

use std::collections::BTreeMap;

use glam::{Mat4, Vec3};
use wok_content::ChunkStore;
use wok_mesh::{MeshGpu, primitive_mesh};
use wok_platform::{FrameCtx, Platform, gfx};
use wok_render::{Camera, RenderItem, Renderer};
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
        let Some(gpu) = self.gpu.as_mut() else { return };
        let Some(mut frame) = gfx::begin_frame(ctx.platform) else { return };

        match self.scene.as_ref() {
            Some(scene) => {
                let Gpu { renderer, primitives, terrain } = gpu;
                let aspect = size.0 as f32 / size.1.max(1) as f32;
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
