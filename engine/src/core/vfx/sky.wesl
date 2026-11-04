// The header of a shader that paints an area's sky: over the whole sky behind the world, every way
// the camera looks, blended over the air. The shader says what colour the sky is that way and how
// much it covers the air (`vfx`, given where the way lies on the sky: x round the horizon from
// north, y down from the zenith), reading the way itself (`look`, a unit vector up the y axis);
// the way to the light the world is lit by and the light it sheds (`light_way`, `light_color`:
// the sun, or by night the full moon, which always stands opposite it; a light below the horizon
// shines black); the way to the moon (`moon_way`) and how deep into the night the sky is
// (`night`, 0 day to 1 when every star is out); the light the sky sheds from a way (`sky_light`,
// which `sample` gives for the way looked); the clouds drifting over the area at any point of the
// world (`cloud_at`), whose shadows fall on the ground under them, how much water the cloud at a
// point holds (`cloud_water_at`, 0 none to 1 a storm cloud's) and how hard it rains under it
// (`rain_under`, 0 none to 1 a cloudburst), the height they lie at (`cloud_base`) and how far the
// wind has carried them along the ground (`cloud_drift`); where the camera looks from (`eye`) and
// the weather the engine tells it of. A colour of 1 shines as a white face square to the sun does.
import bevy_pbr::render::mesh_view_bindings::view;
import bevy_pbr::render::view_transformations::position_world_to_clip;

@group(3) @binding(100) var<uniform> site: Site;
@group(3) @binding(101) var<uniform> params: Params;

struct SkyOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) way: vec3<f32>,
}

const SKY_TAU: f32 = 6.2831855;
const SKY_PI: f32 = 3.1415927;

var<private> looking: vec3<f32>;

fn eye() -> vec3<f32> {
    return view.world_position;
}

fn look() -> vec3<f32> {
    return looking;
}

fn sample(at: vec2<f32>) -> vec4<f32> {
    return vec4<f32>(sky_light(looking), 1.0);
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
