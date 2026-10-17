// Lightning arcing along the site: from the first point of a path through every other, or, on any
// other site, from the near end of its body to the far end, a one-pixel bolt that rewrites itself
// many times a second, fainter bolts wandering beside it, a flare where each point stands, and a
// banded glow around it all that dies with the layer. The bolts keep to their course; only the
// glow washes out over the margin. On a sprite the bolt strikes down through it.
//   color   the glow around the bolts
//   core    the color of the bolts themselves
//   jitter  how far the bolts stray from the course, in tiles
//   bolts   how many bolts run the course; the first is the brightest
fn vfx(uv: vec2<f32>) -> vec4<f32> {
    let suv = site_uv(pixel_uv(uv));
    let image = sample(suv);
    let p = site_local(suv);
    let pixel = site.tiles.x / max(site.size.x, 1.0);
    let frame = floor(site.time * 24.0);
    let flicker = 0.75 + 0.25 * step(0.5, hash(frame * 0.37 + 1.3));
    let count = u32(clamp(params.bolts, 1.0, 4.0));
    let jitter = max(params.jitter, 0.0);
    var light = 0.0;
    var glow = 0.0;
    for (var i = 1u; i < course_count(); i++) {
        let a = course_point(i - 1u);
        let ab = course_point(i) - a;
        let stretch = max(length(ab), 1e-4);
        let t = clamp(dot(p - a, ab) / (stretch * stretch), 0.0, 1.0);
        let off = length(p - (a + ab * t));
        let side = ab.x * (p.y - a.y) - ab.y * (p.x - a.x);
        let signed = select(off, -off, side < 0.0);
        let run = t * stretch;
        let pinned = smoothstep(0.0, 0.3, run) * (1.0 - smoothstep(stretch - 0.3, stretch, run));
        for (var b = 0u; b < count; b++) {
            let seed = f32(b) * 17.9 + f32(i) * 7.3 + frame * 3.1;
            let main = b == 0u;
            let stray = (noise(run * 5.5 + seed) - 0.5) * 2.0 * jitter * pinned * select(1.6, 1.0, main);
            let d = abs(signed - stray);
            light = max(light, step(d, pixel * select(0.6, 1.1, main)) * select(0.55, 1.0, main));
            glow = max(glow, (1.0 - clamp(d / 0.55, 0.0, 1.0)) * select(0.5, 1.0, main));
        }
    }
    var flare = 0.0;
    for (var i = 0u; i < course_count(); i++) {
        let at = p - course_point(i);
        flare = max(flare, (1.0 - clamp(length(at) / 0.5, 0.0, 1.0)) * select(0.9, 0.6, i == 0u));
    }
    let dying = 1.0 - smoothstep(0.45, 1.0, site.progress);
    let bolt = light * flicker * dying;
    let beyond = 1.0 - floor(clamp(max(site_distance(suv), 0.0) / 1.5, 0.0, 1.0) * 3.0) / 3.0;
    let halo = floor(clamp(max(glow, flare), 0.0, 0.999) * 3.0) / 3.0 * 0.45 * flicker * dying * beyond;
    let rgb = mix(params.color.rgb, params.core.rgb, step(0.5, bolt));
    return over(vec4<f32>(rgb, max(bolt * params.core.a, halo * params.color.a)), image);
}

// The points the bolt runs through, in local tiles: a path's own, else the two ends of the body:
// across a sprite from its head to its feet, along anything else.
fn course_count() -> u32 {
    return select(2u, site.point_count, site.point_count >= 2u);
}

fn course_point(index: u32) -> vec2<f32> {
    if site.point_count >= 2u {
        return site_local(path_point(index));
    }
    let span = site_span();
    if site.shape == SHAPE_NONE {
        return vec2<f32>(0.0, select(span.w, span.z, index == 0u));
    }
    return vec2<f32>(select(span.y, span.x, index == 0u), 0.0);
}

fn hash(x: f32) -> f32 {
    return fract(sin(x * 12.9898) * 43758.5453);
}

fn noise(x: f32) -> f32 {
    let i = floor(x);
    let f = fract(x);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(hash(i), hash(i + 1.0), u);
}
