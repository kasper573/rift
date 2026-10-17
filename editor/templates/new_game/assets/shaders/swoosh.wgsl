// A sweep of light crossing the site: a crisp crescent band that runs across the body over the
// layer's duration, trailing a wake that fades in steps behind it. On a cone it wipes from one
// edge to the other like a blade; along a path it runs from the start to the end; on any other
// site it runs straight across, bowed by `curve`. Drawn on the pixel grid.
//   color      the band's color; its alpha scales the whole sweep
//   thickness  the band's width, as a fraction of the sweep's span
//   sweeps     how many times it crosses during the layer
//   along      whether the sweep runs along the facing rather than across it
//   angle      degrees the sweep turns on a sprite or the screen
//   curve      how far the band bows into a crescent, as a fraction of the span
//   trail      how long the wake lingers behind the band, as a fraction of the span
//   clip       on a sprite, how much the sweep keeps to its pixels: 0 crosses the air just around it too
fn vfx(uv: vec2<f32>) -> vec4<f32> {
    let suv = site_uv(pixel_uv(uv));
    let image = sample(suv);
    let p = site_local(suv);
    let phase = fract(site.progress * max(params.sweeps, 0.01));
    let lengthwise = params.along >= 0.5;
    var sweep = 0.0;
    var span = 1.0;
    var across = 0.0;
    var inside = 1.0;
    if site.shape == SHAPE_CONE && !lengthwise {
        let half = site.extent.x * 0.5;
        span = 2.0 * half;
        sweep = atan2(p.y, p.x) + half;
        across = clamp(length(p) / max(site.extent.y, 0.01), 0.0, 1.0);
        inside = step(site_distance(suv), 0.0);
        sweep = sweep + params.curve * span * 0.5 * (1.0 - across);
    } else if site.kind == SITE_PATH && site.point_count >= 2u {
        let near = path_nearest(suv);
        sweep = near.y;
        across = clamp(near.x / PATH_HALF_WIDTH, 0.0, 1.0);
        inside = step(near.x, PATH_HALF_WIDTH);
        sweep = sweep + params.curve * 0.5 * (1.0 - across * across);
    } else if site.shape != SHAPE_NONE {
        let bounds = site_span();
        let lo = select(bounds.z, bounds.x, lengthwise);
        let hi = select(bounds.w, bounds.y, lengthwise);
        span = max(hi - lo, 0.01);
        let main = select(p.y, p.x, lengthwise);
        let other = select(p.x, p.y, lengthwise);
        let other_span = select(bounds.y - bounds.x, bounds.w - bounds.z, lengthwise);
        across = clamp(other / max(other_span * 0.5, 0.01), -1.0, 1.0);
        sweep = main - lo + params.curve * span * 0.5 * (1.0 - across * across);
        inside = step(site_distance(suv), 0.0);
    } else {
        let a = radians(params.angle);
        let heading = vec2<f32>(cos(a), sin(a));
        let centred = suv - 0.5;
        let main = dot(centred, heading);
        let other = dot(centred, vec2<f32>(-heading.y, heading.x));
        span = 1.2;
        across = clamp(other * 2.0, -1.0, 1.0);
        sweep = main + 0.6 + params.curve * 0.5 * (1.0 - across * across);
        let oval = step(site_reach(suv), 1.2);
        inside = oval * mix(1.0, step(0.5, image.a), clamp(params.clip, 0.0, 1.0));
    }
    let head = span * phase;
    let band = max(params.thickness * span, 0.001);
    let ahead = sweep - head;
    let edge = step(abs(ahead), band * 0.5);
    let wake = floor(clamp(1.0 + ahead / max(params.trail * span, 0.001), 0.0, 0.999) * 3.0) / 3.0 * step(ahead, 0.0) * 0.4;
    let settling = 1.0 - smoothstep(0.6, 1.0, site.progress);
    let glow = max(edge, wake) * inside * settling * params.color.a;
    return over(vec4<f32>(params.color.rgb, glow), image);
}
