use crate::platform::Platform;

/// Clear the screen to a solid color. Returns the texture view and encoder for further use,
/// or None if the surface frame could not be acquired.
pub fn begin_frame(platform: &Platform) -> Option<Frame> {
    let output = platform.surface.get_current_texture().ok()?;
    let view = output
        .texture
        .create_view(&wgpu::TextureViewDescriptor::default());
    let encoder = platform
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("wok_platform_frame_encoder"),
        });

    Some(Frame {
        output,
        view,
        encoder,
    })
}

/// An in-progress frame. Call `clear()` to set a background color, then `finish()` to present.
pub struct Frame {
    pub output: wgpu::SurfaceTexture,
    pub view: wgpu::TextureView,
    pub encoder: wgpu::CommandEncoder,
}

impl Frame {
    /// Clear the frame to a solid RGBA color (values 0.0 to 1.0).
    pub fn clear(&mut self, r: f64, g: f64, b: f64) {
        self.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("wok_platform_clear_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color { r, g, b, a: 1.0 }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
    }

    /// Submit the frame's commands and present to the screen.
    ///
    /// Takes `&mut Platform` to record that a real frame has presented: the window is created
    /// hidden, and the runner reveals it after the first present so the user never sees the OS's
    /// blank client area. A skipped frame - `begin_frame` returned `None` - never reaches here,
    /// so the flag marks actual pixels rather than an attempt. Call sites already pass
    /// `ctx.platform` (a `&mut Platform`), so they reborrow with no edit.
    pub fn finish(self, platform: &mut Platform) {
        platform
            .queue
            .submit(std::iter::once(self.encoder.finish()));
        self.output.present();
        platform.presented = true;
    }
}
