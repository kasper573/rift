import bevy_pbr::render::forward_io::{VertexOutput, FragmentOutput};

import engine::core::render::world::clearing::clear_pixel;
import engine::core::render::world::shade::lit_color;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    clear_pixel(in.position.xy);
    var out: FragmentOutput;
    out.color = lit_color(in, is_front);
    return out;
}
