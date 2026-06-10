// ---- forward mesh pass ----
// Cel-shaded geometry under the single directional sun, with the sun's shadow map scaling the
// lambert term and distance fog applied after lighting. One draw per render item; group 1 is the
// per-draw block (declared in common.wgsl), bound at a dynamic offset.

@group(2) @binding(0) var shadow_map: texture_depth_2d;
@group(2) @binding(1) var shadow_sampler: sampler_comparison;

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

// Sun visibility at a world position: 1.0 fully lit, 0.0 fully shadowed. A 3x3 PCF tap grid over
// the shadow map; each tap is itself a hardware 2x2 comparison blend (the sampler filters with
// Linear), so edges resolve soft instead of stair-stepped. Positions outside the map - outside
// the caller's shadow region, or past its depth range - are lit: no shadow information exists
// there, and darkening would invent occluders.
fn sun_shadow(world_pos: vec3<f32>) -> f32 {
    let clip = camera.sun_view_proj * vec4<f32>(world_pos, 1.0);
    let ndc = clip.xyz / clip.w;
    let uv = ndc.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5, 0.5);
    if uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 || ndc.z <= 0.0 || ndc.z >= 1.0 {
        return 1.0;
    }
    let texel = 1.0 / vec2<f32>(textureDimensions(shadow_map));
    var lit = 0.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let offset = vec2<f32>(f32(x), f32(y)) * texel;
            lit += textureSampleCompareLevel(shadow_map, shadow_sampler, uv + offset, ndc.z);
        }
    }
    return lit / 9.0;
}

@fragment
fn fs_mesh(in: MeshVsOut) -> @location(0) vec4<f32> {
    let n = normalize(in.world_normal);
    let to_sun = -light.sun_dir_bands.xyz;

    // The shadow factor scales the lambert term BEFORE band quantization (the design decision):
    // shadowed surface falls through the cel bands like any unlit surface, and full shadow rests
    // exactly on the ambient floor - the same darkness the dark band already uses - instead of
    // multiplying a separate gray over the banded result.
    let lambert = max(dot(n, to_sun), 0.0) * sun_shadow(in.world_pos);

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
    // Deliberately NOT shadowed: rim is a silhouette cue, not illumination.
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
