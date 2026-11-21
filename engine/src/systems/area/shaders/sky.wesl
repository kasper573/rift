// The header of a shader that paints an area's sky: over the whole sky behind the world, every way
// the camera looks, laid over the air. The shader says what light it lays over the air that way,
// already weighed by how much of the pixel it covers, and how much of the air behind it that
// hides (`vfx`, given where the way lies on the sky: x round the horizon from north, y down from
// the zenith; a star adds its light and hides nothing, a cloud hides what it covers), reading
// the way itself (`look`, a unit vector up the y axis);
// the way to the light the world is lit by and the light it sheds on the ground (`light_way`,
// `light_color`: the sun, or by night the full moon, which always stands opposite it, dimmed and
// reddened by the air it crossed; a light below the horizon shines black); the way to the sun and
// the light it sheds on the clouds, which it still reaches for a while after the ground is in
// shade (`sun_way`, `sun_color`); the way to the moon (`moon_way`), how wide its disc is, in
// radians (`moon_size`), and how deep into the night the sky is (`night`, 0 day to 1 when every
// star is out); how much of each colour of a light
// from beyond the air reaches the eye from a way, none from below the horizon (`through_air`,
// which a star, a moon or anything else out there is dimmed and reddened by); a way as the
// turning sky has it at this hour, the sky wheeling with its sun (`sky_turned`, what stars are
// laid out by so they rise and set); how much of the sky one pixel spans, in radians
// (`pixel_angle`, so a point of light can be drawn a pixel wide through any lens); whether the
// stars twinkle (`twinkles`, 1, or 0 for a player who holds them still); the light the sky sheds
// from a way (`sky_light`, which `sample` gives for the way looked); the clouds drifting over the area at any point of the
// world (`cloud_at`), whose shadows fall on the ground under them, how much water the cloud at a
// point holds (`cloud_water_at`, 0 none to 1 a storm cloud's) and how hard it rains under it
// (`rain_under`, 0 none to 1 a cloudburst), the height they lie at (`cloud_base`) and how far the
// wind has carried them along the ground (`cloud_drift`); where the camera looks from (`eye`) and
// the weather the engine tells it of. A colour of 1 shines as a white face square to the sun does.
import bevy_pbr::render::mesh_view_bindings::{globals, view};
import bevy_pbr::render::view_transformations::position_world_to_clip;
@if(SKY_PICTURE)
import engine::core::render::world::sky_picture::sky_way;

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

@if(!SKY_PICTURE)
fn pixel_angle() -> f32 {
    return 2.0 / (view.clip_from_view[1][1] * view.viewport.w);
}
@else
fn pixel_angle() -> f32 {
    return SKY_PI / view.viewport.z;
}

// Where a way lies on the sky, as a sky shader is given it.
fn sky_uv(way: vec3<f32>) -> vec2<f32> {
    return vec2<f32>(
        fract(atan2(way.x, -way.z) / SKY_TAU + 1.0),
        0.5 - asin(clamp(way.y, -1.0, 1.0)) / SKY_PI,
    );
}

// The clouds lie at the far end of every ray, behind whatever the world draws.
@if(!SKY_PICTURE)
@vertex
fn vertex(@location(0) position: vec3<f32>) -> SkyOutput {
    var out: SkyOutput;
    out.way = position;
    var clip = position_world_to_clip(view.world_position + position);
    clip.z = 0.0;
    out.position = clip;
    return out;
}

@if(!SKY_PICTURE)
@fragment
fn fragment(in: SkyOutput) -> @location(0) vec4<f32> {
    looking = normalize(in.way);
    read_weather();
    let sky = vfx(sky_uv(looking));
    return vec4<f32>(sky.rgb * SKY_NITS * view.exposure, sky.a);
}

// Drawn into the sky's picture, the sky is spread over the whole of the face it is drawn on, and
// laid over the air's own there, in candela per square metre whatever the eye is open to.
@if(SKY_PICTURE)
@vertex
fn vertex(@location(0) position: vec3<f32>) -> SkyOutput {
    var out: SkyOutput;
    out.position = vec4<f32>(position.xy, 0.0, 1.0);
    return out;
}

@if(SKY_PICTURE)
@fragment
fn fragment(in: SkyOutput) -> @location(0) vec4<f32> {
    looking = sky_way(in.position.xy / view.viewport.zw);
    read_weather();
    let sky = vfx(sky_uv(looking));
    return vec4<f32>(sky_air(looking) * (1.0 - sky.a) + sky.rgb * SKY_NITS, 1.0);
}
