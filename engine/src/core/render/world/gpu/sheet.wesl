// A cover's sheet over the ground's patches: a point on every point of a patch, lying a hair over
// the ground, or on the water wherever it stands over the ground, weighted by how heavily the
// four paint pixels round it are painted with the sheet's terrain. Where a patch borders a coarser
// one, its points along that edge lie between the coarser one's, as the ground's do.
import engine::core::render::world::gpu::terrain::{
    ground, edge_ends, height_of, level_in, lie_at, metres_of, paint_of, patch_point, world_at,
};

struct Sheet {
    color: vec4<f32>,
    terrain: u32,
    on_water: u32,
    // How far the shore's texels reach at their most, in metres.
    shore_reach: f32,
}

// The water's surface and its shore, held as the heights are.
@group(2) @binding(108) var waters: texture_2d_array<u32>;
@group(2) @binding(109) var overview_water: texture_2d<u32>;
@group(2) @binding(110) var shores: texture_2d_array<u32>;
@group(2) @binding(111) var overview_shore: texture_2d<u32>;
@group(2) @binding(112) var<uniform> drawn_sheet: Sheet;

// How far above the ground a sheet on it lies, so it draws over the ground without fighting it.
const SHEET_LIFT: f32 = 0.02;
// How far under the ground a sheet on water lies away from any water.
const DRY_SINK: f32 = 1.0;
// How far either way along the water a sheet reads its surface's fall, in samples: far enough
// that the gentlest river's fall shows through the steps its heights are kept in.
const FALL_SAMPLES: i32 = 4;

// A point of a sheet: where it lies, its normal, its uv over the whole area, how much water stands
// over the ground under it, how heavily the sheet's terrain is painted there, how far it lies from
// the nearest dry ground and how the water's surface falls there, in metres a metre toward east
// and south. Off water, its depth, shore and fall are nothing.
struct SheetPoint {
    world: vec3<f32>,
    normal: vec3<f32>,
    uv: vec2<f32>,
    depth: f32,
    weight: f32,
    shore: f32,
    fall: vec2<f32>,
}

// The point of the sheet on the point `grid` of the patch the view shows as its `instance`th.
fn sheet_point(grid: vec3<f32>, instance: u32) -> SheetPoint {
    let at = patch_point(grid, instance);
    var point: SheetPoint;
    if at.across > at.stride {
        let ends = edge_ends(at);
        point = between(sheet_at(ends.before, at.across), sheet_at(ends.after, at.across), ends.t);
    } else {
        point = sheet_at(at.sample, at.stride);
    }
    point.world = world_at(at.sample, point.world.y);
    let last = max(ground.samples - 1u, vec2<u32>(1u));
    point.uv = vec2<f32>(min(at.sample, ground.samples - 1u)) / vec2<f32>(last);
    return point;
}

// The point of the sheet over the sample `sample` of a patch laid every `stride` samples: its
// height alone in `world`.
fn sheet_at(sample: vec2<u32>, stride: u32) -> SheetPoint {
    var point: SheetPoint;
    point.weight = painted(sample);
    let lie = lie_at(sample, stride);
    if drawn_sheet.on_water == 0u {
        point.world = vec3<f32>(0.0, lie.w + SHEET_LIFT, 0.0);
        point.normal = normalize(lie.xyz);
        return point;
    }
    let level = surface_level(sample);
    point.world = vec3<f32>(0.0, level, 0.0);
    point.depth = max(level - lie.w, 0.0);
    point.fall = fall_at(sample);
    point.normal = normalize(vec3<f32>(point.fall.x, 1.0, point.fall.y));
    point.shore = level_in(shores, overview_shore, sample) / 255.0 * drawn_sheet.shore_reach;
    return point;
}

// The point `t` of the way from `a` to `b`.
fn between(a: SheetPoint, b: SheetPoint, t: f32) -> SheetPoint {
    var point: SheetPoint;
    point.world = mix(a.world, b.world, t);
    point.normal = normalize(mix(a.normal, b.normal, t));
    point.depth = mix(a.depth, b.depth, t);
    point.weight = mix(a.weight, b.weight, t);
    point.shore = mix(a.shore, b.shore, t);
    point.fall = mix(a.fall, b.fall, t);
    return point;
}

// The share of the four paint pixels round the sample `sample` that the sheet's terrain paints.
fn painted(sample: vec2<u32>) -> f32 {
    let pixels = vec2<i32>(ground.samples) - 1;
    var total = 0.0;
    var count = 0.0;
    for (var corner = 0; corner < 4; corner++) {
        let pixel = vec2<i32>(sample) + vec2<i32>(corner % 2, corner / 2) - 1;
        if all(pixel >= vec2<i32>(0)) && all(pixel < pixels) {
            count += 1.0;
            total += select(0.0, 1.0, paint_of(vec2<u32>(pixel)) == drawn_sheet.terrain);
        }
    }
    return select(0.0, total / count, count > 0.0);
}

// The height of the water's surface at `sample`, in metres.
fn water_of(sample: vec2<u32>) -> f32 {
    return metres_of(level_in(waters, overview_water, sample));
}

// Whether water stands over the ground at `sample`.
fn wet(sample: vec2<u32>) -> bool {
    return water_of(sample) > height_of(sample);
}

// Whether `sample` lies on the ground.
fn inside(sample: vec2<i32>) -> bool {
    return all(sample >= vec2<i32>(0)) && all(sample < vec2<i32>(ground.samples));
}

// The level the sheet stands at over `sample`: the water's surface where water lies, and beside it
// the highest surface next to it, so the sheet runs on flat into the bank and meets it where the
// water does; under the ground away from any water.
fn surface_level(sample: vec2<u32>) -> f32 {
    if wet(sample) {
        return water_of(sample);
    }
    var beside = 0.0;
    var any = false;
    for (var near = 0; near < 9; near++) {
        let at = vec2<i32>(sample) + vec2<i32>(near % 3, near / 3) - 1;
        if inside(at) && wet(vec2<u32>(at)) {
            let level = water_of(vec2<u32>(at));
            beside = select(level, max(beside, level), any);
            any = true;
        }
    }
    return select(height_of(sample) - DRY_SINK, beside, any);
}

// How the water's surface falls at `sample`, in metres a metre toward east and south: read across
// `FALL_SAMPLES` either way, as far as the water reaches, so dry ground beside it doesn't count.
fn fall_at(sample: vec2<u32>) -> vec2<f32> {
    let here = water_of(sample);
    return vec2<f32>(
        fall_along(sample, vec2<i32>(1, 0), here),
        fall_along(sample, vec2<i32>(0, 1), here),
    );
}

fn fall_along(sample: vec2<u32>, way: vec2<i32>, here: f32) -> f32 {
    let ahead = reach(sample, way, here);
    let behind = reach(sample, -way, here);
    let span = (ahead.y + behind.y) / f32(ground.samples_per_metre);
    return select(0.0, (behind.x - ahead.x) / span, span > 0.0);
}

// The surface of the farthest water within `FALL_SAMPLES` of `sample` along `way`, and how many
// samples off it lies: `here` and none where no water lies along it.
fn reach(sample: vec2<u32>, way: vec2<i32>, here: f32) -> vec2<f32> {
    for (var samples = FALL_SAMPLES; samples >= 1; samples--) {
        let at = vec2<i32>(sample) + way * samples;
        if inside(at) && wet(vec2<u32>(at)) {
            return vec2<f32>(water_of(vec2<u32>(at)), f32(samples));
        }
    }
    return vec2<f32>(here, 0.0);
}
