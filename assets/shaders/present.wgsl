#import bevy_sprite::mesh2d_vertex_output::VertexOutput

struct Cut {
    style: u32,
    out_progress: f32,
    in_progress: f32,
    time: f32,
    tile: f32,
    pixel: f32,
    from_radius: f32,
    to_radius: f32,
    from_anchor: vec2<f32>,
    to_anchor: vec2<f32>,
    from_grid: vec2<f32>,
    to_grid: vec2<f32>,
    sweep: vec2<f32>,
    from_reach: f32,
    to_reach: f32,
}

@group(2) @binding(0) var world_texture: texture_2d<f32>;
@group(2) @binding(1) var world_sampler: sampler;
@group(2) @binding(2) var<uniform> tint: vec4<f32>;
@group(2) @binding(3) var snapshot_texture: texture_2d<f32>;
@group(2) @binding(4) var<uniform> cut: Cut;

const CRUMBLE: u32 = 0u;
const TILE_WAVE: u32 = 1u;
const SWEEP: u32 = 2u;
const IRIS: u32 = 3u;
const MOSAIC: u32 = 4u;
const DITHER: u32 = 5u;

const VOID: vec3<f32> = vec3<f32>(0.0030, 0.0034, 0.0052);

var<private> size: vec2<f32>;
var<private> per_pixel: vec2<f32>;

// pixel upscaling, the screen transitions, and screen tint (e.g. the death wash; what triggers
// either lives in the game systems)

@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    size = vec2<f32>(textureDimensions(world_texture));
    let texel = mesh.uv * size;
    per_pixel = fwidth(texel);
    var color = textureSampleLevel(world_texture, world_sampler, crisp(texel), 0.0);
    if cut.in_progress < 1.0 {
        color = vec4<f32>(transition(texel), 1.0);
    }
    if tint.x > 0.5 {
        color = vec4<f32>(min(color.r + 160.0 / 255.0, 1.0), color.g / 3.0, color.b / 3.0, color.a);
    }
    return color;
}

fn crisp(texel: vec2<f32>) -> vec2<f32> {
    let from_center = fract(texel) - 0.5;
    let ramp = vec2<f32>(0.5) - 0.5 * per_pixel;
    let f = (from_center - clamp(from_center, -ramp, ramp)) / per_pixel + 0.5;
    return (floor(texel) + f) / size;
}

fn live(texel: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(world_texture, world_sampler, crisp(texel), 0.0).rgb;
}

fn held(texel: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(snapshot_texture, world_sampler, crisp(texel), 0.0).rgb;
}

fn transition(p: vec2<f32>) -> vec3<f32> {
    switch cut.style {
        case CRUMBLE: { return crumble(p); }
        case TILE_WAVE: { return tile_wave(p); }
        case SWEEP: { return sweep(p); }
        case IRIS: { return iris(p); }
        case MOSAIC: { return mosaic(p); }
        case DITHER: { return dither(p); }
        default: { return fade(p); }
    }
}

fn ease(x: f32) -> f32 {
    let t = clamp(x, 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

fn world_pixel(p: vec2<f32>, grid: vec2<f32>) -> vec2<f32> {
    return grid + (floor((p - grid) / cut.pixel) + 0.5) * cut.pixel;
}

fn cell_of(p: vec2<f32>, grid: vec2<f32>) -> vec2<f32> {
    return floor((p - grid) / cut.tile);
}

fn cell_center(cell: vec2<f32>, grid: vec2<f32>) -> vec2<f32> {
    return grid + (cell + 0.5) * cut.tile;
}

fn tile_distance(cell: vec2<f32>, anchor: vec2<f32>, grid: vec2<f32>) -> f32 {
    let apart = abs(cell - cell_of(anchor, grid));
    return apart.x + apart.y;
}

fn hash(cell: vec2<f32>) -> f32 {
    return fract(sin(dot(cell, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}

fn on_screen(p: vec2<f32>) -> bool {
    return all(p >= vec2<f32>(0.0)) && all(p < size);
}

fn fade(p: vec2<f32>) -> vec3<f32> {
    return mix(mix(held(p), VOID, ease(cut.out_progress)), live(p), ease(cut.in_progress));
}

fn iris(p: vec2<f32>) -> vec3<f32> {
    let spotlight = 1.5 * cut.tile;
    if cut.in_progress < 0.2 {
        var r = mix(cut.from_radius, spotlight, ease(cut.out_progress));
        if cut.in_progress > 0.0 {
            r = mix(spotlight, 0.0, ease(cut.in_progress / 0.2));
        }
        if distance(world_pixel(p, cut.from_grid), cut.from_anchor) < r {
            return held(p);
        }
        return VOID;
    }
    let k = (cut.in_progress - 0.2) / 0.8;
    let r = cut.to_radius * (1.0 - pow(1.0 - k, 3.0));
    if distance(world_pixel(p, cut.to_grid), cut.to_anchor) < r {
        return live(p);
    }
    return VOID;
}

fn diamond(p: vec2<f32>, grid: vec2<f32>) -> f32 {
    let q = p - grid;
    let inside = q - floor(q / cut.tile) * cut.tile;
    let l = (floor(inside / cut.pixel) + 0.5) * cut.pixel / cut.tile - 0.5;
    return abs(l.x) + abs(l.y);
}

// The character's own tiles stay through the hold, so a late arrival still has something to show.
fn leaving(cell: vec2<f32>, spread: f32, kept: f32) -> f32 {
    let d = tile_distance(cell, cut.from_anchor, cut.from_grid);
    if d <= 1.0 {
        return clamp(cut.in_progress / kept, 0.0, 1.0);
    }
    let outer = (d - 1.0) / max(cut.from_reach - 1.0, 1.0);
    return clamp((cut.out_progress - (1.0 - outer) * spread) / (1.0 - spread), 0.0, 1.0);
}

fn arriving(cell: vec2<f32>, start: f32, spread: f32, span: f32) -> f32 {
    let d = tile_distance(cell, cut.to_anchor, cut.to_grid) / max(cut.to_reach, 1.0);
    return clamp((cut.in_progress - (start + d * spread)) / span, 0.0, 1.0);
}

fn tile_wave(p: vec2<f32>) -> vec3<f32> {
    let gone = leaving(cell_of(p, cut.from_grid), 0.6, 0.18);
    let come = arriving(cell_of(p, cut.to_grid), 0.18, 0.5, 0.32);
    if diamond(p, cut.to_grid) < come {
        return live(p);
    }
    if diamond(p, cut.from_grid) < 1.0 - gone {
        return held(p);
    }
    return VOID;
}

fn rotate(v: vec2<f32>, angle: f32) -> vec2<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return vec2<f32>(c * v.x - s * v.y, s * v.x + c * v.y);
}

fn falling(under: vec3<f32>, p: vec2<f32>, cell: vec2<f32>, k: f32) -> vec3<f32> {
    if k <= 0.0 || k >= 1.0 {
        return under;
    }
    let e = k * k;
    let home = cell_center(cell, cut.from_grid);
    let turn = (hash(cell) - 0.5) * 0.9 * e;
    let moved = world_pixel(p, cut.from_grid) - home - vec2<f32>(0.0, 0.6 * cut.tile * e);
    let local = rotate(moved, -turn) / (1.0 - 0.5 * e);
    let source = home + local;
    if any(abs(local) > vec2<f32>(0.5 * cut.tile)) || !on_screen(source) {
        return under;
    }
    return mix(under, held(source), 1.0 - pow(k, 1.6));
}

fn rising(under: vec3<f32>, p: vec2<f32>, cell: vec2<f32>, k: f32) -> vec3<f32> {
    if k <= 0.0 || k >= 1.0 {
        return under;
    }
    let e = 1.0 - pow(1.0 - k, 3.0);
    let home = cell_center(cell, cut.to_grid);
    let turn = (hash(cell) - 0.5) * 0.6 * (1.0 - e);
    let moved = world_pixel(p, cut.to_grid) - home - vec2<f32>(0.0, (1.0 - e) * 0.6 * cut.tile);
    let local = rotate(moved, -turn) / (0.5 + 0.5 * e);
    let source = home + local;
    if any(abs(local) > vec2<f32>(0.5 * cut.tile)) || !on_screen(source) {
        return under;
    }
    return mix(under, live(source), min(1.0, k * 2.0));
}

// Tiles only ever shrink and sink, so a pixel can be covered by its own tile or the one above it,
// never any other.
fn crumble(p: vec2<f32>) -> vec3<f32> {
    let above = vec2<f32>(0.0, -1.0);
    let held_cell = cell_of(p, cut.from_grid);
    let live_cell = cell_of(p, cut.to_grid);
    let fall_here = leaving(held_cell, 0.55, 0.22);
    let fall_above = leaving(held_cell + above, 0.55, 0.22);
    let rise_here = arriving(live_cell, 0.22, 0.48, 0.3);
    let rise_above = arriving(live_cell + above, 0.22, 0.48, 0.3);
    if fall_here <= 0.0 {
        return held(p);
    }
    if rise_here >= 1.0 {
        return live(p);
    }
    var color = VOID;
    if fall_here >= fall_above {
        color = falling(color, p, held_cell, fall_here);
        color = falling(color, p, held_cell + above, fall_above);
    } else {
        color = falling(color, p, held_cell + above, fall_above);
        color = falling(color, p, held_cell, fall_here);
    }
    if rise_here <= rise_above {
        color = rising(color, p, live_cell, rise_here);
        color = rising(color, p, live_cell + above, rise_above);
    } else {
        color = rising(color, p, live_cell + above, rise_above);
        color = rising(color, p, live_cell, rise_here);
    }
    return color;
}

fn sweep(p: vec2<f32>) -> vec3<f32> {
    let v = cut.sweep;
    let n = vec2<f32>(-v.y, v.x);
    let corners = array<vec2<f32>, 4>(vec2<f32>(0.0), vec2<f32>(size.x, 0.0), vec2<f32>(0.0, size.y), size);
    var along = vec2<f32>(1e9, -1e9);
    var across = vec2<f32>(1e9, -1e9);
    for (var i = 0; i < 4; i++) {
        along = vec2<f32>(min(along.x, dot(corners[i], v)), max(along.y, dot(corners[i], v)));
        across = vec2<f32>(min(across.x, dot(corners[i], n)), max(across.y, dot(corners[i], n)));
    }
    let w = world_pixel(p, cut.from_grid);
    let a = (dot(w, v) - along.x) / (along.y - along.x);
    let b = (dot(w, n) - across.x) / (across.y - across.x);
    let slant = 0.22;
    let edge = a + (b - 0.5) * slant + fract(floor(b * 7.0) * 0.618) * 0.1;
    let lo = -slant * 0.5 - 0.01;
    let hi = 1.0 + slant * 0.5 + 0.11;
    if edge < mix(lo, hi, ease(cut.in_progress)) {
        return live(p);
    }
    if edge < mix(lo, hi, ease(cut.out_progress)) {
        let lines = step(0.82, fract(dot(w, v) / (2.0 * cut.tile) + dot(w, n) / (5.0 * cut.tile) - cut.time * 1.4));
        return VOID + lines * 0.008;
    }
    return held(p);
}

fn block(p: vec2<f32>, grid: vec2<f32>, width: f32) -> vec2<f32> {
    return grid + (floor((p - grid) / width) + 0.5) * width;
}

fn mosaic(p: vec2<f32>) -> vec3<f32> {
    if cut.in_progress <= 0.0 {
        let blocks = floor(cut.out_progress * 15.0 + 0.5) + 1.0;
        var color = held(p);
        if blocks > 1.0 {
            color = held(block(p, cut.from_grid, cut.pixel * blocks));
        }
        return color * mix(1.0, 0.8, cut.out_progress);
    }
    if cut.in_progress < 0.3 {
        let k = floor(cut.in_progress / 0.3 * 4.0) / 4.0;
        return mix(held(block(p, cut.from_grid, cut.tile)), live(block(p, cut.to_grid, cut.tile)), k) * 0.8;
    }
    let k = (cut.in_progress - 0.3) / 0.7;
    let blocks = floor((1.0 - k) * 15.0 + 0.5) + 1.0;
    var color = live(p);
    if blocks > 1.0 {
        color = live(block(p, cut.to_grid, cut.pixel * blocks));
    }
    return color * mix(0.8, 1.0, k);
}

fn bayer2(a: vec2<f32>) -> f32 {
    let f = floor(a);
    return fract(dot(f, vec2<f32>(0.5, f.y * 0.75)));
}

fn bayer8(a: vec2<f32>) -> f32 {
    return (bayer2(0.25 * a) * 0.25 + bayer2(0.5 * a)) * 0.25 + bayer2(a);
}

fn dither(p: vec2<f32>) -> vec3<f32> {
    let from_order = 0.5 * bayer8(floor((p - cut.from_grid) / cut.pixel))
        + 0.5 * (1.0 - clamp(distance(world_pixel(p, cut.from_grid), cut.from_anchor) / cut.from_radius, 0.0, 1.0));
    let to_order = 0.5 * bayer8(floor((p - cut.to_grid) / cut.pixel))
        + 0.5 * clamp(distance(world_pixel(p, cut.to_grid), cut.to_anchor) / cut.to_radius, 0.0, 1.0);
    if to_order < cut.in_progress {
        return live(p);
    }
    if from_order < cut.out_progress {
        return VOID;
    }
    return held(p);
}
