// Wear laid over a material's colours by the mesh's second texture coordinates, so it falls over
// a whole roof or beam however the first set repeats its picture: where the mask is white the
// surface takes the wear's colour, which the mottle picks between its two colours at each point.
// A material with no wear, or a mesh with no second coordinates, keeps its colours as they are.
import bevy_pbr::render::forward_io::VertexOutput;

struct Wear {
    low: vec4<f32>,
    high: vec4<f32>,
}

@if(WORN)
@group(3) @binding(104) var wear_mask: texture_2d<f32>;
@if(WORN)
@group(3) @binding(105) var wear_mask_sampler: sampler;
@if(WORN)
@group(3) @binding(106) var wear_mottle: texture_2d<f32>;
@if(WORN)
@group(3) @binding(107) var wear_mottle_sampler: sampler;
@if(WORN)
@group(3) @binding(108) var<uniform> wear: Wear;

fn worn(base: vec4<f32>, in: VertexOutput) -> vec4<f32> {
    @if(WORN && VERTEX_UVS_B) {
        let mottle = textureSample(wear_mottle, wear_mottle_sampler, in.uv_b).r;
        let colour = mix(wear.low.rgb, wear.high.rgb, mottle);
        let shown = textureSample(wear_mask, wear_mask_sampler, in.uv_b).r;
        return vec4<f32>(mix(base.rgb, colour, shown), base.a);
    }
    return base;
}
