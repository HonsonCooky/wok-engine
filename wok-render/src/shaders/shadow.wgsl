// ---- shadow depth pass ----
// Every mesh item rasterized from the sun's fitted orthographic view (camera.sun_view_proj) into
// the shadow map. Depth-only: no fragment stage, no color targets; the pipeline's depth bias does
// the acne work (see pipeline.rs). Everything the caller hands the frame casts - terrain and all
// render items alike, no per-object toggles - and the sky never enters this pass.

@vertex
fn vs_shadow(@location(0) position: vec3<f32>) -> @builtin(position) vec4<f32> {
    return camera.sun_view_proj * draw.model * vec4<f32>(position, 1.0);
}
