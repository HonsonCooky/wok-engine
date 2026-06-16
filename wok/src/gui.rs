//! egui plumbing: event translation, per-frame run, and painting into the frame's encoder.
//!
//! The UI composes onto the platform surfaces the engine already owns: egui-winit translates the
//! raw `WindowEvent`s wok-platform forwards through `App::on_window_event`, and egui-wgpu paints
//! into the same surface view and encoder wok-render's passes just used, as the frame's final
//! pass (color load, no depth - the UI sits over everything). No engine crate sees any of this;
//! egui is a wok-application dependency only.
//!
//! Theme: the editor follows the OS light/dark (`crate::theme` styles both and sets System). The
//! window's startup theme seeds egui here via `window.theme()`, and egui-winit feeds later
//! `ThemeChanged` events through, so the chrome switches live with the desktop.

use wok_platform::Platform;
use wok_platform::wgpu;
use wok_platform::winit::event::WindowEvent;
use wok_platform::winit::window::Window;

pub struct Gui {
    pub ctx: egui::Context,
    state: egui_winit::State,
    renderer: egui_wgpu::Renderer,
}

impl Gui {
    pub fn new(platform: &Platform) -> Gui {
        let ctx = egui::Context::default();
        let state = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            &platform.window,
            Some(platform.window.scale_factor() as f32),
            platform.window.theme(),
            Some(platform.device.limits().max_texture_dimension_2d as usize),
        );
        let renderer = egui_wgpu::Renderer::new(
            &platform.device,
            platform.surface_config.format,
            None, // the UI pass carries no depth attachment
            1,    // surface is not multisampled
            false,
        );
        Gui { ctx, state, renderer }
    }

    /// Feed one raw window event to egui. Called from `App::on_window_event`, before
    /// wok-platform's input collector sees it; the collector still records everything, and the
    /// frame loop decides what to ignore via `wants_pointer_input` / `wants_keyboard_input`.
    pub fn on_event(&mut self, window: &Window, event: &WindowEvent) {
        let _ = self.state.on_window_event(window, event);
    }

    /// Run one UI frame: gather egui's input, build the UI, and hand egui's platform requests
    /// (cursor icon, clipboard) back to winit. The returned output is painted later in the frame
    /// by [`Gui::paint`].
    pub fn run(&mut self, window: &Window, build: impl FnMut(&egui::Context)) -> egui::FullOutput {
        let raw_input = self.state.take_egui_input(window);
        let mut output = self.ctx.run(raw_input, build);
        let platform_output = std::mem::take(&mut output.platform_output);
        self.state.handle_platform_output(window, platform_output);
        output
    }

    /// Paint the UI over the frame: tessellate, sync textures and buffers, and record the final
    /// color-load pass into the caller's encoder.
    pub fn paint(
        &mut self,
        platform: &Platform,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        output: egui::FullOutput,
        size: (u32, u32),
    ) {
        let pixels_per_point = output.pixels_per_point;
        let primitives = self.ctx.tessellate(output.shapes, pixels_per_point);
        for (id, delta) in &output.textures_delta.set {
            self.renderer.update_texture(&platform.device, &platform.queue, *id, delta);
        }
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [size.0.max(1), size.1.max(1)],
            pixels_per_point,
        };
        // Widget-only UIs return no extra command buffers (those serve paint callbacks); submit
        // whatever arrives so the contract holds if a callback ever appears.
        let user_buffers = self.renderer.update_buffers(
            &platform.device,
            &platform.queue,
            encoder,
            &primitives,
            &screen,
        );
        platform.queue.submit(user_buffers);

        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("wok_gui_pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                })
                .forget_lifetime();
            self.renderer.render(&mut pass, &primitives, &screen);
        }

        for id in &output.textures_delta.free {
            self.renderer.free_texture(id);
        }
    }
}
