// A prop drawn from the arenas: each instance a command draws is the culling kernel's, standing
// where its record says, turned about the up axis and grown by the scale the kernel drew it at,
// cleared as far as the kernel says where it hides the camera's subject, and faded in as far as
// the kernel says where its model and its impostor give way to each other. The model's vertices
// are the slab's, in the model's own metres.

import engine::core::render::world::clearing::{clear_at, dither};

struct Instance {
    x: f32,
    y: f32,
    z: f32,
    yaw: f32,
    scale: f32,
    prop: u32,
    seed: u32,
    chunk: u32,
}

@group(2) @binding(100) var<storage, read> instances: array<Instance>;
// Each instance the commands draw, by its place among `instances`, its scale, how far it shows,
// and how far it fades in with whether the eye stands inside it.
@group(2) @binding(101) var<storage, read> drawn: array<vec4<u32>>;

// Where the camera's subject shows on the picture, and how many pixels round it a prop in the way
// clears.
struct Clearing {
    focus: vec2<f32>,
    focus_radius: f32,
}

@group(2) @binding(102) var<uniform> clearing: Clearing;

// Where an instance stands: its place, the quaternion it is turned by, and its size.
struct Placement {
    at: vec3<f32>,
    turn: vec4<f32>,
    scale: f32,
}

// Where the instance a command draws as its `index`th stands.
fn placement(index: u32) -> Placement {
    let listed = drawn[index];
    let instance = instances[listed.x];
    let half = 0.5 * instance.yaw;
    return Placement(
        vec3<f32>(instance.x, instance.y, instance.z),
        vec4<f32>(0.0, sin(half), 0.0, cos(half)),
        bitcast<f32>(listed.y),
    );
}

// The seed of the instance a command draws as its `index`th.
fn seed_of(index: u32) -> u32 {
    return instances[drawn[index].x].seed;
}

// The prop of the instance a command draws as its `index`th, by its place among the table's.
fn prop_of(index: u32) -> u32 {
    return instances[drawn[index].x].prop;
}

// How far the instance a command draws as its `index`th shows, whether the eye stands inside it
// (1 or more for inside), and how far it fades in.
fn shown_of(index: u32) -> vec3<f32> {
    let listed = drawn[index];
    return vec3<f32>(bitcast<f32>(listed.z), f32(listed.w & 1u), f32(listed.w >> 1u) / 65535.0);
}

// Drops the pixel at `pixel` of an instance's model `shown` far, as `shown_of` says: a circle
// round the subject cleared, or all of it while the eye stands inside it, and the share of it not
// yet faded in.
fn clear_instance(pixel: vec2<f32>, shown: vec3<f32>) {
    if dither(pixel + FADE_OFFSET) >= shown.z {
        discard;
    }
    clear_round(pixel, shown);
}

// Drops the pixel at `pixel` of an instance's impostor `shown` far, as `clear_instance` drops its
// model's: the pixels the model keeps while the two fade into each other are the ones the
// impostor drops, so no pixel of the band shows both or neither.
fn clear_impostor(pixel: vec2<f32>, shown: vec3<f32>) {
    if 1.0 - dither(pixel + FADE_OFFSET) > shown.z {
        discard;
    }
    clear_round(pixel, shown);
}

// The fade's dither is stepped off the clearing's, so the two never drop the same pixels.
const FADE_OFFSET: vec2<f32> = vec2<f32>(17.0, 29.0);

fn clear_round(pixel: vec2<f32>, shown: vec3<f32>) {
    let radius = select(clearing.focus_radius, 3.4e38, shown.y > 0.5);
    clear_at(pixel, shown.x, clearing.focus, radius);
}

// `way` turned by the quaternion `turn`.
fn turned(turn: vec4<f32>, way: vec3<f32>) -> vec3<f32> {
    return way + 2.0 * cross(turn.xyz, cross(turn.xyz, way) + turn.w * way);
}

// Where a point of the model at `local` stands in the world, for an instance standing at `placed`.
fn world_of(placed: Placement, local: vec3<f32>) -> vec3<f32> {
    return placed.at + turned(placed.turn, local * placed.scale);
}
