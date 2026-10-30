// The model, lit as the studio lights it: flat-shaded triangles with a
// color per vertex, a headlight, a fill light and a highlight.

#include <metal_stdlib>
using namespace metal;

struct Uniforms {
    float4x4 view;
    float4x4 projection;
};

struct Varyings {
    float4 position [[position]];
    float3 eye_position;
    float3 normal;
    float4 color;
};

// The engine's mesh: little-endian f32s, three per position and normal,
// four per color, one vertex after the other.
vertex Varyings model_vertex(uint id [[vertex_id]],
                             const device packed_float3 *positions [[buffer(0)]],
                             const device packed_float3 *normals [[buffer(1)]],
                             const device float4 *colors [[buffer(2)]],
                             constant Uniforms &u [[buffer(3)]]) {
    float4 p = u.view * float4(float3(positions[id]), 1.0);
    Varyings out;
    out.position = u.projection * p;
    out.eye_position = p.xyz;
    out.normal = (u.view * float4(float3(normals[id]), 0.0)).xyz;
    out.color = colors[id];
    return out;
}

fragment float4 model_fragment(Varyings in [[stage_in]]) {
    float3 n = normalize(in.normal);
    float3 to_eye = normalize(-in.eye_position);
    // Two-sided: light whichever side faces the camera.
    if (dot(n, to_eye) < 0.0) n = -n;
    // A headlight slightly above and right of the camera, and a fill light.
    float3 key = normalize(float3(0.35, 0.45, 1.0));
    float3 fill = normalize(float3(-0.75, -0.35, 0.30));
    float diffuse = max(dot(n, key), 0.0);
    float filled = max(dot(n, fill), 0.0);
    float3 color = in.color.rgb * (0.30 + 0.62 * diffuse + 0.22 * filled);
    float3 halfway = normalize(key + to_eye);
    color += float3(0.16) * pow(max(dot(n, halfway), 0.0), 48.0);
    return float4(min(color, float3(1.0)), 1.0);
}
