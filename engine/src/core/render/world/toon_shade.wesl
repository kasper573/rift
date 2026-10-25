import bevy_pbr::render::{
    forward_io::VertexOutput,
    pbr_fragment::pbr_input_from_standard_material,
    pbr_functions::{alpha_discard, apply_pbr_lighting, main_pass_post_lighting_processing},
    pbr_types::STANDARD_MATERIAL_FLAGS_UNLIT_BIT,
};

// Light levels at which a face steps up from shade to half light and from half light to full.
const HALF_LIT: f32 = 0.32;
const FULL_LIT: f32 = 0.68;
const SHADE: f32 = 0.5;
const HALF: f32 = 0.78;

// The colour of a pixel of a standard material lit in three bands: the face is lit as a flat one
// (or from its vertex normals when `flat` is 0), its light measured on white and stepped, and the
// material's own colour laid over that.
fn toon_color(in: VertexOutput, is_front: bool, flat: f32) -> vec4<f32> {
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    pbr_input.material.base_color = alpha_discard(
        pbr_input.material.flags,
        pbr_input.material.alpha_cutoff,
        pbr_input.material.base_color
    );
    let base = pbr_input.material.base_color;
    let emissive = pbr_input.material.emissive.rgb;
    var color = base.rgb;
    if (pbr_input.material.flags & STANDARD_MATERIAL_FLAGS_UNLIT_BIT) == 0u {
        if flat > 0.5 {
            let plane = normalize(cross(dpdy(in.world_position.xyz), dpdx(in.world_position.xyz)));
            let facing = select(-plane, plane, dot(plane, pbr_input.N) >= 0.0);
            pbr_input.N = facing;
            pbr_input.world_normal = facing;
        }
        pbr_input.material.base_color = vec4<f32>(1.0, 1.0, 1.0, base.a);
        pbr_input.material.emissive = vec4<f32>(0.0, 0.0, 0.0, 1.0);
        let lit = apply_pbr_lighting(pbr_input).rgb;
        let level = dot(lit, vec3<f32>(0.2126, 0.7152, 0.0722));
        let band = select(select(SHADE, HALF, level > HALF_LIT), 1.0, level > FULL_LIT);
        let tint = lit / max(level, 1e-3);
        color = base.rgb * band * min(tint, vec3<f32>(1.25));
    }
    color += emissive;
    return main_pass_post_lighting_processing(pbr_input, vec4<f32>(color, base.a));
}
