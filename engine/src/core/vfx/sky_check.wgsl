// The sky header as the validator sees it, in plain WGSL: the same bindings and helpers, and an
// entry that calls what a sky shader must define, `vfx`.
const SKY_NITS: f32 = 830.0;
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
    cloud_edge: f32,
    cloud_softness: f32,
    cloud_shift: vec2<f32>,
    cloud_second: mat2x2<f32>,
}

var<private> looking: vec3<f32>;

fn eye() -> vec3<f32> {
    return vec3<f32>(0.0);
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

fn sky_light(way: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(0.5) / SKY_NITS;
}

// The field of cloud at a point of the world: how cloudy the sky is there, 0 clear to 1 thick, in
// x, and how far inside the cloud's edge the point lies in y, in the noise's own measure that
// the field is drawn from, negative outside it. The field lies at the clouds' height
// (`cloud_base`), drifting on the wind, and the sun casts its shadows where its rays carry them.
fn cloud_at(point: vec3<f32>) -> vec2<f32> {
    let at = (point.xz - heavens.drift) / heavens.cloud_tile;
    let first = cloud_noise(at);
    let second = cloud_noise(heavens.cloud_second * at + heavens.cloud_shift);
    let inside = 0.5 + (first + second - 1.0) * 0.70710678 - heavens.cloud_edge;
    return vec2<f32>(smoothstep(0.0, heavens.cloud_softness, inside), inside);
}

// The noise the field is drawn from, read where it lies `at`, in tiles, repeating every tile.
// `cloud_at` reads it twice and blends the two reads, so the clouds never show where it repeats.
fn cloud_noise(at: vec2<f32>) -> f32 {
    return textureSampleLevel(cloud_picture, cloud_sampler, at, 0.0).r;
}
@fragment
fn fragment(@location(0) way: vec3<f32>, @location(1) uv: vec2<f32>) -> @location(0) vec4<f32> {
    looking = normalize(way);
    read_weather();
    return vfx(uv);
}
