// The header of a shader run in the material of the thing it plays on: the thing's own lit surface
// is the image `sample` gives, whatever uv is asked for, and what the shader paints is drawn in its
// place.
import bevy_pbr::render::forward_io::{VertexOutput, FragmentOutput};
import engine::core::render::world::toon_shade::toon_color;

@group(3) @binding(100) var<uniform> site: Site;
@group(3) @binding(101) var<uniform> params: Params;

var<private> lit: vec4<f32>;

fn sample(at: vec2<f32>) -> vec4<f32> {
    return lit;
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    lit = toon_color(in, is_front, 1.0);
    var uv = vec2<f32>(0.5);
    @if(VERTEX_UVS_A) {
        uv = in.uv;
    }
    var out: FragmentOutput;
    out.color = vfx(uv);
    return out;
}
