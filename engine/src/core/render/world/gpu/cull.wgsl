// Which of a scenery's prop instances a view draws, and at which level of each part: one
// workgroup for each chunk the view may see, its threads striding through the chunk's instances.
// An instance the view sees is listed in the region of the command that draws its part at the
// level its size on screen asks for, as many times as its model has parts.

struct CullView {
    // The frustum's half-spaces, each facing in: left, right, bottom, top, near and far.
    planes: array<vec4<f32>, 6>,
    // Where sizes on screen are measured from, and how many pixels a metre spans a metre off it.
    eye: vec3<f32>,
    pixels_at_a_meter: f32,
    // Whether a metre spans as many pixels however far off the eye it lies.
    orthographic: u32,
    // Whether what lies behind the near half-space is left out: not for a light, whose casters
    // there still cast into what it sees.
    test_near: u32,
    // How many pixels across a thing among many alike must span to be told apart, how far a level
    // may stray from the full mesh in pixels, and how near the eye nothing is measured from.
    grain_pixels: f32,
    stray_pixels: f32,
    near: f32,
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

// A prop as the view draws it: the middle of its model's box and the radius round it holding the
// box, the box's widest span, and its parts among `parts`, none when nothing of it is drawn.
struct Prop {
    x: f32,
    y: f32,
    z: f32,
    radius: f32,
    extent: f32,
    first_part: u32,
    parts: u32,
    _unused: u32,
}

// A part's levels among `levels`, finest first.
struct Part {
    first_level: u32,
    levels: u32,
}

// How far a level strays from the full mesh, in the model's own units, and the command drawing it.
struct Level {
    strays: f32,
    command: u32,
}

struct Command {
    index_count: u32,
    instance_count: atomic<u32>,
    first_index: u32,
    base_vertex: i32,
    first_instance: u32,
}

struct Commands {
    drawn: atomic<u32>,
    commands: array<Command>,
}

@group(0) @binding(0) var<uniform> view: CullView;
@group(0) @binding(1) var<storage, read> instances: array<Instance>;
// The chunks the view may see: each one's first instance among `instances`, and how many it has.
@group(0) @binding(2) var<storage, read> chunks: array<vec2<u32>>;
@group(0) @binding(3) var<storage, read> props: array<Prop>;
@group(0) @binding(4) var<storage, read> parts: array<Part>;
@group(0) @binding(5) var<storage, read> levels: array<Level>;
@group(0) @binding(6) var<storage, read_write> commands: Commands;
// Each drawn instance by its place among `instances`, and the scale it is drawn at.
@group(0) @binding(7) var<storage, read_write> drawn: array<vec2<u32>>;

const THREADS: u32 = 64u;

@compute @workgroup_size(64)
fn cull(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_index) thread: u32) {
    let chunk = chunks[group.x];
    for (var at = thread; at < chunk.y; at += THREADS) {
        cull_instance(chunk.x + at);
    }
}

fn cull_instance(index: u32) {
    let instance = instances[index];
    if instance.prop >= arrayLength(&props) {
        return;
    }
    let prop = props[instance.prop];
    if prop.parts == 0u {
        return;
    }
    let turn = vec2<f32>(cos(instance.yaw), sin(instance.yaw));
    let middle = vec3<f32>(prop.x, prop.y, prop.z) * instance.scale;
    let center = vec3<f32>(instance.x, instance.y, instance.z)
        + vec3<f32>(turn.x * middle.x + turn.y * middle.z, middle.y, -turn.y * middle.x + turn.x * middle.z);
    if !in_frustum(center, prop.radius * instance.scale) {
        return;
    }
    let pixels_per_meter = pixels_per_meter_at(distance(center, view.eye));
    var scale = instance.scale;
    let across = prop.extent * scale * pixels_per_meter;
    if across < view.grain_pixels {
        let grown = view.grain_pixels / max(across, 1e-6);
        if seed_share(instance.seed) >= 1.0 / (grown * grown) {
            return;
        }
        scale *= grown;
    }
    atomicAdd(&commands.drawn, 1u);
    let allowed = view.stray_pixels / (pixels_per_meter * scale);
    for (var part = prop.first_part; part < prop.first_part + prop.parts; part++) {
        let level = levels[coarsest_within(parts[part], allowed)];
        let slot = atomicAdd(&commands.commands[level.command].instance_count, 1u);
        let first = commands.commands[level.command].first_instance;
        drawn[first + slot] = vec2<u32>(index, bitcast<u32>(scale));
    }
}

// Whether a sphere about `center`, `radius` across, reaches into the view's frustum.
fn in_frustum(center: vec3<f32>, radius: f32) -> bool {
    for (var plane = 0u; plane < 6u; plane++) {
        if plane == 4u && view.test_near == 0u {
            continue;
        }
        if dot(view.planes[plane], vec4<f32>(center, 1.0)) + radius <= 0.0 {
            return false;
        }
    }
    return true;
}

fn pixels_per_meter_at(distance: f32) -> f32 {
    if view.orthographic != 0u {
        return view.pixels_at_a_meter;
    }
    return view.pixels_at_a_meter / max(distance, view.near);
}

// The coarsest of `part`'s levels straying by no more than `allowed`, else its finest.
fn coarsest_within(part: Part, allowed: f32) -> u32 {
    var chosen = part.first_level;
    for (var level = part.first_level + 1u; level < part.first_level + part.levels; level++) {
        if levels[level].strays <= allowed {
            chosen = level;
        }
    }
    return chosen;
}

// An instance's seed as a share between 0 and 1.
fn seed_share(seed: u32) -> f32 {
    return f32(seed >> 8u) / 16777216.0;
}
