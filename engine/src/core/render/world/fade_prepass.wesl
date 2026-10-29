import bevy_pbr::{
    render::pbr_prepass_functions,
    prepass::io as prepass_io,
};

import engine::core::render::world::clearing::clear_pixel;
import engine::core::render::world::prepass_write::prepass_output;

@if(PREPASS_FRAGMENT)
@fragment
fn fragment(in: prepass_io::VertexOutput, @builtin(front_facing) is_front: bool) -> prepass_io::FragmentOutput {
    clear_pixel(in.position.xy);
    return prepass_output(in, is_front);
}
@else
// Depth alone is the sun's shadow map, which a faded thing keeps whole.
@fragment
fn fragment(in: prepass_io::VertexOutput) {
    pbr_prepass_functions::prepass_sample_color_and_alpha_discard(in);
}
