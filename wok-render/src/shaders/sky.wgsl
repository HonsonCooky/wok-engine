// ---- sky pass ----
// The parametric gradient background: one fullscreen triangle drawn before geometry with depth
// writes off. Each fragment unprojects its NDC position to a world-space view ray and blends
// horizon to zenith by the ray's upward component; rays at or below the horizon clamp to the
// horizon color. No sun disc, stars, or clouds yet (later parts).

struct SkyVsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
}

@vertex
fn vs_sky(@builtin(vertex_index) index: u32) -> SkyVsOut {
    // The oversized-triangle trick: indices 0, 1, 2 map to (-1,-1), (3,-1), (-1,3), covering the
    // whole viewport with a single primitive and no vertex buffer.
    let xy = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u)) * 2.0 - 1.0;
    var out: SkyVsOut;
    out.clip = vec4<f32>(xy, 1.0, 1.0);
    out.ndc = xy;
    return out;
}

@fragment
fn fs_sky(in: SkyVsOut) -> @location(0) vec4<f32> {
    // Unproject the fragment's far-plane point and form the view ray from the eye through it.
    let far = camera.inv_view_proj * vec4<f32>(in.ndc, 1.0, 1.0);
    let dir = normalize(far.xyz / far.w - camera.eye.xyz);
    let t = clamp(dir.y, 0.0, 1.0);
    return vec4<f32>(mix(light.horizon_fog_end.xyz, light.zenith_fog_on.xyz, t), 1.0);
}
