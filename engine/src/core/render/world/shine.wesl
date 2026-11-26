import bevy_pbr::render::forward_io::VertexOutput;
import bevy_pbr::render::mesh_bindings::mesh;
import bevy_pbr::render::mesh_view_bindings::view;

import engine::core::render::world::shade::{finished, lit};

// What a vfx lays over a thing's lit surface, as the engine lays it in the buffer.
struct SurfaceLook {
    tint: vec4<f32>,
    rim: vec4<f32>,
    glow: vec4<f32>,
    mask: vec4<f32>,
}

// The looks of every shining thing, by slot; slot 0 lays nothing.
@group(3) @binding(109) var<storage, read> looks: array<SurfaceLook>;

// How many of a mesh tag's low bits tell how much of the mesh has faded.
const FADE_BITS: u32 = 8u;

// A pixel of a world material lit, with the look its mesh's tag names laid over it, then fogged
// and finished. Slot 0 is a look of nothing, so every pixel reads its look alike.
fn shone_color(in: VertexOutput, is_front: bool) -> vec4<f32> {
    var pixel = lit(in, is_front);
    let look = looks[mesh[in.instance_index].tag >> FADE_BITS];
    var seen = 1.0;
    if look.mask.w > 0.0 {
        seen = smoothstep(look.mask.w, look.mask.w * 0.45, distance(in.world_position.xyz, look.mask.xyz));
    }
    let color = pixel.color.rgb;
    let luma = dot(color, vec3<f32>(0.3, 0.59, 0.11));
    let tinted = mix(color, look.tint.rgb * (0.35 + luma * 1.6), look.tint.a * seen);
    let toward_eye = normalize(view.world_position - in.world_position.xyz);
    let rim = pow(1.0 - clamp(dot(normalize(in.world_normal), toward_eye), 0.0, 1.0), 2.5);
    let added = (look.rim.rgb * look.rim.a * rim + look.glow.rgb * look.glow.a) * seen;
    pixel.color = vec4<f32>(tinted + added, pixel.color.a);
    return finished(pixel);
}
