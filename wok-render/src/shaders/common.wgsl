// ---- shared frame bindings ----
// Group 0 holds the per-frame uniforms every pass reads: the camera and the light state. Group 1
// is the per-draw block, shared by the mesh and shadow passes (the sky pass ignores it). Each
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
    // The sun's orthographic view-projection, fitted per frame to the caller's shadow region
    // (shadow.rs). The shadow pass rasterizes through it; the mesh pass samples the map by it.
    sun_view_proj: mat4x4<f32>,
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

struct Draw {
    model: mat4x4<f32>,
    // Inverse-transpose of the model's upper 3x3, computed CPU-side per draw, so normals stay
    // perpendicular to surfaces under non-uniform scale.
    normal: mat4x4<f32>,
    // xyz: flat base color, linear RGB; w: opacity (1.0 opaque, below it the mesh pass
    // screen-door discards - see mesh.wgsl).
    color: vec4<f32>,
}

@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var<uniform> light: Light;
@group(1) @binding(0) var<uniform> draw: Draw;
