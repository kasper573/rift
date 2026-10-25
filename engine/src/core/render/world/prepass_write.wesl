import bevy_pbr::{
    render::{
        pbr_prepass_functions,
        pbr_bindings,
        pbr_types,
        pbr_functions,
        mesh_bindings::mesh,
        view_transformations::position_world_to_view,
    },
    prepass::io as prepass_io,
};

// What every mesh of the world writes in the prepass: the standard prepass, less normal maps the
// style never uses. The motion vector target carries the pixel's depth in metres from the eye
// instead of its motion, which nothing uses: the style inks silhouettes by comparing neighbours'
// depths, and a depth texture cannot be read by pixel on every platform, while a colour target can.
@if(PREPASS_FRAGMENT)
fn prepass_output(in: prepass_io::VertexOutput, is_front: bool) -> prepass_io::FragmentOutput {
    pbr_prepass_functions::prepass_sample_color_and_alpha_discard(in);
    var out: prepass_io::FragmentOutput;
@if(UNCLIPPED_DEPTH_ORTHO_EMULATION)
    out.frag_depth = in.unclipped_depth;
@if(NORMAL_PREPASS) {
    let flags = pbr_bindings::material.flags;
    let double_sided = (flags & pbr_types::STANDARD_MATERIAL_FLAGS_DOUBLE_SIDED_BIT) != 0u;
    let is_logically_front = pbr_functions::winding_corrected_front_facing(mesh[in.instance_index].flags, is_front);
    let normal = pbr_functions::prepare_world_normal(in.world_normal, double_sided, is_logically_front);
    out.normal = vec4(normal * 0.5 + vec3(0.5), 1.0);
}
@if(MOTION_VECTOR_PREPASS)
    out.motion_vector = vec2<f32>(-position_world_to_view(in.world_position.xyz).z, 0.0);
    return out;
}
