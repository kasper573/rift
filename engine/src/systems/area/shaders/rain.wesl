// The header of a shader that draws an area's rain: on shells round the eye, each a sphere at a
// distance the area's rainfall names, drawn from the farthest in, every way the camera looks,
// blended over the world in front of which it falls. The shader says what colour the rain is at
// a point of a shell and how much it covers what lies behind (`vfx`, given where the way lies on
// the sky: x round the horizon from north, y down from the zenith), reading the point itself
// (`point`), the shell's distance from the eye (`shell`), where the camera looks from and the way
// it looks (`eye`, `look`, a unit vector up the y axis) and the surface of the world behind the
// point, where rain splashes (`behind`); how hard it rains there (`rain_under`, 0 none to 1 a
// cloudburst, `raining` and `sheltered` with it, as the weather's helpers tell), the wind, the
// moment and lightning's flash (`weather`); and the light the rain is lit by, as the sky's header
// tells it (`light_way`, `light_color`, `sky_light`, `night`). A colour of 1 shines as a white
// face square to the sun does.
import bevy_pbr::render::mesh_view_bindings::{globals, view};
import bevy_pbr::render::view_transformations::{frag_coord_to_ndc, position_ndc_to_world, position_world_to_clip};
@if(DEPTH_PREPASS)
import bevy_pbr::prepass::utils::prepass_depth;

@group(3) @binding(100) var<uniform> site: Site;
@group(3) @binding(101) var<uniform> params: Params;

struct RainOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) way: vec3<f32>,
}

const RAIN_TAU: f32 = 6.2831855;
const RAIN_PI: f32 = 3.1415927;
// How far off `behind` puts the surface of a pixel through which the sky shows.
const OPEN_SKY: f32 = 100000.0;

var<private> looking: vec3<f32>;
var<private> shell_distance: f32;
var<private> pixel: vec4<f32>;

fn eye() -> vec3<f32> {
    return view.world_position;
}

fn look() -> vec3<f32> {
    return looking;
}

fn shell() -> f32 {
    return shell_distance;
}

fn point() -> vec3<f32> {
    return view.world_position + looking * shell_distance;
}

// The point of the world's surface the pixel shows behind the rain, or one far off along the way
// looked where it shows the sky.
fn behind() -> vec3<f32> {
    @if(DEPTH_PREPASS) {
        let depth = prepass_depth(pixel, 0u);
        if depth > 0.0 {
            return position_ndc_to_world(vec3<f32>(frag_coord_to_ndc(pixel).xy, depth));
        }
    }
    return view.world_position + looking * OPEN_SKY;
}

fn sample(at: vec2<f32>) -> vec4<f32> {
    return vec4<f32>(sky_light(looking), 1.0);
}

@vertex
fn vertex(@location(0) position: vec3<f32>) -> RainOutput {
    var out: RainOutput;
    out.way = position;
    out.position = position_world_to_clip(view.world_position + position);
    return out;
}

@fragment
fn fragment(in: RainOutput) -> @location(0) vec4<f32> {
    shell_distance = length(in.way);
    looking = in.way / max(shell_distance, 1e-6);
    pixel = in.position;
    read_weather();
    let uv = vec2<f32>(
        fract(atan2(looking.x, -looking.z) / RAIN_TAU + 1.0),
        0.5 - asin(clamp(looking.y, -1.0, 1.0)) / RAIN_PI,
    );
    let rain = vfx(uv);
    return vec4<f32>(rain.rgb * SKY_NITS * view.exposure, rain.a);
}
