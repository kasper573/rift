// The heavens as a shader under the open sky reads them: the light the world is lit by, the moon
// and the night, and the field of cloud drifting over the area, which holds water and rains.

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
    water_edge: f32,
    rain_lean: vec2<f32>,
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

// The field of cloud at a point of the world: how cloudy the sky is there, 0 clear to 1 thick, in
// x, and how far inside the cloud's edge the point lies in y, in the noise's own measure that
// the field is drawn from, negative outside it. The field lies at the clouds' height
// (`cloud_base`), drifting on the wind, and the sun casts its shadows where its rays carry them.
fn cloud_at(point: vec3<f32>) -> vec2<f32> {
    let inside = cloud_read(point, 0) - heavens.cloud_edge;
    return vec2<f32>(smoothstep(0.0, heavens.cloud_softness, inside), inside);
}

// How much water the cloud at a point of the world holds, 0 none to 1 as much as a storm cloud's:
// none where there is no cloud.
fn cloud_water_at(point: vec3<f32>) -> f32 {
    let wet = cloud_read(point, 1);
    return cloud_at(point).x * smoothstep(heavens.water_edge, heavens.water_edge + heavens.cloud_softness, wet);
}

// How far the wind carries the rain sideways for every metre it falls (x east, z south).
fn rain_lean() -> vec2<f32> {
    return heavens.rain_lean;
}

// How hard it rains at a point of the world, 0 none to 1 a cloudburst: as hard as the rain field
// tells of the ground under the point, which already falls from the cloud upwind of it as the lie
// of the land shares the downpour out. The rain seen falling is so the rain that wets the ground,
// rings the water and is heard, wherever the clouds' own edges lie between the field's cells.
fn rain_under(point: vec3<f32>) -> f32 {
    return rain_field(point.xz).z;
}

// The noise the field is drawn from in `channel`, its cloud in 0 and its water in 1, read where
// the point lies under it twice and blended, so the clouds never show where the noise repeats.
fn cloud_read(point: vec3<f32>, channel: i32) -> f32 {
    let at = (point.xz - heavens.drift) / heavens.cloud_tile;
    let first = textureSampleLevel(cloud_picture, cloud_sampler, at, 0.0)[channel];
    let second = textureSampleLevel(cloud_picture, cloud_sampler, heavens.cloud_second * at + heavens.cloud_shift, 0.0)[channel];
    return 0.5 + (first + second - 1.0) * 0.70710678;
}
