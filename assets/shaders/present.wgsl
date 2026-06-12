#import bevy_sprite::mesh2d_vertex_output::VertexOutput

@group(2) @binding(0) var world_texture: texture_2d<f32>;
@group(2) @binding(1) var world_sampler: sampler;
@group(2) @binding(2) var<uniform> dead: f32;

// pixel upscaling + death tint

@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    let tex_size = vec2<f32>(textureDimensions(world_texture));
    let texel = mesh.uv * tex_size;
    let per_pixel = fwidth(texel);
    let from_center = fract(texel) - 0.5;
    let ramp = vec2<f32>(0.5) - 0.5 * per_pixel;
    let f = (from_center - clamp(from_center, -ramp, ramp)) / per_pixel + 0.5;
    let uv = (floor(texel) + f) / tex_size;
    var color = textureSample(world_texture, world_sampler, uv);
    if dead > 0.5 {
        color = vec4<f32>(min(color.r + 160.0 / 255.0, 1.0), color.g / 3.0, color.b / 3.0, color.a);
    }
    return color;
}
