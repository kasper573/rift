// The ground as the GPU draws it: patches of one shared grid, each laid over the ground's height
// samples from its first, a sample every `stride` along each way, its heights read from the atlas:
// from the slot of the tile they lie in where a slot holds it, else between the overview's values.
// Where a patch borders a coarser one, its points along that edge lie on the coarser one's edge, so
// the two meet without a crack.

struct TerrainPatch {
    first: vec2<u32>,
    stride: u32,
    // The stride of the patch across each edge, as a power of two: west in the lowest byte, then
    // north, east and south.
    edges: u32,
}

// The ground the atlas holds, as the patches are laid over it.
struct TerrainGround {
    // Where the ground's first sample lies, in metres.
    origin: vec2<f32>,
    // Height samples across and down.
    samples: vec2<u32>,
    // Tiles across and down, and the pixels along a tile's side.
    tiles: vec2<u32>,
    tile_pixels: u32,
    samples_per_metre: u32,
    // The overviews' texels across and down, and the ground's samples to a texel of the
    // heights' overview.
    overview: vec2<u32>,
    paint_overview: vec2<u32>,
    overview_span: u32,
    // The cells along a patch's side.
    grid_cells: u32,
    // The metres a level of the heights stands for, and the level ground level lies on.
    level_step: f32,
    ground_level: f32,
    // The ground a canopy texel spans each way, in metres, the widest a canopy's props are told
    // to be, and how many pixels across a prop must span for the view to draw it rather than thin
    // it away.
    canopy_texel: f32,
    canopy_extent_max: f32,
    grain_pixels: f32,
}

@group(2) @binding(100) var<uniform> ground: TerrainGround;
@group(2) @binding(101) var<storage, read> patches: array<TerrainPatch>;
// The patches each view shows, by their place among `patches`, the view's from its first instance.
@group(2) @binding(102) var<storage, read> shown: array<u32>;
@group(2) @binding(103) var heights: texture_2d_array<u32>;
@group(2) @binding(104) var overview_heights: texture_2d<u32>;
// The slot each tile is held in, one past it, or none.
@group(2) @binding(105) var table: texture_2d<u32>;

// What the ground is painted with: the paint of the tiles held in detail, a layer a slot, and the
// overview's.
@group(2) @binding(106) var paints: texture_2d_array<u32>;
@group(2) @binding(107) var overview_paint: texture_2d<u32>;

// Where a point of the shared grid lies on the patch a view shows: its sample, the patch's
// stride, and, where it lies on an edge the patch shares with a coarser one, the way along that
// edge and the coarser one's stride, which is the patch's own elsewhere.
struct PatchPoint {
    sample: vec2<u32>,
    stride: u32,
    way: vec2<u32>,
    across: u32,
}

// A point of a patch on the ground: where it lies in the world, its normal, and the stride of the
// patch, which the paint under it is read at.
struct GroundPoint {
    world: vec3<f32>,
    normal: vec3<f32>,
    stride: u32,
}

// Where the point `grid` of the shared grid, from 0 to 1 across each way, lies on the patch the
// view shows as its `instance`th.
fn patch_point(grid: vec3<f32>, instance: u32) -> PatchPoint {
    let laid = patches[shown[instance]];
    let point = vec2<u32>(round(grid.xz * f32(ground.grid_cells)));
    let sample = laid.first + point * laid.stride;
    var west_east = laid.stride;
    if point.x == 0u {
        west_east = edge_stride(laid.edges, 0u);
    } else if point.x == ground.grid_cells {
        west_east = edge_stride(laid.edges, 2u);
    }
    var north_south = laid.stride;
    if point.y == 0u {
        north_south = edge_stride(laid.edges, 1u);
    } else if point.y == ground.grid_cells {
        north_south = edge_stride(laid.edges, 3u);
    }
    if west_east > laid.stride {
        return PatchPoint(sample, laid.stride, vec2<u32>(0u, 1u), west_east);
    }
    if north_south > laid.stride {
        return PatchPoint(sample, laid.stride, vec2<u32>(1u, 0u), north_south);
    }
    return PatchPoint(sample, laid.stride, vec2<u32>(0u), laid.stride);
}

// The point on the ground of the point `grid` of the patch the view shows as its `instance`th.
fn ground_point(grid: vec3<f32>, instance: u32) -> GroundPoint {
    let at = patch_point(grid, instance);
    var lie: vec4<f32>;
    if at.across > at.stride {
        let ends = edge_ends(at);
        let a = lie_at(ends.before, at.across);
        let b = lie_at(ends.after, at.across);
        lie = vec4<f32>(
            normalize(mix(normalize(a.xyz), normalize(b.xyz), ends.t)),
            mix(a.w, b.w, ends.t),
        );
    } else {
        lie = lie_at(at.sample, at.stride);
    }
    return GroundPoint(world_at(at.sample, lie.w), normalize(lie.xyz), at.stride);
}

// The samples either side of a point on an edge it shares with a coarser patch, which that patch
// has points on, and how far along from the first to the second it lies.
struct EdgeEnds {
    before: vec2<u32>,
    after: vec2<u32>,
    t: f32,
}

fn edge_ends(at: PatchPoint) -> EdgeEnds {
    let offset = dot(at.sample, at.way) % at.across;
    let before = at.sample - at.way * offset;
    return EdgeEnds(before, before + at.way * at.across, f32(offset) / f32(at.across));
}

// Where the sample `sample` lies in the world, at `height`.
fn world_at(sample: vec2<u32>, height: f32) -> vec3<f32> {
    let at = ground.origin + vec2<f32>(min(sample, ground.samples - 1u)) / f32(ground.samples_per_metre);
    return vec3<f32>(at.x, height, at.y);
}

// The stride of the patch across edge `edge` of a patch whose edges are `edges`.
fn edge_stride(edges: u32, edge: u32) -> u32 {
    return 1u << ((edges >> (edge * 8u)) & 0xffu);
}

// The ground's normal at `sample`, its neighbours `stride` samples off each way, in xyz, and its
// height in w.
fn lie_at(sample: vec2<u32>, stride: u32) -> vec4<f32> {
    let last = ground.samples - 1u;
    let at = min(sample, last);
    let east = height_of(min(at + vec2<u32>(stride, 0u), last))
        - height_of(at - vec2<u32>(min(at.x, stride), 0u));
    let south = height_of(min(at + vec2<u32>(0u, stride), last))
        - height_of(at - vec2<u32>(0u, min(at.y, stride)));
    let step = f32(stride) / f32(ground.samples_per_metre);
    return vec4<f32>(-east, 2.0 * step, -south, height_of(at));
}

// The ground's height at `sample`, in metres.
fn height_of(sample: vec2<u32>) -> f32 {
    return metres_of(level_in(heights, overview_heights, sample));
}

// The height a level of the heights stands for, in metres.
fn metres_of(level: f32) -> f32 {
    return (level - ground.ground_level) * ground.level_step;
}

// The value at the sample `sample` of the ground `details` holds a slot a tile of, and `overview`
// holds the overview of: from the slot of the tile it lies in where one holds it, else read
// between the overview's.
fn level_in(details: texture_2d_array<u32>, overview: texture_2d<u32>, sample: vec2<u32>) -> f32 {
    let tile = min(sample / ground.tile_pixels, ground.tiles - 1u);
    let slot = i32(textureLoad(table, tile, 0).r) - 1;
    if slot >= 0 {
        let local = sample - tile * ground.tile_pixels + 1u;
        return f32(textureLoad(details, local, slot, 0).r);
    }
    let span = ground.overview_span;
    let last = ground.samples - 1u;
    let low = min(sample / span, max(ground.overview, vec2<u32>(2u)) - 2u);
    let high = min(low + 1u, ground.overview - 1u);
    let start = low * span;
    let end = min(high * span, last);
    let t = select(
        vec2<f32>(0.0),
        vec2<f32>(sample - start) / vec2<f32>(max(end - start, vec2<u32>(1u))),
        end > start,
    );
    let a = f32(textureLoad(overview, vec2<u32>(low.x, low.y), 0).r);
    let b = f32(textureLoad(overview, vec2<u32>(high.x, low.y), 0).r);
    let c = f32(textureLoad(overview, vec2<u32>(low.x, high.y), 0).r);
    let d = f32(textureLoad(overview, vec2<u32>(high.x, high.y), 0).r);
    return mix(mix(a, b, t.x), mix(c, d, t.x), t.y);
}

// The terrain painting the pixel `pixel`: from the slot of the tile it lies in where one holds it,
// else the overview's nearest.
fn paint_of(pixel: vec2<u32>) -> u32 {
    let tile = min(pixel / ground.tile_pixels, ground.tiles - 1u);
    let slot = i32(textureLoad(table, tile, 0).r) - 1;
    if slot >= 0 {
        let local = pixel - tile * ground.tile_pixels + 1u;
        return textureLoad(paints, local, slot, 0).r;
    }
    let texel = pixel / ground.overview_span;
    return textureLoad(overview_paint, min(texel, ground.paint_overview - 1u), 0).r;
}
