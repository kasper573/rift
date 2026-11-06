// A prop drawn from the arenas: each instance a command draws is the culling kernel's, standing
// where its record says, turned about the up axis and grown by the scale the kernel drew it at.
// The model's vertices are the slab's, in the model's own metres.

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
// Each instance the commands draw, by its place among `instances`, and its scale.
@group(2) @binding(101) var<storage, read> drawn: array<vec2<u32>>;

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

// `way` turned by the quaternion `turn`.
fn turned(turn: vec4<f32>, way: vec3<f32>) -> vec3<f32> {
    return way + 2.0 * cross(turn.xyz, cross(turn.xyz, way) + turn.w * way);
}

// Where a point of the model at `local` stands in the world, for an instance standing at `placed`.
fn world_of(placed: Placement, local: vec3<f32>) -> vec3<f32> {
    return placed.at + turned(placed.turn, local * placed.scale);
}
