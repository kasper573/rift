import bevy_pbr::{
    render::pbr_prepass_functions,
    prepass::io as prepass_io,
};

// What a cover writes in a prepass that has a fragment stage: the sun's shadow map on a device
// that cannot leave depth unclipped, where the depth is written by hand.
@if(PREPASS_FRAGMENT)
fn prepass_output(in: prepass_io::VertexOutput, is_front: bool) -> prepass_io::FragmentOutput {
    pbr_prepass_functions::prepass_sample_color_and_alpha_discard(in);
    var out: prepass_io::FragmentOutput;
@if(UNCLIPPED_DEPTH_ORTHO_EMULATION)
    out.frag_depth = in.unclipped_depth;
    return out;
}
