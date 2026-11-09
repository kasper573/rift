// Outlines laid over what a view shows: each pixel showing an outlined instance, by the ids its
// pick pass wrote, takes its layer's colour where a pixel within the outline's width shows anything
// else. The second layer is laid over the first.

struct Outlines {
    colors: array<vec4<f32>, 2>,
    // How many ids each layer holds: the first layer's first in `outlined`, the second's after them.
    counts: vec2<u32>,
    width: u32,
}

struct FullscreenVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@group(0) @binding(0) var ids: texture_2d<u32>;
// Each layer's ids, ascending.
@group(0) @binding(1) var<storage, read> outlined: array<u32>;
@group(0) @binding(2) var<uniform> outlines: Outlines;

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(in.position.xy);
    for (var layer = 1; layer >= 0; layer -= 1) {
        if edged(u32(layer), pixel) {
            return outlines.colors[layer];
        }
    }
    discard;
}

// Whether `pixel` shows an instance of `layer` and a pixel within the outline's width doesn't.
fn edged(layer: u32, pixel: vec2<i32>) -> bool {
    if !held(layer, id_at(pixel)) {
        return false;
    }
    let width = i32(outlines.width);
    for (var y = -width; y <= width; y += 1) {
        for (var x = -width; x <= width; x += 1) {
            if !held(layer, id_at(pixel + vec2<i32>(x, y))) {
                return true;
            }
        }
    }
    return false;
}

// The id the pixel at `pixel` shows, 0 off the picture.
fn id_at(pixel: vec2<i32>) -> u32 {
    let size = vec2<i32>(textureDimensions(ids));
    if any(pixel < vec2<i32>(0)) || any(pixel >= size) {
        return 0u;
    }
    return textureLoad(ids, pixel, 0).r;
}

// Whether `layer` holds `id`, by halving its ascending ids.
fn held(layer: u32, id: u32) -> bool {
    if id == 0u {
        return false;
    }
    var low = select(0u, outlines.counts.x, layer == 1u);
    var high = low + outlines.counts[layer];
    while low < high {
        let middle = (low + high) / 2u;
        let at = outlined[middle];
        if at == id {
            return true;
        }
        if at < id {
            low = middle + 1u;
        } else {
            high = middle;
        }
    }
    return false;
}
