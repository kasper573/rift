// Which of a scenery's prop instances a view draws, and at which level of each part, in three
// passes: one workgroup for each chunk the view may see, its threads striding through the chunk's
// instances. `count` counts each instance the view sees in the command that draws its part at
// the level its size on screen asks for, as many times as its model has parts. `place` lays the
// commands' regions of the drawn instances out one after another, each as large as its count,
// and starts the counts over. `list` then lists each seen instance in its commands' regions. The
// view that clears what hides the camera's subject moves each fading instance it sees a step
// toward cleared while the way from its eye to the subject passes through the instance's box,
// and back toward whole once it doesn't, and lists it with how far it shows and whether the eye
// stands inside it.

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
    // Whom the view's camera follows, and how far a fade moves this frame: none for a view that
    // draws every prop whole.
    subject: vec3<f32>,
    fade_step: f32,
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
    // Whether it clears away where it hides the camera's subject, tested by the box `half` each
    // way of the middle.
    fades: u32,
    half_x: f32,
    half_y: f32,
    half_z: f32,
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
// Each drawn instance by its place among `instances`, the scale it is drawn at, how far it shows
// and whether the eye stands inside it.
@group(0) @binding(7) var<storage, read_write> drawn: array<vec4<u32>>;
// How far each instance shows, by its place among `instances`: 1 whole, 0 cleared away.
@group(0) @binding(8) var<storage, read_write> shown: array<f32>;

const THREADS: u32 = 64u;
const PLACERS: u32 = 256u;

@compute @workgroup_size(64)
fn count(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_index) thread: u32) {
    let chunk = chunks[group.x];
    for (var at = thread; at < chunk.y; at += THREADS) {
        count_instance(chunk.x + at);
    }
}

var<workgroup> stripes: array<u32, 256>;

// Each thread sums a stripe of the commands' counts, the stripes are summed up in turn, and each
// thread then lays its stripe's regions out from where the stripes before it end.
@compute @workgroup_size(256)
fn place(@builtin(local_invocation_index) thread: u32) {
    let all = arrayLength(&commands.commands);
    let stripe = (all + PLACERS - 1u) / PLACERS;
    let low = min(thread * stripe, all);
    let high = min(low + stripe, all);
    var counted = 0u;
    for (var command = low; command < high; command++) {
        counted += atomicLoad(&commands.commands[command].instance_count);
    }
    stripes[thread] = counted;
    workgroupBarrier();
    if thread == 0u {
        var before = 0u;
        for (var at = 0u; at < PLACERS; at++) {
            let here = stripes[at];
            stripes[at] = before;
            before += here;
        }
    }
    workgroupBarrier();
    var first = stripes[thread];
    for (var command = low; command < high; command++) {
        commands.commands[command].first_instance = first;
        first += atomicExchange(&commands.commands[command].instance_count, 0u);
    }
}

@compute @workgroup_size(64)
fn list(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_index) thread: u32) {
    let chunk = chunks[group.x];
    for (var at = thread; at < chunk.y; at += THREADS) {
        list_instance(chunk.x + at);
    }
}

// An instance as the view sees it: whether it does at all, the scale it is drawn at, and how far
// a level may stray from the full mesh in the model's own units.
struct Sighted {
    seen: bool,
    scale: f32,
    allowed: f32,
    prop: Prop,
    turn: vec2<f32>,
}

fn count_instance(index: u32) {
    let instance = instances[index];
    let sighted = sight(instance);
    if !sighted.seen {
        return;
    }
    atomicAdd(&commands.drawn, 1u);
    let prop = sighted.prop;
    for (var part = prop.first_part; part < prop.first_part + prop.parts; part++) {
        let level = levels[coarsest_within(parts[part], sighted.allowed)];
        atomicAdd(&commands.commands[level.command].instance_count, 1u);
    }
}

fn list_instance(index: u32) {
    let instance = instances[index];
    let sighted = sight(instance);
    if !sighted.seen {
        return;
    }
    let prop = sighted.prop;
    let fade = faded(index, instance, prop, sighted.turn);
    let listed = vec4<u32>(index, bitcast<u32>(sighted.scale), bitcast<u32>(fade.x), u32(fade.y));
    for (var part = prop.first_part; part < prop.first_part + prop.parts; part++) {
        let level = levels[coarsest_within(parts[part], sighted.allowed)];
        let slot = atomicAdd(&commands.commands[level.command].instance_count, 1u);
        drawn[commands.commands[level.command].first_instance + slot] = listed;
    }
}

fn sight(instance: Instance) -> Sighted {
    var sighted: Sighted;
    sighted.seen = false;
    if instance.prop >= arrayLength(&props) {
        return sighted;
    }
    let prop = props[instance.prop];
    if prop.parts == 0u {
        return sighted;
    }
    let turn = vec2<f32>(cos(instance.yaw), sin(instance.yaw));
    let middle = vec3<f32>(prop.x, prop.y, prop.z) * instance.scale;
    let center = vec3<f32>(instance.x, instance.y, instance.z)
        + vec3<f32>(turn.x * middle.x + turn.y * middle.z, middle.y, -turn.y * middle.x + turn.x * middle.z);
    if !in_frustum(center, prop.radius * instance.scale) {
        return sighted;
    }
    let pixels_per_meter = pixels_per_meter_at(distance(center, view.eye));
    var scale = instance.scale;
    let across = prop.extent * scale * pixels_per_meter;
    if across < view.grain_pixels {
        let grown = view.grain_pixels / max(across, 1e-6);
        if seed_share(instance.seed) >= 1.0 / (grown * grown) {
            return sighted;
        }
        scale *= grown;
    }
    sighted.seen = true;
    sighted.scale = scale;
    sighted.allowed = view.stray_pixels / (pixels_per_meter * scale);
    sighted.prop = prop;
    sighted.turn = turn;
    return sighted;
}

// How far the instance at `index` shows this frame, moved a step toward cleared while the way
// from the eye to the subject passes through its box and toward whole once it doesn't, and
// whether the eye stands inside the box: whole, outside, for a view that clears nothing.
fn faded(index: u32, instance: Instance, prop: Prop, turn: vec2<f32>) -> vec2<f32> {
    if view.fade_step <= 0.0 || prop.fades == 0u {
        return vec2<f32>(1.0, 0.0);
    }
    let low = vec3<f32>(prop.x - prop.half_x, prop.y - prop.half_y, prop.z - prop.half_z);
    let high = vec3<f32>(prop.x + prop.half_x, prop.y + prop.half_y, prop.z + prop.half_z);
    let at = vec3<f32>(instance.x, instance.y, instance.z);
    let eye = local_of(view.eye, at, turn, instance.scale);
    let subject = local_of(view.subject, at, turn, instance.scale);
    let inside = all(eye >= low) && all(eye <= high);
    let wanted = select(1.0, 0.0, inside || crosses(eye, subject, low, high));
    let was = shown[index];
    let now = was + clamp(wanted - was, -view.fade_step, view.fade_step);
    shown[index] = now;
    return vec2<f32>(now, select(0.0, 1.0, inside));
}

// Where `world` lies in the model's own metres, for an instance standing at `at`, turned by
// `turn` and grown `scale` times.
fn local_of(world: vec3<f32>, at: vec3<f32>, turn: vec2<f32>, scale: f32) -> vec3<f32> {
    let way = world - at;
    return vec3<f32>(turn.x * way.x - turn.y * way.z, way.y, turn.y * way.x + turn.x * way.z) / scale;
}

// Whether the segment from `a` to `b` passes through the box from `low` to `high`.
fn crosses(a: vec3<f32>, b: vec3<f32>, low: vec3<f32>, high: vec3<f32>) -> bool {
    let way = b - a;
    var enter = 0.0;
    var exit = 1.0;
    for (var axis = 0u; axis < 3u; axis++) {
        if abs(way[axis]) < 1e-6 {
            if a[axis] < low[axis] || a[axis] > high[axis] {
                return false;
            }
            continue;
        }
        let t1 = (low[axis] - a[axis]) / way[axis];
        let t2 = (high[axis] - a[axis]) / way[axis];
        enter = max(enter, min(t1, t2));
        exit = min(exit, max(t1, t2));
        if enter > exit {
            return false;
        }
    }
    return true;
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
