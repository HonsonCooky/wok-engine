//! The editor's render pass: GPU residency and the per-frame draw, split from `crate::app` to keep
//! that file at the frame loop's altitude.
//!
//! Composition only, per the HLD's application layer: the authored model flows through wok-content's
//! store into runtime arrays, wok-mesh uploads them, wok-render draws exactly the list it is handed,
//! and the chunk-origin composition happens here because the render contract makes it caller policy.
//! egui paints last into the same encoder. [`Gpu`] is the residency created once a device exists;
//! [`EditorApp::render`] builds the render list each frame and hands it to the renderer.

use std::collections::BTreeMap;

use glam::{Mat4, Vec3};
use wok_mesh::{MeshGpu, primitive_mesh};
use wok_platform::{FrameCtx, Platform, gfx};
use wok_render::{Camera, DepthMode, LineSegment, RenderItem, Renderer};
use wok_scene::{ChunkCoord, Primitive, SurfaceTag, VisibleItem};

use crate::app::EditorApp;
use crate::gui::Gui;
use crate::lines;
use crate::model::{EditorModel, chunk_origin, scene_bounds};
use crate::pick;
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
/// data (the engine only carries the tag).
fn surface_color(surface: Option<&SurfaceTag>) -> Vec3 {
    match surface.map(SurfaceTag::as_str) {
        Some("grass") => Vec3::new(0.40, 0.60, 0.35),
        Some("wood") => Vec3::new(0.60, 0.42, 0.24),
        Some("stone") => Vec3::new(0.55, 0.55, 0.58),
        Some("metal") => Vec3::new(0.80, 0.45, 0.25),
        _ => Vec3::new(0.70, 0.70, 0.70),
    }
}

/// GPU residency, created in `init` once a device exists: the renderer, the egui integration, one
/// uploaded mesh per unit primitive (shared by every placement), and one terrain mesh per chunk.
pub(crate) struct Gpu {
    pub(crate) renderer: Renderer,
    pub(crate) gui: Gui,
    primitives: Vec<MeshGpu>,
    pub(crate) terrain: BTreeMap<ChunkCoord, MeshGpu>,
}

impl Gpu {
    /// Build the GPU residency from the loaded model: the renderer sized to the surface, the egui
    /// integration with the editor theme applied, one mesh per unit primitive, and one terrain mesh
    /// per loaded chunk.
    pub(crate) fn new(platform: &Platform, model: &EditorModel) -> Gpu {
        let config = &platform.surface_config;
        let renderer = Renderer::new(&platform.device, config.format, config.width, config.height);

        let primitives = PRIMITIVES
            .iter()
            .map(|&p| MeshGpu::upload(&platform.device, &primitive_mesh(p)))
            .collect();
        let mut terrain = BTreeMap::new();
        for (coord, runtime) in model.store.iter_loaded() {
            if let Some(mesh) = runtime.terrain_mesh.as_ref() {
                terrain.insert(coord, MeshGpu::upload(&platform.device, mesh));
            }
        }
        let gui = Gui::new(platform);
        theme::apply(&gui.ctx);
        Gpu { renderer, gui, primitives, terrain }
    }
}

impl EditorApp {
    /// Each selected placement's classified colliders as an x-ray cage.
    fn selection_lines(&self) -> Vec<LineSegment> {
        let mut out = Vec::new();
        for sel in self.model.selection.iter() {
            if let Some(placement) = self.model.placement(sel)
                && let Some(prefab) = self.model.prefabs.get(&placement.prefab)
            {
                for collider in pick::placement_colliders(prefab, placement, chunk_origin(sel.coord)) {
                    lines::collider_lines(&collider, lines::SELECTION_COLOR, &mut out);
                }
            }
        }
        out
    }

    pub(crate) fn render(&mut self, ctx: &mut FrameCtx, ui_output: Option<egui::FullOutput>) {
        let far = self.far_plane();
        let cage = self.selection_lines();
        let model = &self.model;
        let Some(gpu) = self.gpu.as_mut() else { return };
        let Gpu { renderer, gui, primitives, terrain } = gpu;

        let aspect = self.size.0 as f32 / self.size.1.max(1) as f32;
        let camera = Camera { view_proj: self.camera.view_proj(aspect, far), eye: self.camera.position };

        let mut items: Vec<RenderItem> = Vec::new();
        for (coord, runtime) in model.store.iter_loaded() {
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
                    // placements simply do not draw yet.
                    VisibleItem::Mesh { .. } => {}
                }
            }
        }
        self.draw_items = items.len();

        let Some(mut frame) = gfx::begin_frame(ctx.platform) else { return };
        renderer.render(
            &ctx.platform.device,
            &ctx.platform.queue,
            &mut frame.encoder,
            &frame.view,
            &camera,
            &self.light,
            scene_bounds(&model.store),
            &items,
        );
        if !cage.is_empty() {
            renderer.render_lines(
                &ctx.platform.device,
                &ctx.platform.queue,
                &mut frame.encoder,
                &frame.view,
                &cage,
                DepthMode::XRay,
            );
        }
        if let Some(output) = ui_output {
            gui.paint(ctx.platform, &mut frame.encoder, &frame.view, output, self.size);
        }
        frame.finish(ctx.platform);
    }
}
