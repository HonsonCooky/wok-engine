use cpal::traits::{DeviceTrait, HostTrait};
use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

/// Description of the application window.
///
/// Pass `width: 0` or `height: 0` to opt into auto-sizing: the platform queries
/// the primary monitor at startup and picks a window that's roughly 75% of the
/// monitor's logical dimensions, clamped to comfortable bounds. Useful for
/// authoring tools where "one step down from native" is a more sensible default
/// than a fixed 1024x720 that's tiny on 4K and oversized on a 1366x768 laptop.
pub struct Desc {
    pub title: &'static str,
    pub width: u32,
    pub height: u32,
    pub vsync: bool,
}

/// Holds the initialized platform resources: window, GPU device, and audio device.
pub struct Platform {
    pub window: Arc<Window>,
    pub instance: wgpu::Instance,
    pub surface: wgpu::Surface<'static>,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface_config: wgpu::SurfaceConfiguration,
    pub audio_device: cpal::Device,
    pub audio_config: cpal::SupportedStreamConfig,
    supported_present_modes: Vec<wgpu::PresentMode>,
    /// Set by [`crate::gfx::Frame::finish`] the first time a frame actually presents. The runner
    /// watches it to reveal the window after that first real frame (the window is created hidden),
    /// so the user never sees the OS's blank client area. Stays true; the runner reveals once.
    pub(crate) presented: bool,
}

impl Platform {
    pub fn reconfigure_surface(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.surface_config.width = width;
            self.surface_config.height = height;
            self.surface.configure(&self.device, &self.surface_config);
        }
    }

    pub fn set_vsync(&mut self, vsync: bool) {
        self.surface_config.present_mode = pick_present_mode(vsync, &self.supported_present_modes);
        self.surface.configure(&self.device, &self.surface_config);
    }

    /// Initialize platform resources (window, GPU device, audio device) for `desc`. Call this
    /// from inside an `ActiveEventLoop` callback (`resumed` or `user_event`). Returns a ready
    /// `Platform`; the caller is responsible for keeping the event loop alive.
    ///
    /// This is what `wok_platform::run` uses internally. It's exposed so consumers that want to own
    /// their own event loop (e.g. tools that open and close windows on demand) can construct
    /// a Platform whenever they need one.
    ///
    /// # Panics
    /// Panics on any unrecoverable failure: window creation, GPU adapter discovery, device
    /// initialization, or audio device access.
    pub fn init(event_loop: &ActiveEventLoop, desc: &Desc) -> Self {
        let (width, height) = if desc.width == 0 || desc.height == 0 {
            auto_window_size(event_loop)
        } else {
            (desc.width, desc.height)
        };
        let window_attrs = Window::default_attributes()
            .with_title(desc.title)
            .with_inner_size(winit::dpi::LogicalSize::new(width, height))
            // Start hidden so the OS never shows the default (blank) client area before the first
            // frame is drawn. The runner reveals the window after the first frame presents (see
            // `resumed` and the redraw handler), so the first thing on screen is a finished frame.
            .with_visible(false);

        let window = Arc::new(
            event_loop
                .create_window(window_attrs)
                .expect("Failed to create window"),
        );

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });

        let surface = instance
            .create_surface(window.clone())
            .expect("Failed to create surface");

        let (adapter, device, queue) = pollster::block_on(async {
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::default(),
                    compatible_surface: Some(&surface),
                    force_fallback_adapter: false,
                })
                .await
                .expect("Failed to find GPU adapter");

            let (device, queue) = adapter
                .request_device(
                    &wgpu::DeviceDescriptor {
                        label: Some("wok_platform_device"),
                        ..Default::default()
                    },
                    None,
                )
                .await
                .expect("Failed to open GPU device");

            (adapter, device, queue)
        });

        let size = window.inner_size();
        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(surface_caps.formats[0]);

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            format: surface_format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: pick_present_mode(desc.vsync, &surface_caps.present_modes),
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &surface_config);

        let audio_host = cpal::default_host();
        let audio_device = audio_host
            .default_output_device()
            .expect("No audio output device found");
        let audio_config = audio_device
            .default_output_config()
            .expect("No default audio output config");

        Platform {
            window,
            instance,
            surface,
            adapter,
            device,
            queue,
            surface_config,
            audio_device,
            audio_config,
            supported_present_modes: surface_caps.present_modes,
            presented: false,
        }
    }
}

/// Pick a sensible window size from the primary monitor's logical dimensions.
/// 75% of the monitor's logical size on each axis, clamped to a comfortable
/// range so we don't end up with a 600x400 toolbox on a 1366x768 laptop or a
/// 2880x1620 viewport on a 4K display. Falls back to 1280x720 if no monitor is
/// reported (headless / detached scenarios).
fn auto_window_size(event_loop: &ActiveEventLoop) -> (u32, u32) {
    if let Some(monitor) = event_loop.primary_monitor() {
        let physical = monitor.size();
        let scale = monitor.scale_factor();
        if physical.width > 0 && physical.height > 0 && scale > 0.0 {
            let logical_w = (physical.width as f64 / scale * 0.75) as u32;
            let logical_h = (physical.height as f64 / scale * 0.75) as u32;
            return (logical_w.clamp(1024, 2400), logical_h.clamp(640, 1500));
        }
    }
    (1280, 720)
}

fn pick_present_mode(vsync: bool, supported: &[wgpu::PresentMode]) -> wgpu::PresentMode {
    if vsync {
        // Prefer AutoVsync, fall back to Fifo (always supported)
        if supported.contains(&wgpu::PresentMode::AutoVsync) {
            wgpu::PresentMode::AutoVsync
        } else {
            wgpu::PresentMode::Fifo
        }
    } else {
        // Prefer Immediate, then Mailbox, then Fifo as last resort
        if supported.contains(&wgpu::PresentMode::Immediate) {
            wgpu::PresentMode::Immediate
        } else if supported.contains(&wgpu::PresentMode::Mailbox) {
            wgpu::PresentMode::Mailbox
        } else {
            wgpu::PresentMode::Fifo
        }
    }
}

/// Trait the consumer implements to receive lifecycle callbacks.
///
/// Why callbacks instead of letting the consumer drive their own loop: macOS and web require the
/// OS to own the main loop, so wok-platform can't hand control to the consumer on those platforms.
/// Rather than expose two shapes (a loop-owning entry point on Linux/Windows, a callback shape
/// on macOS/web), wok-platform applies the strictest platform's requirement everywhere. One shape, no
/// platform-conditional consumer code.
pub trait App {
    fn init(&mut self, platform: &Platform);
    fn frame(&mut self, ctx: &mut FrameCtx);
    fn cleanup(&mut self, platform: &Platform);

    /// Optional raw window-event hook. Invoked before wok-platform's input collector
    /// processes the event. The `Platform` argument is `Some` once `init` has
    /// run; `None` for events that arrive before resume (rare). Default impl
    /// does nothing - games typically use the processed `InputState` in
    /// `frame`. Tooling that needs raw events (egui, custom IME, etc.) opts in.
    fn on_window_event(&mut self, _platform: Option<&Platform>, _event: &WindowEvent) {}
}

/// Per-frame context passed to `App::frame`.
pub struct FrameCtx<'a> {
    pub platform: &'a mut Platform,
    pub dt: f32,
    pub width: u32,
    pub height: u32,
    pub input: crate::input::InputState,
    /// Rumble requests pushed by the game during the frame. wok-platform's runner drains
    /// this after `App::frame` returns and dispatches the gilrs effects.
    pub rumble_requests: Vec<RumbleRequest>,
    /// Set to true to request a clean shutdown after this frame.
    pub should_close: bool,
}

impl FrameCtx<'_> {
    /// Queue a controller rumble. Strong + weak are 0..u16::MAX magnitudes for the
    /// low-frequency and high-frequency motors respectively. Duration is in
    /// milliseconds. Effects play on all connected gamepads.
    pub fn rumble(&mut self, strong: u16, weak: u16, duration_ms: u32) {
        self.rumble_requests.push(RumbleRequest {
            strong,
            weak,
            duration_ms,
        });
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RumbleRequest {
    pub strong: u16,
    pub weak: u16,
    pub duration_ms: u32,
}

struct Runner<A: App> {
    app: A,
    desc: Desc,
    platform: Option<Platform>,
    last_frame: Option<std::time::Instant>,
    input_collector: crate::input::InputCollector,
    gilrs: Option<gilrs::Gilrs>,
    /// Active rumble effects, kept alive until their duration elapses. gilrs stops
    /// the effect when the Effect handle is dropped.
    active_effects: Vec<(gilrs::ff::Effect, std::time::Instant)>,
    /// True once the window has been revealed, after the first frame presented. Guards the
    /// one-shot reveal so `set_visible(true)` fires exactly once.
    revealed: bool,
}

impl<A: App> ApplicationHandler for Runner<A> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.platform.is_some() {
            return;
        }
        let platform = Platform::init(event_loop, &self.desc);
        self.gilrs = gilrs::Gilrs::new().ok();
        self.app.init(&platform);
        // The window is created hidden, so the OS sends no initial paint to bootstrap the loop.
        // Request the first redraw ourselves; the frame it draws presents, and the redraw handler
        // reveals the window once that present lands.
        platform.window.request_redraw();
        self.platform = Some(platform);
        self.last_frame = Some(std::time::Instant::now());
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: winit::event::DeviceId,
        event: winit::event::DeviceEvent,
    ) {
        self.input_collector.handle_device_event(&event);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        // App gets the raw event first - so a tool like egui can consume mouse
        // / keyboard events before wok-platform's input collector treats them as
        // gameplay input. The collector still sees them; consumers that care
        // about who-eats-the-event-first can ignore the input snapshot when
        // their UI has focus.
        self.app.on_window_event(self.platform.as_ref(), &event);
        self.input_collector.handle_window_event(&event);

        match event {
            WindowEvent::CloseRequested => {
                if let Some(ref platform) = self.platform {
                    self.app.cleanup(platform);
                }
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                if let Some(ref mut platform) = self.platform {
                    platform.reconfigure_surface(size.width, size.height);
                }
            }
            WindowEvent::RedrawRequested => {
                let now = std::time::Instant::now();
                let dt = self
                    .last_frame
                    .map_or(0.0, |last| now.duration_since(last).as_secs_f32());
                self.last_frame = Some(now);

                if let Some(ref mut gilrs) = self.gilrs {
                    self.input_collector.poll_gamepads(gilrs);
                }

                let input_state = self.input_collector.snapshot();

                if let Some(ref mut platform) = self.platform {
                    let size = platform.window.inner_size();
                    let mut ctx = FrameCtx {
                        platform,
                        dt,
                        width: size.width,
                        height: size.height,
                        input: input_state,
                        rumble_requests: Vec::new(),
                        should_close: false,
                    };
                    self.app.frame(&mut ctx);
                    let rumbles = std::mem::take(&mut ctx.rumble_requests);
                    let close = ctx.should_close;
                    // Dispatch rumble requests via gilrs.
                    if let Some(gilrs) = self.gilrs.as_mut()
                        && !rumbles.is_empty()
                    {
                        let ids: Vec<gilrs::GamepadId> =
                            gilrs.gamepads().map(|(id, _)| id).collect();
                        if !ids.is_empty() {
                            for r in rumbles {
                                let mut builder = gilrs::ff::EffectBuilder::new();
                                if r.strong > 0 {
                                    builder.add_effect(gilrs::ff::BaseEffect {
                                        kind: gilrs::ff::BaseEffectType::Strong {
                                            magnitude: r.strong,
                                        },
                                        scheduling: gilrs::ff::Replay {
                                            play_for: gilrs::ff::Ticks::from_ms(r.duration_ms),
                                            ..Default::default()
                                        },
                                        envelope: Default::default(),
                                    });
                                }
                                if r.weak > 0 {
                                    builder.add_effect(gilrs::ff::BaseEffect {
                                        kind: gilrs::ff::BaseEffectType::Weak { magnitude: r.weak },
                                        scheduling: gilrs::ff::Replay {
                                            play_for: gilrs::ff::Ticks::from_ms(r.duration_ms),
                                            ..Default::default()
                                        },
                                        envelope: Default::default(),
                                    });
                                }
                                if let Ok(effect) = builder.gamepads(&ids).finish(gilrs) {
                                    let _ = effect.play();
                                    let expires = std::time::Instant::now()
                                        + std::time::Duration::from_millis(
                                            r.duration_ms as u64 + 50,
                                        );
                                    self.active_effects.push((effect, expires));
                                }
                            }
                        }
                    }
                    // Drop expired effects so the rumble actually stops.
                    let cutoff = std::time::Instant::now();
                    self.active_effects.retain(|(_, expires)| cutoff < *expires);
                    if close {
                        self.app.cleanup(platform);
                        event_loop.exit();
                    } else {
                        // Reveal the window once the first real frame has presented (gfx::Frame::finish
                        // set `presented`), exactly once. A skipped frame - begin_frame returned None
                        // before the surface was ready - never sets it, so the reveal waits for actual
                        // pixels and the window's first appearance is a finished frame.
                        if platform.presented && !self.revealed {
                            platform.window.set_visible(true);
                            self.revealed = true;
                        }
                        platform.window.request_redraw();
                    }
                }
            }
            _ => {}
        }
    }
}

/// Run the application. This enters the platform event loop and does not return until the
/// window is closed.
///
/// # Panics
/// Panics if the event loop, window, GPU adapter, GPU device, or audio device cannot be created.
pub fn run<A: App + 'static>(app: A, desc: Desc) {
    let event_loop = EventLoop::new().expect("Failed to create event loop");
    let mut runner = Runner {
        app,
        desc,
        platform: None,
        last_frame: None,
        input_collector: crate::input::InputCollector::new(),
        gilrs: None,
        active_effects: Vec::new(),
        revealed: false,
    };
    event_loop.run_app(&mut runner).expect("Event loop error");
}
