// ---- shared frame bindings ----
// Group 0 holds the per-frame uniforms both passes read: the camera and the light state. Each
// pipeline's shader module is this file concatenated with the pass-specific file at build time
// (see pipeline.rs), so these declarations exist exactly once.
//
// Layout note: every field is vec4-aligned by construction. Scalars ride in the fourth lane of a
// related color or direction instead of fighting WGSL uniform padding rules; the CPU-side packing
// in uniforms.rs is the single source of what sits where.

struct Camera {
    view_proj: mat4x4<f32>,
    // Inverse of view_proj; the sky pass unprojects NDC positions to world-space rays with it.
    inv_view_proj: mat4x4<f32>,
    // xyz: camera world position. Fog distance and rim lighting measure from here.
    eye: vec4<f32>,
}

struct Light {
    // xyz: normalized travel direction of sunlight (sun toward scene); w: cel band count.
    sun_dir_bands: vec4<f32>,
    // xyz: sun color, linear RGB; w: cel transition softness.
    sun_color_soft: vec4<f32>,
    // xyz: ambient floor color; w: rim light intensity.
    ambient_rim: vec4<f32>,
    // xyz: fog color; w: fog start distance in metres.
    fog_color_start: vec4<f32>,
    // xyz: sky horizon color; w: fog end distance in metres.
    horizon_fog_end: vec4<f32>,
    // xyz: sky zenith color.
    zenith: vec4<f32>,
}

@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var<uniform> light: Light;
