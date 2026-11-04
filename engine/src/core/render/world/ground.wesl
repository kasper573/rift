import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    mesh_functions::get_tag,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::main_pass_post_lighting_processing,
};
import engine::core::render::world::shade::lighting;

@group(3) @binding(100) var pictures: texture_2d_array<f32>;
@group(3) @binding(101) var pictures_sampler: sampler;
@group(3) @binding(103) var paints: texture_2d_array<u32>;
@group(3) @binding(104) var layers: texture_2d<f32>;
// Where each chunk's paint pixels lie: from the edge of the first at `xy`, `z` to the metre.
@group(3) @binding(105) var grids: texture_2d<f32>;

// What the terrains lay at a point: their colour, and how they look soaked, the share of their
// colour left in x and how rough they lie in y, as far as rain wets them in z.
struct Laid {
    color: vec3<f32>,
    wet: vec3<f32>,
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    read_weather();
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    let chunk = i32(get_tag(in.instance_index));
    let ground = painted(in.world_position.xz, chunk);
    let soaked = wetness(in.world_position.xyz) * ground.wet.z;
    pbr_input.material.base_color = vec4<f32>(ground.color * mix(1.0, ground.wet.x, soaked), 1.0);
    pbr_input.material.perceptual_roughness = mix(pbr_input.material.perceptual_roughness, ground.wet.y, soaked);
    var out: FragmentOutput;
    out.color = main_pass_post_lighting_processing(pbr_input, lighting(pbr_input));
    return out;
}

// What the terrains lay at `at` on `chunk`: those of the four paint pixels round it, each
// weighed by how near it lies, as the paint blends wherever the game reads it.
fn painted(at: vec2<f32>, chunk: i32) -> Laid {
    let grid = textureLoad(grids, vec2<i32>(chunk, 0), 0);
    let pixel = (at - grid.xy) * grid.z - 0.5;
    let corner = floor(pixel);
    let t = pixel - corner;
    let a = terrain_at(chunk, corner, vec2<f32>(0.0, 0.0), at);
    let b = terrain_at(chunk, corner, vec2<f32>(1.0, 0.0), at);
    let c = terrain_at(chunk, corner, vec2<f32>(0.0, 1.0), at);
    let d = terrain_at(chunk, corner, vec2<f32>(1.0, 1.0), at);
    return Laid(
        mix(mix(a.color, b.color, t.x), mix(c.color, d.color, t.x), t.y),
        mix(mix(a.wet, b.wet, t.x), mix(c.wet, d.wet, t.x), t.y),
    );
}

// What the terrain painted on `chunk` at the pixel `step` from `corner` lays at `at`: its colour
// under its picture, repeating every tile, and how it looks soaked.
fn terrain_at(chunk: i32, corner: vec2<f32>, step: vec2<f32>, at: vec2<f32>) -> Laid {
    let last = vec2<i32>(textureDimensions(paints)) - 1;
    let pixel = clamp(vec2<i32>(corner + step), vec2<i32>(0), last);
    let painted = textureLoad(paints, pixel, chunk, 0).r;
    let terrain = min(i32(painted), i32(textureDimensions(layers).x) - 1);
    let look = textureLoad(layers, vec2<i32>(terrain, 0), 0);
    let picture = i32(textureLoad(layers, vec2<i32>(terrain, 1), 0).r);
    let wet = textureLoad(layers, vec2<i32>(terrain, 2), 0).xyz;
    return Laid(look.rgb * textureSample(pictures, pictures_sampler, at / look.w, picture).rgb, wet);
}
