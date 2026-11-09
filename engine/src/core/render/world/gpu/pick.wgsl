// What each pixel of a view shows of an arena's instances, by id: every instance a view's commands
// draw, standing where its record says, writes its id where it is the surface the view's own depth
// shows there. A pixel a cut-out texture left out, or one where something else stands nearer,
// writes nothing. An id is the instance's place among the arena's plus one, under the arena's slot
// among those the view draws, so 0 is nothing.

struct PickView {
    clip_from_world: mat4x4<f32>,
    slot: u32,
}

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

@group(0) @binding(0) var<uniform> view: PickView;
@group(0) @binding(1) var<storage, read> instances: array<Instance>;
@group(0) @binding(2) var<storage, read> drawn: array<vec4<u32>>;
@group(0) @binding(3) var scene_depth: texture_depth_2d;

// How far a point may lie off the surface the view shows there, as a share of how far off the eye
// either lies, and still be that surface: room for a surface's own stages bending it a little.
const SAME_SURFACE: f32 = 0.01;

struct Picked {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) id: u32,
}

@vertex
fn vertex(@builtin(instance_index) index: u32, @location(0) local: vec3<f32>) -> Picked {
    let listed = drawn[index];
    let instance = instances[listed.x];
    let half = 0.5 * instance.yaw;
    let turn = vec4<f32>(0.0, sin(half), 0.0, cos(half));
    let way = local * bitcast<f32>(listed.y);
    let turned = way + 2.0 * cross(turn.xyz, cross(turn.xyz, way) + turn.w * way);
    let world = vec3<f32>(instance.x, instance.y, instance.z) + turned;
    var out: Picked;
    out.position = view.clip_from_world * vec4<f32>(world, 1.0);
    out.id = (view.slot << 24u) | (listed.x + 1u);
    return out;
}

@fragment
fn fragment(in: Picked) -> @location(0) u32 {
    let shown = textureLoad(scene_depth, vec2<i32>(in.position.xy), 0);
    if abs(in.position.z - shown) > max(in.position.z, shown) * SAME_SURFACE {
        discard;
    }
    return in.id;
}
