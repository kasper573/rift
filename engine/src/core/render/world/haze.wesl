// Haze lying in boxes on the ground, laid over what the camera sees: each pixel hazes by how far
// the way from the eye to what it shows runs through each box, taken to run down evenly from the
// eye's height over its ground to the ground the point stands on; where nothing of the world
// stands, the sky shows unhazed.

import bevy_render::view::View;

struct HazeBox {
    low: vec2<f32>,
    high: vec2<f32>,
    height: f32,
    clear: f32,
    thick: f32,
    thickest: f32,
}

struct Haze {
    color: vec3<f32>,
    ground: f32,
    boxes: array<HazeBox, 32>,
    count: u32,
}

struct FullscreenVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var<uniform> haze: Haze;
@group(0) @binding(2) var depth: texture_depth_2d;

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let deep = textureLoad(depth, vec2<i32>(in.position.xy), 0);
    if deep <= 0.0 || haze.count == 0u {
        discard;
    }
    let clip = vec4<f32>(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0, deep, 1.0);
    let seen = view.world_from_clip * clip;
    let point = seen.xyz / seen.w;
    let eye = view.world_position;
    let above = max(eye.y - haze.ground, 0.0);
    var through = 1.0;
    for (var index = 0u; index < haze.count; index++) {
        through *= 1.0 - hazed(haze.boxes[index], eye, point, above);
    }
    let cover = 1.0 - through;
    return vec4<f32>(haze.color.rgb * cover, cover);
}

// How far toward the haze's colour `lying` turns what is seen at `point` from `eye`, `above` its
// ground: as far as the way runs through the box, within its rect and under its height over the
// ground.
fn hazed(lying: HazeBox, eye: vec3<f32>, point: vec3<f32>, above: f32) -> f32 {
    let way = point - eye;
    let across = span(eye.x, way.x, lying.low.x, lying.high.x);
    let along = span(eye.z, way.z, lying.low.y, lying.high.y);
    var under = 0.0;
    if above > lying.height {
        under = 1.0 - lying.height / above;
    }
    let enter = max(max(across.x, along.x), max(under, 0.0));
    let leave = min(min(across.y, along.y), 1.0);
    let inside = max(leave - enter, 0.0) * length(way);
    let thickening = clamp((inside - lying.clear) / max(lying.thick - lying.clear, 1e-3), 0.0, 1.0);
    return lying.thickest * thickening;
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
