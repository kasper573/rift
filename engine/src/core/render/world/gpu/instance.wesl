// A prop drawn from the arenas: each instance a command draws is the culling kernel's, standing
// where its record says, turned about the up axis and grown by the scale the kernel drew it at,
// cleared as far as the kernel says where it hides the camera's subject, and faded in as far as
// the kernel says where its model and its impostor give way to each other. The model's vertices
// are the slab's, in the model's own metres.

import engine::core::render::world::clearing::{clear_at, dither};
@if(MARK)
import bevy_pbr::render::mesh_view_bindings::view;

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

// The slot of the instances' scenery among those the view draws, while the instances write their
// ids for a view that tells what it points at.
@if(PICK)
@group(2) @binding(106) var<uniform> slot: u32;

// The sets of outlined instances each of the arena's instances is in, two bits an instance, the
// first set's lower: while the instances of the sets draw their whole shapes for a view that
// outlines them, on several samples a pixel. An instance past the end is in neither.
@if(MARK)
@group(2) @binding(107) var<storage, read> marks: array<u32>;

// The sets holding the instance a command draws as its `index`th.
@if(MARK)
fn marks_of(index: u32) -> u32 {
    let instance = drawn[index].x;
    let word = instance >> 4u;
    if word >= arrayLength(&marks) {
        return 0u;
    }
    return (marks[word] >> ((instance & 15u) * 2u)) & 3u;
}

// Where a point at `world` of an instance the sets `held` hold shows on the picture: nowhere for
// an instance in neither, and without the jitter the view is drawn with from frame to frame, as
// the outline is laid over the picture the frames have settled into.
@if(MARK)
fn mark_clip(held: u32, world: vec3<f32>) -> vec4<f32> {
    if held == 0u {
        return vec4<f32>(0.0);
    }
    return view.unjittered_clip_from_world * vec4<f32>(world, 1.0);
}

// What a pixel of an instance the sets `held` hold writes: each set its own channel.
@if(MARK)
fn marked(held: u32) -> vec4<f32> {
    return vec4<f32>(f32(held & 1u), f32((held >> 1u) & 1u), 0.0, 1.0);
}

// The id of the instance a command draws as its `index`th: its place among the arena's plus one,
// under its scenery's slot, so 0 is nothing.
@if(PICK)
fn id_of(index: u32) -> u32 {
    return (slot << 24u) | (drawn[index].x + 1u);
}

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
