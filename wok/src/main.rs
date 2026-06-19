//! wok: the engine's reference editor application.
//!
//! Cleared to a minimal shell for a redesign rebuild. Read `designs/sharp-edges.md` before adding the
//! render path, the chrome, or resize handling - it lists the traps this rebuild must not rediscover
//! (atomic frame sizing from the acquired texture, the Vulkan teardown chain, the egui panel/flush
//! and full-height-cell patterns, and snapshot-driven visual verification).
//!
//! This stub opens a window and clears it each frame through the platform's frame loop:
//! `gfx::begin_frame` -> draw -> `Frame::finish`. The editor is rebuilt on top from here, re-adding
//! the engine crates (wok-render, wok-scene, wok-content, wok-mesh, wok-light) and the egui chrome
//! (egui / egui-wgpu / egui-winit 0.31, plus the egui_kittest + accesskit snapshot dev-dependencies)
//! to `Cargo.toml` as each is used.

use wok_platform::{App, Desc, FrameCtx, Platform, gfx};

struct Editor;

impl App for Editor {
    fn init(&mut self, _platform: &Platform) {}

    fn frame(&mut self, ctx: &mut FrameCtx) {
        // Acquire, clear, present. When the 3D view returns, size its depth buffer from `frame.size()`
        // (the acquired surface texture), never from a separately tracked window size - see
        // `designs/sharp-edges.md` section 1, the resize-divergence trap.
        let Some(mut frame) = gfx::begin_frame(ctx.platform) else { return };
        frame.clear(0.09, 0.10, 0.12);
        frame.finish(ctx.platform);
    }

    fn cleanup(&mut self, _platform: &Platform) {}
}

fn main() {
    wok_platform::run(Editor, Desc { title: "wok", width: 0, height: 0, vsync: true });
}
