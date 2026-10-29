// The header of a shader that paints an area's sky: over the whole sky behind the world, every way
// the camera looks, blended over the air. The shader says what colour the sky is that way and how
// much it covers the air (`vfx`, given where the way lies on the sky: x round the horizon from
// north, y down from the zenith), reading the way itself (`look`, a unit vector up the y axis);
// the way to the light the world is lit by and the light it sheds (`light_way`, `light_color`:
// the sun, or by night the full moon, which always stands opposite it; a light below the horizon
// shines black); the way to the moon (`moon_way`) and how deep into the night the sky is
// (`night`, 0 day to 1 when every star is out); the light the sky sheds from a way (`sky_light`,
// which `sample` gives for the way looked); the clouds drifting over the area at any point of the
// world (`cloud_at`), whose shadows fall on the ground under them, the height they lie at
// (`cloud_base`) and how far the wind has carried them along the ground (`cloud_drift`); where the camera looks from (`eye`) and the weather the engine tells it of. A
// colour of 1 shines as a white face square to the sun does.
import bevy_pbr::render::mesh_view_bindings;
import bevy_pbr::render::mesh_view_bindings::{view, lights, light_probes};
import bevy_pbr::render::view_transformations::position_world_to_clip;

@group(3) @binding(100) var<uniform> site: Site;
@group(3) @binding(101) var<uniform> params: Params;
@group(3) @binding(103) var<uniform> heavens: Heavens;
@group(3) @binding(104) var cloud_picture: texture_2d<f32>;
@group(3) @binding(105) var cloud_sampler: sampler;

struct Heavens {
    toward: vec3<f32>,
    shining: f32,
    color: vec4<f32>,
    moon: vec3<f32>,
    night: f32,
    cloud_base: f32,
    cloud_tile: f32,
    drift: vec2<f32>,
}

struct SkyOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) way: vec3<f32>,
}

// What a colour of 1 shines at, in candela per square metre: what the sun lights a white face
// square to it to.
const SKY_NITS: f32 = 830.0;
const SKY_TAU: f32 = 6.2831855;
const SKY_PI: f32 = 3.1415927;

var<private> looking: vec3<f32>;

fn eye() -> vec3<f32> {
    return view.world_position;
}

fn look() -> vec3<f32> {
    return looking;
}

fn light_way() -> vec3<f32> {
    return heavens.toward;
}

fn light_color() -> vec3<f32> {
    return heavens.color.rgb * heavens.shining;
}

fn moon_way() -> vec3<f32> {
    return heavens.moon;
}

fn night() -> f32 {
    return heavens.night;
}

fn cloud_base() -> f32 {
    return heavens.cloud_base;
}

fn cloud_drift() -> vec2<f32> {
    return heavens.drift;
}

fn sample(at: vec2<f32>) -> vec4<f32> {
    return vec4<f32>(sky_light(looking), 1.0);
}

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

// The field of cloud at a point of the world: how cloudy the sky is there, 0 clear to 1 thick, in
// x, and how far inside the cloud's edge the point lies in y, in the noise's own measure that
// the field is drawn from, negative outside it. The field lies at the clouds' height
// (`cloud_base`), drifting on the wind, and the sun casts its shadows where its rays carry them.
fn cloud_at(point: vec3<f32>) -> vec2<f32> {
    let at = (point.xz - heavens.drift) / heavens.cloud_tile;
    let field = textureSampleLevel(cloud_picture, cloud_sampler, at, 0.0).rg;
    return vec2<f32>(field.x, field.y * 2.0 - 1.0);
}

// The clouds lie at the far end of every ray, behind whatever the world draws.
@vertex
fn vertex(@location(0) position: vec3<f32>) -> SkyOutput {
    var out: SkyOutput;
    out.way = position;
    var clip = position_world_to_clip(view.world_position + position);
    clip.z = 0.0;
    out.position = clip;
    return out;
}

@fragment
fn fragment(in: SkyOutput) -> @location(0) vec4<f32> {
    looking = normalize(in.way);
    read_weather();
    let uv = vec2<f32>(
        fract(atan2(looking.x, -looking.z) / SKY_TAU + 1.0),
        0.5 - asin(clamp(looking.y, -1.0, 1.0)) / SKY_PI,
    );
    let clouds = vfx(uv);
    return vec4<f32>(clouds.rgb * SKY_NITS * view.exposure, clouds.a);
}
