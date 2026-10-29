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

// What a material of the world writes in a prepass that has a fragment stage, past the pixels
// its alpha drops: its depth where the device cannot leave depth unclipped, the way its face
// turns for the normals the main pass lights it by, and how it moved for the frames to settle.
@if(PREPASS_FRAGMENT)
fn prepass_output(in: prepass_io::VertexOutput, is_front: bool) -> prepass_io::FragmentOutput {
    pbr_prepass_functions::prepass_sample_color_and_alpha_discard(in);
    var out: prepass_io::FragmentOutput;
    @if(UNCLIPPED_DEPTH_ORTHO_EMULATION) {
        out.frag_depth = in.unclipped_depth;
    }
    @if(NORMAL_PREPASS) {
        let double_sided = (material.flags & pbr_types::STANDARD_MATERIAL_FLAGS_DOUBLE_SIDED_BIT) != 0u;
        let is_logically_front = pbr_functions::winding_corrected_front_facing(mesh[in.instance_index].flags, is_front);
        let normal = pbr_functions::prepare_world_normal(in.world_normal, double_sided, is_logically_front);
        out.normal = vec4(normal * 0.5 + vec3(0.5), 1.0);
    }
    @if(MOTION_VECTOR_PREPASS) {
        out.motion_vector = pbr_prepass_functions::calculate_motion_vector(in.world_position, in.previous_world_position);
    }
    return out;
}
