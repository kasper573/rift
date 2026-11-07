// What a prop's shaders draw with when they bake a model's impostor atlas instead of a frame:
// each instance they draw is a cell of the atlas, looking at the model's bounding sphere from its
// own way orthographically, and the pixel they would light they write unlit instead, as the far
// field reads it back: its colour, its normal folded onto an octahedron, and its depth across the
// sphere.

// A cell of the atlas: the clip space of its view from the model's own metres, the way it looks
// from, the ways its picture's right and up run in the model, and where its square lies in the
// layer, as an offset and a scale of its clip space.
struct BakeCell {
    clip_from_model: mat4x4<f32>,
    seen: vec4<f32>,
    right: vec4<f32>,
    up: vec4<f32>,
    place: vec4<f32>,
}

@group(2) @binding(105) var<storage, read> bake_cells: array<BakeCell>;

struct BakeOutput {
    @location(0) albedo: vec4<f32>,
    @location(1) normal: vec2<f32>,
    @location(2) depth: f32,
}

// Where a point of the model at `local` lands in the layer's clip space, seen by the cell at
// `cell`.
fn bake_clip(cell: u32, local: vec3<f32>) -> vec4<f32> {
    let placed = bake_cells[cell];
    let clip = placed.clip_from_model * vec4<f32>(local, 1.0);
    return vec4<f32>(clip.xy * placed.place.z + placed.place.xy, clip.z, 1.0);
}

// The way the cell at `cell` looks from.
fn bake_seen(cell: u32) -> vec3<f32> {
    return bake_cells[cell].seen.xyz;
}

fn bake_right(cell: u32) -> vec3<f32> {
    return bake_cells[cell].right.xyz;
}

fn bake_up(cell: u32) -> vec3<f32> {
    return bake_cells[cell].up.xyz;
}

// What the atlas keeps of a pixel: `color` covering it whole, `normal`, and `depth` as the
// pixel's own.
fn baked(color: vec3<f32>, normal: vec3<f32>, depth: f32) -> BakeOutput {
    var out: BakeOutput;
    out.albedo = vec4<f32>(color, 1.0);
    out.normal = fold_oct(normalize(normal));
    out.depth = depth;
    return out;
}

// Where `direction` lies on the octahedron unfolded over the unit square.
fn fold_oct(direction: vec3<f32>) -> vec2<f32> {
    let scaled = direction / (abs(direction.x) + abs(direction.y) + abs(direction.z));
    var folded = scaled.xz;
    if scaled.y < 0.0 {
        folded = (1.0 - abs(scaled.zx)) * select(vec2<f32>(-1.0), vec2<f32>(1.0), folded >= vec2<f32>(0.0));
    }
    return folded * 0.5 + 0.5;
}
