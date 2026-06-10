// ---- forward mesh pass ----
// Cel-shaded geometry under the single directional sun, with distance fog applied after lighting.
// One draw per render item; group 1 is the per-draw block, bound at a dynamic offset.

struct Draw {
    model: mat4x4<f32>,
    // Inverse-transpose of the model's upper 3x3, computed CPU-side per draw, so normals stay
    // perpendicular to surfaces under non-uniform scale.
    normal: mat4x4<f32>,
    // xyz: flat base color, linear RGB.
    color: vec4<f32>,
}

@group(1) @binding(0) var<uniform> draw: Draw;

struct MeshVsIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
}

struct MeshVsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) world_normal: vec3<f32>,
}

@vertex
fn vs_mesh(in: MeshVsIn) -> MeshVsOut {
    let world = draw.model * vec4<f32>(in.position, 1.0);
    var out: MeshVsOut;
    out.clip = camera.view_proj * world;
    out.world_pos = world.xyz;
    out.world_normal = (draw.normal * vec4<f32>(in.normal, 0.0)).xyz;
    return out;
}

@fragment
fn fs_mesh(in: MeshVsOut) -> @location(0) vec4<f32> {
    let n = normalize(in.world_normal);
    let to_sun = -light.sun_dir_bands.xyz;
    let lambert = max(dot(n, to_sun), 0.0);

    // Quantize the lambert term into discrete cel bands. With B bands the lit levels are
    // i / (B - 1) for band index i; the smoothstep blends the last `softness` fraction of each
    // band into the next, so the band edge is a controlled gradient instead of a hard alias.
    // uniforms.rs guarantees B >= 2 and softness > 0.
    let bands = light.sun_dir_bands.w;
    let softness = light.sun_color_soft.w;
    let scaled = lambert * bands;
    let band = min(floor(scaled), bands - 1.0);
    let edge = smoothstep(1.0 - softness, 1.0, scaled - band);
    let level = clamp((band + edge) / (bands - 1.0), 0.0, 1.0);

    // Rim light: grazing silhouettes brighten in the sun's color, scaled by rim intensity. The
    // fixed exponent sets how tight the rim hugs the silhouette; revisit if scenes want it tunable.
    let view = normalize(camera.eye.xyz - in.world_pos);
    let facing = max(dot(n, view), 0.0);
    let rim = light.ambient_rim.w * pow(1.0 - facing, 3.0) * light.sun_color_soft.xyz;

    // The ambient floor is a minimum, not an additive term: fully shadowed bands sit at exactly
    // the ambient color, keeping the dark side of the cel ramp flat.
    let lit = max(light.sun_color_soft.xyz * level, light.ambient_rim.xyz);
    let shaded = draw.color.xyz * lit + rim;

    // Distance fog after lighting (HLD: fog is always on). uniforms.rs guarantees end > start.
    let dist = distance(in.world_pos, camera.eye.xyz);
    let span = light.horizon_fog_end.w - light.fog_color_start.w;
    let fog = clamp((dist - light.fog_color_start.w) / span, 0.0, 1.0);
    return vec4<f32>(mix(shaded, light.fog_color_start.xyz, fog), 1.0);
}
