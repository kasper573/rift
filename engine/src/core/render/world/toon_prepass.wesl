import bevy_pbr::{
    render::{
        pbr_prepass_functions,
        pbr_bindings,
        pbr_types,
        pbr_functions,
        mesh_bindings::mesh,
    },
    prepass::io as prepass_io,
};

import engine::core::render::world::dither::dither_discard;

struct Toon {
    shown: f32,
    outlined: f32,
    padding: vec2<f32>,
}

@group(3) @binding(100) var<uniform> toon: Toon;

// The standard prepass, less normal maps the style never uses, with the fade's dither: a pixel the
// main pass discards must not write depth either, or it would hide what lies behind it.
@if(PREPASS_FRAGMENT)
@fragment
fn fragment(
    in: prepass_io::VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> prepass_io::FragmentOutput {
    dither_discard(in.position.xy, toon.shown);
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
    out.motion_vector = pbr_prepass_functions::calculate_motion_vector(in.world_position, in.previous_world_position);
    return out;
}
@else
@fragment
fn fragment(in: prepass_io::VertexOutput) {
    dither_discard(in.position.xy, toon.shown);
    pbr_prepass_functions::prepass_sample_color_and_alpha_discard(in);
}
