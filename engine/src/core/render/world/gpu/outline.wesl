// Outlines laid over a view's finished picture: round the shapes each set of outlined instances
// marked, a line as wide as the outline is, outside them. The marks tell the share of each pixel
// a shape covers, so the line lies where the shape's edge does within the pixel, and moves with
// it by less than a pixel at a time. A gap in a set's shapes too narrow to hold the line's width
// across is none to it: the pinholes between a crown's leaves, which come and go as the leaves
// sway, draw no line, and a gap growing as wide fades its line in. The second set's line is laid
// over the first's.

struct Outlines {
    colors: array<vec4<f32>, 2>,
    width: f32,
}

struct FullscreenVertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// Each set's shapes, a channel a set.
@group(0) @binding(0) var marks: texture_2d<f32>;
@group(0) @binding(1) var between: sampler;
@group(0) @binding(2) var<uniform> outlines: Outlines;

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let near = marked_near(in.position.xy);
    var color = vec3<f32>(0.0);
    var cover = 0.0;
    for (var which = 0u; which < 2u; which += 1u) {
        if near[which] == 0.0 {
            continue;
        }
        let line = lined(which, vec2<i32>(in.position.xy)) * outlines.colors[which].a;
        color = mix(color * cover, outlines.colors[which].rgb, line);
        cover = line + cover * (1.0 - line);
        color /= max(cover, 0.0001);
    }
    if cover == 0.0 {
        discard;
    }
    return vec4<f32>(color, cover);
}

// How far pixels a line may lie from, each way: its width and the pixel its edge eases over.
fn reach() -> i32 {
    return i32(ceil(outlines.width)) + 1;
}

// How much each set marks within the line's reach of the pixel at `at`, none where no line can
// lie: read between the marks' pixels, four at a read, so the many pixels far from every shape
// cost a few reads each.
fn marked_near(at: vec2<f32>) -> vec2<f32> {
    let size = vec2<f32>(textureDimensions(marks));
    let pairs = (reach() + 2) / 2;
    var found = vec2<f32>(0.0);
    for (var y = -pairs; y < pairs; y += 1) {
        for (var x = -pairs; x < pairs; x += 1) {
            let corner = floor(at) + vec2<f32>(f32(2 * x + 1), f32(2 * y + 1));
            found += textureSampleLevel(marks, between, corner / size, 0.0).rg;
        }
    }
    return found;
}

// How much of the pixel at `pixel` the line round the shapes of the set `which` covers: the share
// of it outside the shapes and within the outline's width of them, none of it in a narrow gap.
fn lined(which: u32, pixel: vec2<i32>) -> f32 {
    let off = off(which, pixel);
    let cover = clamp(outlines.width + 0.5 - off, 0.0, 1.0) * clamp(off + 0.5, 0.0, 1.0);
    if cover == 0.0 {
        return 0.0;
    }
    return cover * open(which, pixel);
}

// How far the middle of the pixel at `pixel` lies outside the shapes of the set `which`, in
// pixels, under 0 within one, and no further than the line reaches: the nearest edge lies as far
// into the nearest pixel a shape covers as the share of that pixel it covers.
fn off(which: u32, pixel: vec2<i32>) -> f32 {
    let reach = reach();
    var nearest = f32(reach);
    for (var y = -reach; y <= reach; y += 1) {
        for (var x = -reach; x <= reach; x += 1) {
            let covered = covered(which, pixel + vec2<i32>(x, y));
            if covered > 0.0 {
                nearest = min(nearest, length(vec2<f32>(f32(x), f32(y))) + 0.5 - covered);
            }
        }
    }
    return nearest;
}

// How far in the open the pixel at `pixel` lies, from 0 to 1: whether a pixel within the line's
// width of its near edge lies that far outside every shape of the set `which`, so the line fits
// across the gap it lies in, easing in over the last half pixel.
fn open(which: u32, pixel: vec2<i32>) -> f32 {
    let within = outlines.width + 0.5;
    let radius = i32(ceil(within));
    var clearance = 0.0;
    for (var y = -radius; y <= radius; y += 1) {
        for (var x = -radius; x <= radius; x += 1) {
            if f32(x * x + y * y) <= within * within {
                clearance = max(clearance, off(which, pixel + vec2<i32>(x, y)));
            }
        }
    }
    return clamp((clearance - outlines.width) * 2.0 + 1.0, 0.0, 1.0);
}

// The share of the pixel at `pixel` the shapes of the set `which` cover, none off the picture.
fn covered(which: u32, pixel: vec2<i32>) -> f32 {
    let size = vec2<i32>(textureDimensions(marks));
    if any(pixel < vec2<i32>(0)) || any(pixel >= size) {
        return 0.0;
    }
    return textureLoad(marks, pixel, 0)[which];
}
