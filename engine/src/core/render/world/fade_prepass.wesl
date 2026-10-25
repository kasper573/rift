import bevy_pbr::{
    render::{
        pbr_prepass_functions,
        pbr_bindings::material,
        pbr_types,
        pbr_functions,
        mesh_bindings::mesh,
    },
    prepass::io as prepass_io,
};

import engine::core::render::world::clearing::clear_pixel;

@if(PREPASS_FRAGMENT)
@fragment
fn fragment(in: prepass_io::VertexOutput, @builtin(front_facing) is_front: bool) -> prepass_io::FragmentOutput {
    clear_pixel(in.position.xy);
    pbr_prepass_functions::prepass_sample_color_and_alpha_discard(in);
    var out: prepass_io::FragmentOutput;
@if(UNCLIPPED_DEPTH_ORTHO_EMULATION)
    out.frag_depth = in.unclipped_depth;
@if(NORMAL_PREPASS) {
    let double_sided = (material.flags & pbr_types::STANDARD_MATERIAL_FLAGS_DOUBLE_SIDED_BIT) != 0u;
    let is_logically_front = pbr_functions::winding_corrected_front_facing(mesh[in.instance_index].flags, is_front);
    let normal = pbr_functions::prepare_world_normal(in.world_normal, double_sided, is_logically_front);
    out.normal = vec4(normal * 0.5 + vec3(0.5), 1.0);
}
@if(MOTION_VECTOR_PREPASS)
    out.motion_vector = pbr_prepass_functions::calculate_motion_vector(in.world_position, in.previous_world_position);
    return out;
}
@else
// Depth alone is the sun's shadow map, which a faded thing keeps whole.
@fragment
fn fragment(in: prepass_io::VertexOutput) {
    pbr_prepass_functions::prepass_sample_color_and_alpha_discard(in);
}
