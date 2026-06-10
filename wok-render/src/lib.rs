//! wok-render: the forward renderer.
//!
//! Part 1 is the smallest renderer that puts real pixels on screen: GPU mesh drawing through a
//! single forward pass, cel-shaded geometry under a [`LightState`]'s directional sun, distance
//! fog (always on, an identity commitment per the HLD), and the parametric gradient sky. Later
//! parts add the single shadow map, post-process, and alpha cutout.
//!
//! ## The render-list contract
//!
//! The caller supplies, each frame: a [`Camera`] (final view-projection matrix plus eye
//! position), a [`LightState`], and a list of [`RenderItem`]s, each a final world transform, a
//! [`MeshGpu`] reference, and a flat base color. wok-render reads no stores and no pools, keeps
//! no per-frame scene state, and draws exactly what it is handed; chunk-origin composition,
//! culling, and ordering are the caller's policy. This is HLD principle 5 applied to drawing:
//! the game owns the loop and the state, the engine owns the GPU mechanics.
//!
//! ## Shading
//!
//! All shading is driven by the [`LightState`] passed each frame; nothing is baked into the
//! pipeline. The sun's lambert term quantizes into `band_count` discrete bands with
//! `transition_softness` smoothstep edges; rim light at `rim_intensity` brightens silhouettes;
//! the ambient color is a floor (the minimum light any surface receives), not an additive term.
//! Fog blends toward the fog color by distance from the eye after lighting. The sky is a
//! horizon-to-zenith gradient evaluated per pixel from the camera's view ray. Shader sources
//! live in `src/shaders/` and are compiled into the binary.
//!
//! ## Determinism
//!
//! Rendering output is not simulation state (see `designs/project-canon.md`): nothing here feeds
//! back into gameplay, and the determinism contract's rendering carve-out applies. The CPU side
//! is still plainly deterministic - uniform packing is pure arithmetic - and pixel-level
//! regression is Level 3's screenshot diff, later.
//!
//! ## Errors
//!
//! The crate exposes no error enum: construction and rendering have no reportable failure modes
//! (see [`Renderer`]). Per canon, a `thiserror` enum lands with the first genuine one.
//!
//! [`LightState`]: wok_light::LightState
//! [`MeshGpu`]: wok_mesh::MeshGpu

mod pipeline;
mod renderer;
mod uniforms;

pub use renderer::{Camera, RenderItem, Renderer};
