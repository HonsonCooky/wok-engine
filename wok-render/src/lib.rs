//! wok-render: the forward renderer.
//!
//! Part 1 was the smallest renderer that puts real pixels on screen: GPU mesh drawing through a
//! single forward pass, cel-shaded geometry under a [`LightState`]'s directional sun, distance
//! fog (always on, an identity commitment per the HLD), and the parametric gradient sky. Part 2
//! adds the other identity commitment: one shadow map per frame, rendered from the sun. This
//! revision adds per-item opacity as screen-door cutout (below). Later parts add post-process.
//!
//! ## The render-list contract
//!
//! The caller supplies, each frame: a [`Camera`] (final view-projection matrix plus eye
//! position), a [`LightState`], a shadow region (a world-space [`Aabb`], see below), and a list
//! of [`RenderItem`]s, each a final world transform, a [`MeshGpu`] reference, a flat base
//! color, and an opacity. wok-render reads no stores and no pools, keeps no per-frame scene
//! state, and draws
//! exactly what it is handed; chunk-origin composition, culling, and ordering are the caller's
//! policy. This is HLD principle 5 applied to drawing: the game owns the loop and the state, the
//! engine owns the GPU mechanics.
//!
//! By default the frame fills the whole target. A caller may instead confine it to a sub-rect with
//! [`Renderer::set_viewport`] (a [`ViewportRect`] in physical pixels): the colour passes' viewport
//! and scissor scope to the rect, the offscreen depth and shadow resources stay sized to the full
//! target, and the caller supplies an aspect-matched camera so the view stays centred and
//! undistorted inside it. It is "where on the target", not a second configuration (HLD: one
//! target). The editor uses it to keep the 3D inside its viewport panel; taste leaves it unset.
//!
//! After the frame, a caller may overlay debug lines: [`Renderer::render_lines`] draws a list of
//! [`LineSegment`]s (world-space endpoints plus a flat color) through the same camera, unlit and
//! unfogged, outside the shadow pass entirely. The caller's [`DepthMode`] decides whether the
//! lines depth-test against the frame's geometry (world-anchored cues) or draw through it (x-ray
//! diagnostics: a hitbox cage must read even behind the surface it describes). A diagnostic
//! overlay, not a scene element; it exists so applications can draw hitboxes and other invisible
//! structure without inventing degenerate meshes.
//!
//! ## Opacity: screen-door cutout, not blending
//!
//! The HLD's transparency commitment is alpha cutout only - no sorted blending - and per-item
//! opacity does not amend it: an opacity below 1.0 discards fragments on a 4x4 Bayer threshold
//! matrix tiled over screen space, which is cutout, used as a fade. Bayer rather than noise
//! because the pattern is ordered and a pure function of the pixel coordinate: stable frame to
//! frame (no temporal shimmer), and an opacity of k/16 keeps exactly k of every 16 pixels of a
//! covered tile. Surviving fragments are ordinary opaque fragments: they shade, fog, and write
//! depth exactly as at opacity 1.0, and the item still casts its full shadow (the depth-only
//! shadow pass never reads opacity). That depth-and-shadow policy is deliberate v1 behavior: a
//! fade reads fine with the real shadow still grounding the object; revisit only if it reads
//! wrong in play. At exactly 1.0 nothing discards and the output is bit-identical to the
//! pre-opacity renderer.
//!
//! ## The shadow map
//!
//! One shadow map per frame, from the [`LightState`]'s sun: a depth-only pass renders every item
//! from the sun's view through an orthographic projection fitted to the caller's shadow region
//! (the world-space AABB shadows must cover - typically the bounds of the caller's loaded
//! content). The fit snaps to shadow-map texels so the map does not shimmer as the region moves
//! (see `shadow.rs`); resolution is a construction parameter ([`DEFAULT_SHADOW_MAP_SIZE`]).
//! Everything in the render list casts and receives, terrain included; the sky does neither;
//! there are no per-object toggles. Dynamic pool lights do not cast (HLD).
//!
//! ## Shading
//!
//! All shading is driven by the [`LightState`] passed each frame; nothing is baked into the
//! pipeline. The sun's lambert term - scaled by the shadow factor first, so shadowed surface
//! falls through the bands and full shadow rests exactly on the ambient floor - quantizes into
//! `band_count` discrete bands with `transition_softness` smoothstep edges; rim light at
//! `rim_intensity` brightens silhouettes and is deliberately not shadowed (a silhouette cue, not
//! illumination); the ambient color is a floor (the minimum light any surface receives), not an
//! additive term. Fog blends toward the fog color by distance from the eye after lighting. The
//! sky is a horizon-to-zenith gradient evaluated per pixel from the camera's view ray. Shader
//! sources live in `src/shaders/` and are compiled into the binary.
//!
//! ## Determinism
//!
//! Rendering output is not simulation state (see `designs/project-canon.md`): nothing here feeds
//! back into gameplay, and the determinism contract's rendering carve-out applies. The CPU side
//! is still plainly deterministic - uniform packing and the shadow fit are pure arithmetic - and
//! pixel-level regression is Level 3's screenshot diff, later.
//!
//! ## Errors
//!
//! The crate exposes no error enum: construction and rendering have no reportable failure modes
//! (see [`Renderer`]). Per canon, a `thiserror` enum lands with the first genuine one.
//!
//! [`LightState`]: wok_light::LightState
//! [`MeshGpu`]: wok_mesh::MeshGpu
//! [`Aabb`]: wok_scene::Aabb

mod pipeline;
mod renderer;
mod shadow;
mod uniforms;

pub use renderer::{
    Camera, DEFAULT_SHADOW_MAP_SIZE, DepthMode, LineSegment, RenderItem, Renderer, ViewportRect,
};
