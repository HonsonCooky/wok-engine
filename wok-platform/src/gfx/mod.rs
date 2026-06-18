use crate::platform::Platform;

/// Begin one frame: acquire the surface texture, a view onto it, and a command encoder, or `None`
/// if the surface could not hand out a texture this frame (the caller skips the frame).
///
/// This is the one place a frame is acquired, so it is where size consistency is established. The
/// returned [`Frame::size`] is the acquired texture's real size; the caller sizes its depth buffer
/// and viewport from that, never from a separately tracked window size. The two cannot then disagree
/// mid-resize: the window's reported size races ahead of the surface (the surface is reconfigured
/// only on the lagging `Resized` event), so a frame sized from the window can bind a depth buffer of
/// one size against a colour target of another - the "attachments have differing sizes" validation
/// error. Sizing everything from the acquired texture removes the second clock.
///
/// A `Lost`/`Outdated` surface (typically mid-resize) is reconfigured to its current size and the
/// texture re-acquired once; if it still will not produce one, the frame is skipped. No
/// `SurfaceTexture` is held across the reconfigure, so this never strands an unpresented frame for
/// surface teardown to trip over.
pub fn begin_frame(platform: &mut Platform) -> Option<Frame> {
    let output = match platform.surface.get_current_texture() {
        Ok(output) => output,
        Err(wgpu::SurfaceError::Outdated | wgpu::SurfaceError::Lost) => {
            platform
                .surface
                .configure(&platform.device, &platform.surface_config);
            platform.surface.get_current_texture().ok()?
        }
        Err(_) => return None,
    };
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
    /// The acquired surface texture's size in physical pixels: the one authoritative size for this
    /// frame. Size the depth buffer and the viewport from it, so the colour target, depth, and
    /// viewport are one size taken together - see [`begin_frame`].
    pub fn size(&self) -> (u32, u32) {
        (self.output.texture.width(), self.output.texture.height())
    }

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
