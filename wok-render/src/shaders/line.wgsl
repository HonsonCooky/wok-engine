// ---- debug line pass ----
// Unlit, depth-tested line segments drawn after the meshes in a frame. Each vertex carries a
// world-space position and a color, transformed by the same camera the meshes used. No lighting,
// no fog, no shadow interaction: diagnostics want the authored color, unconditionally, and a line
// has no surface for any of those to act on.

struct LineVsIn {
    @location(0) position: vec3<f32>,
    @location(1) color: vec3<f32>,
}

struct LineVsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec3<f32>,
}

@vertex
fn vs_line(in: LineVsIn) -> LineVsOut {
    var out: LineVsOut;
    out.clip = camera.view_proj * vec4<f32>(in.position, 1.0);
    out.color = in.color;
    return out;
}

@fragment
fn fs_line(in: LineVsOut) -> @location(0) vec4<f32> {
    return vec4<f32>(in.color, 1.0);
}
