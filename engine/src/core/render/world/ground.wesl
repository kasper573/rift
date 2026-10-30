import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::main_pass_post_lighting_processing,
};
import engine::core::render::world::shade::lighting;
import engine::core::render::world::terrain::Ground;

@group(3) @binding(100) var pictures: texture_2d_array<f32>;
@group(3) @binding(101) var pictures_sampler: sampler;
@group(3) @binding(102) var paint: texture_2d<u32>;
@group(3) @binding(103) var layers: texture_2d<f32>;
@group(3) @binding(104) var<uniform> ground: Ground;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.material.base_color = vec4<f32>(painted(in.world_position.xz), 1.0);
    var out: FragmentOutput;
    out.color = main_pass_post_lighting_processing(pbr_input, lighting(pbr_input));
    return out;
}

// The ground's colour at `at`: the terrains of the four paint pixels round it, each weighed by how
// near it lies, as the paint blends wherever the game reads it.
fn painted(at: vec2<f32>) -> vec3<f32> {
    let pixel = (at - ground.origin) * ground.samples_per_metre - 0.5;
    let corner = floor(pixel);
    let t = pixel - corner;
    return terrain_at(corner, vec2<f32>(0.0, 0.0), at) * (1.0 - t.x) * (1.0 - t.y)
        + terrain_at(corner, vec2<f32>(1.0, 0.0), at) * t.x * (1.0 - t.y)
        + terrain_at(corner, vec2<f32>(0.0, 1.0), at) * (1.0 - t.x) * t.y
        + terrain_at(corner, vec2<f32>(1.0, 1.0), at) * t.x * t.y;
}

// The colour the terrain painted at the pixel `step` from `corner` lays at `at`: its colour under
// its picture, repeating every tile.
fn terrain_at(corner: vec2<f32>, step: vec2<f32>, at: vec2<f32>) -> vec3<f32> {
    let last = vec2<i32>(textureDimensions(paint)) - 1;
    let pixel = clamp(vec2<i32>(corner + step), vec2<i32>(0), last);
    let terrain = i32(textureLoad(paint, pixel, 0).r);
    let look = textureLoad(layers, vec2<i32>(terrain, 0), 0);
    let picture = i32(textureLoad(layers, vec2<i32>(terrain, 1), 0).r);
    return look.rgb * textureSample(pictures, pictures_sampler, at / look.w, picture).rgb;
}
