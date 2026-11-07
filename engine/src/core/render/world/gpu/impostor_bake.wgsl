// A model's impostor atlas: the model's full mesh, part by part, an instance a cell, each drawn
// by the cell's orthographic view and placed in the cell, writing what the far field draws a prop
// by: its base colour, its normal folded onto an octahedron, and its depth across the model's
// bounding sphere.

override CELLS: u32;

struct BakePart {
    color: vec4<f32>,
    cutoff: f32,
    textured: u32,
}

@group(0) @binding(0) var<storage, read> clip_from_model: array<mat4x4<f32>>;
@group(0) @binding(1) var<uniform> part: BakePart;
@group(0) @binding(2) var base_texture: texture_2d<f32>;
@group(0) @binding(3) var base_sampler: sampler;

struct VertexIn {
    @builtin(instance_index) cell: u32,
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
}

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
}

// The cell's view frames the model's bounding sphere, so its clip space is the cell's square,
// placed in the layer's clip space by the cell's column and row, rows downward.
@vertex
fn vertex(in: VertexIn) -> VertexOut {
    let clip = clip_from_model[in.cell] * vec4<f32>(in.position, 1.0);
    let column = f32(in.cell % CELLS);
    let row = f32(in.cell / CELLS);
    let cells = f32(CELLS);
    var out: VertexOut;
    out.position = vec4<f32>(
        (clip.x + 1.0 + 2.0 * column) / cells - 1.0,
        (clip.y - 1.0 - 2.0 * row) / cells + 1.0,
        clip.z,
        1.0,
    );
    out.normal = in.normal;
    out.uv = in.uv;
    out.color = in.color;
    return out;
}

struct FragmentOut {
    @location(0) albedo: vec4<f32>,
    @location(1) normal: vec2<f32>,
    @location(2) depth: f32,
}

@fragment
fn fragment(in: VertexOut, @builtin(front_facing) front: bool) -> FragmentOut {
    var color = part.color * in.color;
    if part.textured != 0u {
        color *= textureSample(base_texture, base_sampler, in.uv);
    }
    if color.a < part.cutoff {
        discard;
    }
    let normal = normalize(select(-in.normal, in.normal, front));
    var out: FragmentOut;
    out.albedo = vec4<f32>(color.rgb, 1.0);
    out.normal = fold(normal);
    out.depth = in.position.z;
    return out;
}

// Where `direction` lies on the octahedron unfolded over the unit square.
fn fold(direction: vec3<f32>) -> vec2<f32> {
    let scaled = direction / (abs(direction.x) + abs(direction.y) + abs(direction.z));
    var folded = scaled.xz;
    if scaled.y < 0.0 {
        folded = (1.0 - abs(scaled.zx)) * select(vec2<f32>(-1.0), vec2<f32>(1.0), folded >= vec2<f32>(0.0));
    }
    return folded * 0.5 + 0.5;
}
