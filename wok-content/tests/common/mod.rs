//! Shared test helpers. Headless wgpu adapter+device acquisition for GPU-touching tests.
//!
//! Headless adapters work everywhere wgpu does; they pick whichever backend the platform's
//! wgpu can reach without a surface. For tests this is enough - we never present a
//! swapchain, we only allocate buffers and queue writes.

use std::sync::Arc;

use pantry::wgpu;

/// Acquire a headless wgpu device + queue for tests. Returns Arc-wrapped handles matching
/// `ContentSystem::new`'s signature so tests can drop straight into the real wiring.
///
/// The instance is built with the default backend set (whatever the host supports). Adapter
/// power preference is `LowPower` because tests don't render and don't want to spin up
/// discrete GPUs unnecessarily.
pub fn init_gpu() -> (Arc<wgpu::Device>, Arc<wgpu::Queue>) {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::LowPower,
        force_fallback_adapter: false,
        compatible_surface: None,
    }))
    .expect("no wgpu adapter available - tests require a runnable GPU or software fallback");
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("wok-content-test-device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults(),
            memory_hints: wgpu::MemoryHints::default(),
        },
        None, // trace path
    ))
    .expect("wgpu device request failed");
    (Arc::new(device), Arc::new(queue))
}
