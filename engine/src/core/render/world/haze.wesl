// Fog lying in banks on the ground, laid over what the camera sees, the sky included: thicker
// air, thickest at each bank's floor and thinning upward, so a pixel is fogged by how much of
// that air its line of sight crosses on the way to what it shows, and a hilltop rises out of a
// bank its foot stands in. The fog is lit by the sky it lies under, as the sky's picture shows
// it widely round the way looked, and by the sun and the moon, whose light it throws onward;
// under a dark sky, which has no light to give it, by the light each bank sheds itself.

import bevy_render::view::View;
import engine::core::render::world::sky_picture::sky_round;

struct HazeLight {
    toward: vec3<f32>,
    shed: vec3<f32>,
}

struct HazeBox {
    low: vec2<f32>,
    high: vec2<f32>,
    floor: f32,
    fall: f32,
    thickest: f32,
    edge: f32,
    glow: vec3<f32>,
}

struct Haze {
    lights: array<HazeLight, 2>,
    boxes: array<HazeBox, 32>,
    count: u32,
    under_sky: u32,
}

struct FullscreenVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var<uniform> haze: Haze;
@group(0) @binding(2) var depth: texture_depth_2d;
@group(0) @binding(3) var sky_picture: texture_2d<f32>;
@group(0) @binding(4) var sky_sampler: sampler;

const TAU: f32 = 6.2831855;
// The depth the sky is taken to lie at: far enough that a line of sight to it has left every
// bank it will ever leave.
const SKY_DEPTH: f32 = 1.0e-7;
// Less fog than this along a line of sight shows nothing.
const NO_FOG: f32 = 1.0e-4;
// How much of the light a droplet scatters goes on the way it came, -1 all back to 1 all on.
const ONWARD: f32 = 0.8;
// How far the way the fog reads the sky by is lifted off the horizon, as a share of the way's
// own length, and how coarse a size of the sky's picture it reads there: one whose pixels each
// span a good part of the sky.
const SKY_LIFT: f32 = 0.3;
const SKY_WIDELY: f32 = 4.5;
// Where along a stretch a bank's sides are weighed, and how much each place counts.
const WEIGHED_AT: array<f32, 4> = array<f32, 4>(0.0694318, 0.3300095, 0.6699905, 0.9305682);
const WEIGHS: array<f32, 4> = array<f32, 4>(0.1739274, 0.3260726, 0.3260726, 0.1739274);
@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    if haze.count == 0u {
        discard;
    }
    let deep = max(textureLoad(depth, vec2<i32>(in.position.xy), 0), SKY_DEPTH);
    let clip = vec4<f32>(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0, deep, 1.0);
    let seen = view.world_from_clip * clip;
    let eye = view.world_position;
    let way = seen.xyz / seen.w - eye;
    var crossed = 0.0;
    var shed = vec3<f32>(0.0);
    for (var index = 0u; index < haze.count; index++) {
        let fog = fog_crossed(haze.boxes[index], eye, way);
        crossed += fog;
        shed += haze.boxes[index].glow * fog;
    }
    if crossed < NO_FOG {
        discard;
    }
    let looking = normalize(way);
    var light = shed / crossed;
    if haze.under_sky != 0u {
        light += sky_round_about(looking);
    }
    for (var index = 0u; index < 2u; index++) {
        light += haze.lights[index].shed * thrown_on(dot(looking, haze.lights[index].toward));
    }
    let cover = 1.0 - exp(-crossed);
    return vec4<f32>(light * view.exposure * cover, cover);
}

// The fog of `bank` between the eye and the end of `way`: thickest at the bank's floor, falling
// to a third (by e) every `fall` metres up, summed along the way by the closed form of that
// fall, and thinned toward the bank's sides.
fn fog_crossed(bank: HazeBox, eye: vec3<f32>, way: vec3<f32>) -> f32 {
    let across = span(eye.x, way.x, bank.low.x, bank.high.x);
    let along = span(eye.z, way.z, bank.low.y, bank.high.y);
    let enter = max(max(across.x, along.x), 0.0);
    let leave = min(min(across.y, along.y), 1.0);
    if leave <= enter {
        return 0.0;
    }
    let run = length(way) * (leave - enter);
    // Ground under the floor is taken to lie at it.
    let entered = max(eye.y + way.y * enter - bank.floor, 0.0) / bank.fall;
    let left = max(eye.y + way.y * leave - bank.floor, 0.0) / bank.fall;
    let rise = left - entered;
    var thick = exp(-(entered + left) / 2.0);
    // A level line crosses air of one thickness, where the closed form divides by nothing.
    if abs(rise) > 1.0e-3 {
        thick = (exp(-entered) - exp(-left)) / rise;
    }
    var sides = 0.0;
    for (var index = 0; index < 4; index++) {
        let at = eye.xz + way.xz * mix(enter, leave, WEIGHED_AT[index]);
        let inside = min(at - bank.low, bank.high - at) / max(bank.edge, 1.0e-3);
        sides += WEIGHS[index] * clamp(inside.x, 0.0, 1.0) * clamp(inside.y, 0.0, 1.0);
    }
    return bank.thickest * thick * run * sides;
}

// The share of the way from `start`, going `step` along, that lies between `low` and `high`.
fn span(start: f32, step: f32, low: f32, high: f32) -> vec2<f32> {
    if abs(step) < 1e-6 {
        if start >= low && start <= high {
            return vec2<f32>(0.0, 1.0);
        }
        return vec2<f32>(1.0, 0.0);
    }
    let a = (low - start) / step;
    let b = (high - start) / step;
    return vec2<f32>(min(a, b), max(a, b));
}

// The light of the sky round `looking`, in candela per square metre: the sky's picture read
// toward the way flattened to the horizon, at a size so coarse that a read takes in a wide
// stretch of sky and no cloud's outline shows in the fog it lights.
fn sky_round_about(looking: vec3<f32>) -> vec3<f32> {
    let middle = normalize(vec3<f32>(looking.x, SKY_LIFT, looking.z));
    return textureSampleLevel(sky_picture, sky_sampler, sky_round(middle), SKY_WIDELY).rgb;
}

// How much of a light a droplet throws toward the eye, looking `cosine` of the way toward the
// light.
fn thrown_on(cosine: f32) -> f32 {
    let bent = 1.0 + ONWARD * ONWARD - 2.0 * ONWARD * cosine;
    return (1.0 - ONWARD * ONWARD) / (2.0 * TAU * bent * sqrt(bent));
}
