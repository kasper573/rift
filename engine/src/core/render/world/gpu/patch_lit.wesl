import bevy_pbr::render::{
    mesh_types::MESH_FLAGS_SHADOW_RECEIVER_BIT,
    mesh_view_bindings::view,
    pbr_functions::{calculate_view, main_pass_post_lighting_processing},
    pbr_types::{pbr_input_new, STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT},
    view_transformations::position_world_to_clip,
};
import engine::core::render::world::gpu::patch::placed;
import engine::core::render::world::shade::lighting;

const GROUND: vec3<f32> = vec3<f32>(0.09, 0.13, 0.05);
const ROUGHNESS: f32 = 0.9;

struct Vertex {
    @location(0) position: vec3<f32>,
}

struct Placed {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_position: vec4<f32>,
}

@vertex
fn vertex(in: Vertex) -> Placed {
    let world = placed(in.position);
    return Placed(position_world_to_clip(world.xyz), world);
}

@fragment
fn fragment(in: Placed) -> @location(0) vec4<f32> {
    var input = pbr_input_new();
    input.material.base_color = vec4<f32>(GROUND, 1.0);
    input.material.perceptual_roughness = ROUGHNESS;
    input.material.flags |= STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT;
    input.flags = MESH_FLAGS_SHADOW_RECEIVER_BIT;
    input.frag_coord = in.clip;
    input.world_position = in.world_position;
    input.world_normal = vec3<f32>(0.0, 1.0, 0.0);
    input.N = input.world_normal;
    input.is_orthographic = view.clip_from_view[3].w == 1.0;
    input.V = calculate_view(in.world_position, input.is_orthographic);
    return main_pass_post_lighting_processing(input, lighting(input));
}
