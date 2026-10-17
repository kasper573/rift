
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
}

struct Site {
    rect: vec4<f32>,
    size: vec2<f32>,
    pixel: vec2<f32>,
    kind: u32,
    facing: f32,
    progress: f32,
    time: f32,
    overscan: vec2<f32>,
    shape: u32,
    point_count: u32,
    origin: vec2<f32>,
    extent: vec2<f32>,
    tiles: vec2<f32>,
    points: array<vec4<f32>, 8>,
}

const SITE_ACTOR: u32 = 0u;
const SITE_OBJECT: u32 = 1u;
const SITE_AREA: u32 = 2u;
const SITE_PATH: u32 = 3u;
const SITE_SCREEN: u32 = 4u;
const SITE_WIDGET: u32 = 5u;

const SHAPE_NONE: u32 = 0u;
const SHAPE_CIRCLE: u32 = 1u;
const SHAPE_RECT: u32 = 2u;
const SHAPE_CONE: u32 = 3u;
const SHAPE_LINE: u32 = 4u;

@group(2) @binding(0) var site_texture: texture_2d<f32>;
@group(2) @binding(1) var site_sampler: sampler;
@group(2) @binding(2) var<uniform> site: Site;
@group(2) @binding(3) var<uniform> params: Params;

// Every world site is drawn on a quad with a margin of two tiles on each side; the screen as is.
// `site_uv` runs 0..1 over the site itself and past that range on the margin. The margin is for
// what the effect sheds, drifting clear of the body and dissolving: smoke, sparks, mist, the
// outer wash of a glow. The effect proper keeps to the body the data describes.
fn site_uv(uv: vec2<f32>) -> vec2<f32> {
    return (uv - 0.5) * site.overscan + 0.5;
}

// The margin in tiles on each side of the site, along x and y.
fn site_margin() -> vec2<f32> {
    return site.tiles * (site.overscan - 1.0) * 0.5;
}

// The quad uv snapped to the centre of the world pixel it falls in, so a shader draws on the
// game's own pixel grid and its gradients and edges step with the pixel art instead of blurring
// over it.
fn pixel_uv(uv: vec2<f32>) -> vec2<f32> {
    let pixels = max(site.size * site.overscan, vec2<f32>(1.0));
    return (floor(uv * pixels) + 0.5) / pixels;
}

// `level` cut into `steps` flat bands, the top band at 1: light that steps the way pixel art
// shades instead of fading smoothly.
fn bands(level: f32, steps: f32) -> f32 {
    return floor(clamp(level, 0.0, 0.9999) * steps + 1.0) / steps * step(0.001, level);
}

// The site's own image at a site uv, transparent outside it.
fn sample(at: vec2<f32>) -> vec4<f32> {
    let inside = f32(all(at >= vec2<f32>(0.0)) && all(at <= vec2<f32>(1.0)));
    let texel = mix(site.rect.xy, site.rect.zw, clamp(at, vec2<f32>(0.0), vec2<f32>(1.0)));
    return textureSample(site_texture, site_sampler, texel) * inside;
}

// Every site has a body: an area's footprint, the corridor along a path, the image of a sprite or
// of the screen. The `site_*` functions below read it the same way whatever the site is, so one
// shader draws right on all of them.
const PATH_HALF_WIDTH: f32 = 0.5;

// Where a site uv lies in tiles from the site's point, along its facing then across it. An area's
// point is where it was aimed, a path's its first point, a sprite's or the screen's its centre.
// site.facing is in screen terms (y up) where uv, like tiles, runs y down.
fn site_local(uv: vec2<f32>) -> vec2<f32> {
    let offset = (uv - site.origin) * site.tiles;
    let heading = vec2<f32>(cos(site.facing), -sin(site.facing));
    let side = vec2<f32>(-heading.y, heading.x);
    return vec2<f32>(dot(offset, heading), dot(offset, side));
}

// How far the body reaches from the site's point in local tiles: along the facing (x: least,
// y: most) and across it (z: least, w: most).
fn site_span() -> vec4<f32> {
    switch site.shape {
        case 1u: {
            let r = site.extent.x;
            return vec4<f32>(-r, r, -r, r);
        }
        case 2u: {
            let half = site.extent * 0.5;
            return vec4<f32>(-half.x, half.x, -half.y, half.y);
        }
        case 3u: {
            let reach = site.extent.y;
            let wide = reach * sin(min(site.extent.x * 0.5, 1.5707964));
            return vec4<f32>(0.0, reach, -wide, wide);
        }
        case 4u: {
            let half = site.extent.y * 0.5;
            return vec4<f32>(0.0, site.extent.x, -half, half);
        }
        default: {
            if site.kind == SITE_PATH && site.point_count >= 2u {
                return path_span();
            }
            let half = site.tiles * 0.5;
            return vec4<f32>(-half.x, half.x, -half.y, half.y);
        }
    }
}

// The signed distance in tiles from a site uv to the edge of the site's body, negative inside it.
fn site_distance(uv: vec2<f32>) -> f32 {
    let p = site_local(uv);
    switch site.shape {
        case 1u: {
            return length(p) - site.extent.x;
        }
        case 2u: {
            return box_distance(p, site.extent * 0.5);
        }
        case 3u: {
            let half = site.extent.x * 0.5;
            let reach = length(p);
            let turned = abs(atan2(p.y, p.x)) - half;
            let to_side = reach * sin(clamp(turned, -1.5707964, 1.5707964));
            return max(reach - site.extent.y, to_side);
        }
        case 4u: {
            return box_distance(p - vec2<f32>(site.extent.x * 0.5, 0.0), site.extent * 0.5);
        }
        default: {
            if site.kind == SITE_PATH && site.point_count >= 2u {
                return path_distance(uv) - PATH_HALF_WIDTH;
            }
            return box_distance(p, site.tiles * 0.5);
        }
    }
}

// How far out a site uv lies from the site's point: 0 there, 1 at the far rim of the body and past
// 1 beyond it. That rim is a circle's or a sprite's edge all round, a rectangle's edge, a cone's
// arc, a line's far end or a path's end, so a front at a growing reach spreads out of the point,
// fans down a cone and runs the length of a line or path. The sides of a cone, a line or a path
// are not counted: a shader clips to them with site_distance where it matters.
fn site_reach(uv: vec2<f32>) -> f32 {
    let p = site_local(uv);
    switch site.shape {
        case 1u: {
            return length(p) / max(site.extent.x, 1e-4);
        }
        case 2u: {
            let half = max(site.extent * 0.5, vec2<f32>(1e-4));
            return max(abs(p.x) / half.x, abs(p.y) / half.y);
        }
        case 3u: {
            return length(p) / max(site.extent.y, 1e-4);
        }
        case 4u: {
            return p.x / max(site.extent.x, 1e-4);
        }
        default: {
            if site.kind == SITE_PATH && site.point_count >= 2u {
                return path_nearest(uv).y;
            }
            return length(p / max(site.tiles * 0.5, vec2<f32>(1e-4)));
        }
    }
}

// How much of the site's body a site uv covers, 1 inside and 0 out, softened over a tenth of a
// tile at the edge: a sprite's or the screen's is its image's own alpha.
fn site_cover(uv: vec2<f32>) -> f32 {
    if site.kind == SITE_ACTOR || site.kind == SITE_OBJECT || site.kind == SITE_SCREEN {
        return sample(uv).a;
    }
    return 1.0 - smoothstep(-0.05, 0.05, site_distance(uv));
}

// The site's image at a site uv, or, on a site that has none, a white sheet of its body for a
// shader that reshapes or recolors what it is given to work on.
fn sheet(at: vec2<f32>) -> vec4<f32> {
    if site.kind == SITE_ACTOR || site.kind == SITE_OBJECT || site.kind == SITE_SCREEN {
        return sample(at);
    }
    return vec4<f32>(1.0, 1.0, 1.0, site_cover(at) * 0.6);
}

// `effect` drawn over `image`: what a shader returns to show through to the sprite it is on.
fn over(effect: vec4<f32>, image: vec4<f32>) -> vec4<f32> {
    let a = effect.a + image.a * (1.0 - effect.a);
    let rgb = (effect.rgb * effect.a + image.rgb * image.a * (1.0 - effect.a)) / max(a, 1e-4);
    return vec4<f32>(rgb, a);
}

fn box_distance(p: vec2<f32>, half: vec2<f32>) -> f32 {
    let d = abs(p) - half;
    return length(max(d, vec2<f32>(0.0))) + min(max(d.x, d.y), 0.0);
}

// The reach of a path's corridor in local tiles, as site_span gives it.
fn path_span() -> vec4<f32> {
    var span = vec4<f32>(1e9, -1e9, 1e9, -1e9);
    for (var i = 0u; i < site.point_count; i++) {
        let p = site_local(path_point(i));
        span = vec4<f32>(min(span.x, p.x), max(span.y, p.x), min(span.z, p.y), max(span.w, p.y));
    }
    return span + vec4<f32>(-PATH_HALF_WIDTH, PATH_HALF_WIDTH, -PATH_HALF_WIDTH, PATH_HALF_WIDTH);
}

// A point of a path site, in site uv: the first is where the path starts.
fn path_point(index: u32) -> vec2<f32> {
    let word = site.points[min(index, 15u) / 2u];
    if (index % 2u) == 0u {
        return word.xy;
    }
    return word.zw;
}

// The distance in tiles from a site uv to the nearest stretch of a path site, and how far along the
// whole path, 0 at its start and 1 at its end, the nearest point of it lies: x and y of the result.
fn path_nearest(uv: vec2<f32>) -> vec2<f32> {
    if site.point_count < 2u {
        return vec2<f32>(length((uv - path_point(0u)) * site.tiles), 0.0);
    }
    var total = 0.0;
    for (var i = 1u; i < site.point_count; i++) {
        total += length((path_point(i) - path_point(i - 1u)) * site.tiles);
    }
    var nearest = 1e9;
    var along = 0.0;
    var walked = 0.0;
    for (var i = 1u; i < site.point_count; i++) {
        let a = path_point(i - 1u) * site.tiles;
        let b = path_point(i) * site.tiles;
        let p = uv * site.tiles;
        let ab = b - a;
        let stretch = max(length(ab), 1e-4);
        let t = clamp(dot(p - a, ab) / (stretch * stretch), 0.0, 1.0);
        let d = length(p - (a + ab * t));
        if d < nearest {
            nearest = d;
            along = (walked + t * stretch) / max(total, 1e-4);
        }
        walked += stretch;
    }
    return vec2<f32>(nearest, along);
}

// The distance in tiles from a site uv to the nearest stretch of a path site.
fn path_distance(uv: vec2<f32>) -> f32 {
    return path_nearest(uv).x;
}

// How long a path site is in tiles, every stretch added up.
fn path_length() -> f32 {
    var total = 0.0;
    for (var i = 1u; i < site.point_count; i++) {
        total += length((path_point(i) - path_point(i - 1u)) * site.tiles);
    }
    return total;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let painted = vfx(in.uv);
    return vec4<f32>(painted.rgb, painted.a * quad_guard(in.uv));
}

// Nothing may stop hard at the edge of the quad a site is drawn on: whatever a shader paints fades
// out over the outer half tile of the margin. The screen is drawn without a margin.
fn quad_guard(uv: vec2<f32>) -> f32 {
    if site.kind == SITE_SCREEN {
        return 1.0;
    }
    let quad = site.tiles * site.overscan;
    let border = min(min(uv.x, 1.0 - uv.x) * quad.x, min(uv.y, 1.0 - uv.y) * quad.y);
    return smoothstep(0.0, 0.5, border);
}
