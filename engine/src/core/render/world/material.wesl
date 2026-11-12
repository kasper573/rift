import bevy_pbr::render::forward_io::{VertexOutput, FragmentOutput};

import engine::core::render::world::opacity::thin;
import engine::core::render::world::shade::lit_color;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    thin(in.position.xy, in.instance_index);
    var out: FragmentOutput;
    out.color = lit_color(in, is_front);
    return out;
}
