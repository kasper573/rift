// A cover's sheet over the ground's patches: a point on every point of a patch, lying a hair over
// the ground, or on the water wherever it stands over the ground, weighted by how heavily the
// four paint pixels round it are painted with the sheet's terrain. Where a patch borders a coarser
// one, its points along that edge lie between the coarser one's, as the ground's do.
import engine::core::render::world::gpu::terrain::{
    ground, edge_ends, lie_at, metres_of, paint_of, patch_point, texels_in, world_at,
};

struct Sheet {
    color: vec4<f32>,
    terrain: u32,
    on_water: u32,
    // How far the shore's texels reach at their most, in metres.
    shore_reach: f32,
    // The fall a level of the surface's fall stands for, in metres a metre.
    fall_unit: f32,
}

// The surface a sheet on water lies on, held as the heights are: its level, its fall toward east
// and south as levels over `FALL_MIDDLE`, and how far it lies from the nearest dry ground, up to
// `shore_reach` at the texel's most.
@group(2) @binding(108) var surfaces: texture_2d_array<u32>;
@group(2) @binding(109) var overview_surface: texture_2d<u32>;
@group(2) @binding(110) var<uniform> drawn_sheet: Sheet;

// How far above the ground a sheet on it lies, so it draws over the ground without fighting it.
const SHEET_LIFT: f32 = 0.02;
// The fall level of a surface that doesn't fall.
const FALL_MIDDLE: f32 = 32768.0;
// The shore level of a surface at the shore's reach.
const SHORE_MOST: f32 = 65535.0;

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
    let surface = texels_in(surfaces, overview_surface, sample);
    let level = metres_of(surface.x);
    point.world = vec3<f32>(0.0, level, 0.0);
    point.depth = max(level - lie.w, 0.0);
    point.fall = (surface.yz - FALL_MIDDLE) * drawn_sheet.fall_unit;
    point.normal = normalize(vec3<f32>(point.fall.x, 1.0, point.fall.y));
    point.shore = surface.w / SHORE_MOST * drawn_sheet.shore_reach;
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
