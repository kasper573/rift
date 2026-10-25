import bevy_pbr::render::{
    forward_io::VertexOutput,
    mesh_view_bindings::view,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
};

// How brightly a glowing face shines, in candela per square metre: well over what the sun lights
// a white face to, so it blooms.
const GLOW_NITS: f32 = 3000.0;
// A vertex colour's alpha under this marks a face that glows with its own colour instead of being
// lit.
const GLOWING: f32 = 0.5;

// The colour of a pixel of a standard material in the world's light, or glowing with its own
// colour when its vertices say so; fogged and finished the way every pixel of the world is.
fn lit_color(in: VertexOutput, is_front: bool) -> vec4<f32> {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.material.base_color = alpha_discard(
        pbr_input.material.flags,
        pbr_input.material.alpha_cutoff,
        pbr_input.material.base_color
    );
    let base = pbr_input.material.base_color;
    var color: vec4<f32>;
    if base.a < GLOWING {
        pbr_input.material.base_color.a = 1.0;
        color = vec4<f32>(base.rgb * GLOW_NITS * view.exposure, 1.0);
    } else {
        color = apply_pbr_lighting(pbr_input);
    }
    return main_pass_post_lighting_processing(pbr_input, color);
}
