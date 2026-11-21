// A sheet over the ground's patches: a point on every point of a patch, lying a hair over the
// ground wherever the sheet's terrain paints it, or on the water wherever the sheet's water lies,
// weighted by how many of the four pixels round it name the sheet's terrain, or by how far the
// sheet's water leads the other waters among them. Where a patch borders a coarser one, its
// points along that edge lie between the coarser one's, as the ground's do.
import engine::core::render::world::gpu::terrain::{
    ground, edge_ends, lie_at, metres_of, overview_surface, paint_of, patch_point, shore_of,
    surfaces, texels_in, water_of, world_at,
};

struct Sheet {
    color: vec4<f32>,
    // Which of the area's terrains the sheet lies over, or which of its waters on water.
    index: u32,
    on_water: u32,
    // How many samples off a point its openness is sought.
    open_samples: u32,
    // The steepest fall the surface tells, in metres a metre.
    fall_most: f32,
    // For a sheet on water: whether a picture of the sky is bound for it to mirror, whether its
    // mirror looks for the land as well as the sky, and how much it bends what lies under it,
    // 0 for a player who has stilled that.
    under_sky: u32,
    mirrors_land: u32,
    bends: f32,
}

@group(2) @binding(110) var<uniform> drawn_sheet: Sheet;
// For each of the area's terrains its colour in the first row and its silt, in g, in the
// second; for each of its waters how fast it fades each colour and its murk in the first row,
// and the colour of its depths in the second.
@group(2) @binding(111) var terrain_looks: texture_2d<f32>;
@group(2) @binding(112) var water_looks: texture_2d<f32>;
// The sky's picture, for a sheet on water to mirror, at every size.
@group(2) @binding(113) var sky_picture: texture_2d<f32>;
@group(2) @binding(114) var sky_picture_sampler: sampler;

const TAU: f32 = 6.2831855;
// How far above the ground a sheet on it lies, so it draws over the ground without fighting it.
const SHEET_LIFT: f32 = 0.02;
// The fall level of a surface that doesn't fall, the middle of what a 16 bit texel holds, and
// how many levels lie to each side of it: a level holds the fall's share of the steepest by its
// square root.
const FALL_MIDDLE: f32 = 32768.0;
const FALL_LEVELS: f32 = 32767.0;
// By how much of the pixels round a point the water named first leads one that ties with it.
const TIE_LEAD: f32 = 0.01;
// How far behind the waterline dry ground lies before no waterline runs near it, as a share of
// how far back the shore's texels reach.
const ASHORE: f32 = 0.95;

// A point of a sheet: where it lies, its normal, its uv over the whole area, how much water stands
// over the ground under it, less than nothing where the ground stands over the water's level,
// how heavily the sheet's terrain is painted there or how far the sheet's water leads the other
// waters round it, more than nothing where the point is the sheet's and never nothing where
// waters meet, how far it lies out from the waterline, less than nothing on dry ground behind it,
// how far from the waterline the water round it lies at its most, and how the water's surface
// falls there, in metres a metre toward east and south; and on water what the bed under
// it gives the water (its colour, and its silt in a) and what the waters round it are like, the
// waters meeting there blended: how fast they fade each colour with their murk in a, and the
// colour of their depths. Off water, all of those are nothing.
struct SheetPoint {
    world: vec3<f32>,
    normal: vec3<f32>,
    uv: vec2<f32>,
    depth: f32,
    weight: f32,
    shore: f32,
    open: f32,
    fall: vec2<f32>,
    bed: vec4<f32>,
    fading: vec4<f32>,
    depths: vec3<f32>,
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
    let lie = lie_at(sample, stride);
    if drawn_sheet.on_water == 0u {
        point.weight = painted(sample);
        point.world = vec3<f32>(0.0, lie.w + SHEET_LIFT, 0.0);
        point.normal = normalize(lie.xyz);
        return point;
    }
    point = lying_round(sample);
    let surface = texels_in(surfaces, overview_surface, sample);
    let level = metres_of(surface.x);
    point.world = vec3<f32>(0.0, level, 0.0);
    point.depth = level - lie.w;
    let kept = (surface.yz - FALL_MIDDLE) / FALL_LEVELS;
    point.fall = sign(kept) * kept * kept * drawn_sheet.fall_most;
    point.normal = normalize(vec3<f32>(point.fall.x, 1.0, point.fall.y));
    point.shore = shore_of(surface);
    point.open = point.shore;
    // Only a point near a waterline has water round it that may lie no farther out than it.
    if point.shore > -ground.shore_back * ASHORE && point.shore < f32(drawn_sheet.open_samples) / f32(ground.samples_per_metre) {
        let last = vec2<i32>(ground.samples) - 1;
        for (var way = 0; way < 8; way++) {
            let turn = f32(way) * TAU / 8.0;
            let off = vec2<i32>(round(vec2<f32>(cos(turn), sin(turn)) * f32(drawn_sheet.open_samples)));
            let there = vec2<u32>(clamp(vec2<i32>(sample) + off, vec2<i32>(0), last));
            point.open = max(point.open, shore_of(texels_in(surfaces, overview_surface, there)));
        }
    }
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
    point.open = mix(a.open, b.open, t);
    point.fall = mix(a.fall, b.fall, t);
    point.bed = mix(a.bed, b.bed, t);
    point.fading = mix(a.fading, b.fading, t);
    point.depths = mix(a.depths, b.depths, t);
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
            total += select(0.0, 1.0, paint_of(vec2<u32>(pixel)) == drawn_sheet.index);
        }
    }
    return select(0.0, total / count, count > 0.0);
}

// What lies round the sample `sample` of a sheet on water, of the four pixels round it: how far
// the sheet's water leads the others among them, what their terrains give the water and what
// their waters are like. Two waters that share the pixels evenly tie, and the one named first
// in the area leads by a hair, so every point is one water's alone.
fn lying_round(sample: vec2<u32>) -> SheetPoint {
    let pixels = vec2<i32>(ground.samples) - 1;
    var point: SheetPoint;
    var named: array<u32, 4>;
    var count = 0u;
    for (var corner = 0; corner < 4; corner++) {
        let pixel = vec2<i32>(sample) + vec2<i32>(corner % 2, corner / 2) - 1;
        if all(pixel >= vec2<i32>(0)) && all(pixel < pixels) {
            let water = i32(min(water_of(vec2<u32>(pixel)), textureDimensions(water_looks).x - 1u));
            let terrain = i32(min(paint_of(vec2<u32>(pixel)), textureDimensions(terrain_looks).x - 1u));
            named[count] = u32(water);
            count += 1u;
            point.bed += vec4<f32>(
                textureLoad(terrain_looks, vec2<i32>(terrain, 0), 0).rgb,
                textureLoad(terrain_looks, vec2<i32>(terrain, 1), 0).g,
            );
            point.fading += textureLoad(water_looks, vec2<i32>(water, 0), 0);
            point.depths += textureLoad(water_looks, vec2<i32>(water, 1), 0).rgb;
        }
    }
    var own = 0.0;
    var rival = 0.0;
    var rival_water = drawn_sheet.index;
    for (var one = 0u; one < count; one++) {
        var alike = 0.0;
        for (var other = 0u; other < count; other++) {
            alike += select(0.0, 1.0, named[other] == named[one]);
        }
        if named[one] == drawn_sheet.index {
            own = alike;
        } else if alike > rival || (alike == rival && named[one] < rival_water) {
            rival = alike;
            rival_water = named[one];
        }
    }
    let share = 1.0 / max(f32(count), 1.0);
    point.weight = (own - rival) * share;
    if rival > 0.0 {
        point.weight += select(-TIE_LEAD, TIE_LEAD, drawn_sheet.index < rival_water);
    }
    point.bed *= share;
    point.fading *= share;
    point.depths *= share;
    return point;
}
