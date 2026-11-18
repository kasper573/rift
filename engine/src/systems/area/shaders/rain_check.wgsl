// The rain header as the validator sees it, in plain WGSL: the same bindings and helpers, and an
// entry that calls what a rain shader must define, `vfx`.
const SKY_NITS: f32 = 830.0;
@group(3) @binding(100) var<uniform> site: Site;
@group(3) @binding(101) var<uniform> params: Params;

var<private> looking: vec3<f32>;
var<private> shell_distance: f32;

fn eye() -> vec3<f32> {
    return vec3<f32>(0.0);
}

fn look() -> vec3<f32> {
    return looking;
}

fn shell() -> f32 {
    return shell_distance;
}

fn point() -> vec3<f32> {
    return looking * shell_distance;
}

fn behind() -> vec3<f32> {
    return looking * 100000.0;
}

fn sample(at: vec2<f32>) -> vec4<f32> {
    return vec4<f32>(sky_light(looking), 1.0);
}

fn sky_light(way: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(0.5) / SKY_NITS;
}

@fragment
fn fragment(@location(0) way: vec3<f32>, @location(1) uv: vec2<f32>) -> @location(0) vec4<f32> {
    shell_distance = length(way);
    looking = normalize(way);
    read_weather();
    return vfx(uv);
}
