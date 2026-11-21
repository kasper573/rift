import bevy_pbr::render::{
    forward_io::VertexOutput,
    mesh_view_bindings::{lights, view},
    mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_VOLUMETRIC_BIT,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::PbrInput,
};

// How brightly a glowing face shines, in candela per square metre: well over what the sun lights
// a white face to, so it blooms.
const GLOW_NITS: f32 = 3000.0;
// A vertex colour's alpha under this marks a face that glows with its own colour instead of being
// lit.
const GLOWING: f32 = 0.5;
// How much shorter than a unit the normal handed to Bevy's lighting is, see `lighting`.
const HAIR_SHORT: f32 = 0.999999;

// DEPRECATED: a temporary stand-in for Bevy's `apply_pbr_lighting`, to be deleted once
// https://github.com/bevyengine/bevy/pull/26335 ships in the Bevy this engine builds on; every
// world shader lights through it until then. Bevy takes pow(1 - N·V, 5) with N·V unclamped above,
// so a face turned square to the camera now and then rounds it past 1: a pow of a negative base is
// undefined in WGSL and comes out NaN, which bloom spreads into a flash over the screen. A normal a hair short of a unit keeps N·V
// under 1 and lights alike otherwise.
fn lighting(input: PbrInput) -> vec4<f32> {
    var shortened = input;
    shortened.N = input.N * HAIR_SHORT;
    return apply_pbr_lighting(shortened);
}

// The way toward the light that casts the world's shadows, the sun by day and the moon by night:
// of the sky's lights, the one whose shafts show in the air. Straight up where none shines.
fn caster_way() -> vec3<f32> {
    var way = vec3<f32>(0.0, 1.0, 0.0);
    for (var index = 0u; index < lights.n_directional_lights; index++) {
        let light = &lights.directional_lights[index];
        way = (*light).direction_to_light;
        if ((*light).flags & DIRECTIONAL_LIGHT_FLAGS_VOLUMETRIC_BIT) != 0u {
            break;
        }
    }
    return way;
}

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
        color = lighting(pbr_input);
    }
    return main_pass_post_lighting_processing(pbr_input, color);
}
