import bevy_pbr::render::mesh_view_bindings;
import bevy_pbr::render::mesh_view_bindings::{lights, light_probes};

// What a colour of 1 shines at, in candela per square metre: what the sun lights a white face
// square to it to.
const SKY_NITS: f32 = 830.0;

// The light the sky sheds from `way`: the air's own, as it lights the world, where the GPU draws
// the air, over the ambient light everywhere.
fn sky_light(way: vec3<f32>) -> vec3<f32> {
    var light = lights.ambient_color.rgb;
    var toward = way;
    toward.z = -toward.z;
    @if(ENVIRONMENT_MAP && MULTIPLE_LIGHT_PROBES_IN_ARRAY) {
        if light_probes.view_cubemap_index >= 0 {
            light += textureSampleLevel(
                mesh_view_bindings::diffuse_environment_maps[light_probes.view_cubemap_index],
                mesh_view_bindings::environment_map_sampler,
                toward,
                0.0
            ).rgb * light_probes.intensity_for_view;
        }
    }
    @if(ENVIRONMENT_MAP && !MULTIPLE_LIGHT_PROBES_IN_ARRAY) {
        if light_probes.view_cubemap_index >= 0 {
            light += textureSampleLevel(
                mesh_view_bindings::diffuse_environment_map,
                mesh_view_bindings::environment_map_sampler,
                toward,
                0.0
            ).rgb * light_probes.intensity_for_view;
        }
    }
    return light / SKY_NITS;
}
