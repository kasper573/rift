// A blade's slash across the site: a crescent stroke, thick in the middle and drawn to a point at
// each end, swept from one end to the other in a few frames, a spark at its leading tip, then
// gone in steps. More streaks cut side by side, each a beat after the last. On a sprite the
// stroke runs over the body and leaves a dark cut along its line for a moment, so the blow lands
// on the one it hit rather than beside them. Drawn on the pixel grid in two tones.
//   color    the stroke; its alpha scales the whole slash
//   angle    degrees the stroke is turned from level, positive turning it clockwise
//   length   how long the stroke is, in metres
//   streaks  how many strokes cut side by side
const SWEEP: f32 = 0.45;
const SPACING: f32 = 0.26;
const BEAT: f32 = 0.08;

fn vfx(uv: vec2<f32>) -> vec4<f32> {
    let suv = site_uv(pixel_uv(uv));
    let image = sample(suv);
    let p = site_local(suv);
    let pixel = site.metres.x / max(site.size.x, 1.0);
    let t = site.progress;
    let on_sprite = site.kind == SITE_ACTOR || site.kind == SITE_OBJECT;
    let a = radians(params.angle);
    let heading = vec2<f32>(cos(a), sin(a));
    let side = vec2<f32>(-heading.y, heading.x);
    let half = max(params.length, 0.2) * 0.5;
    let count = u32(clamp(params.streaks, 1.0, 4.0));
    var core = 0.0;
    var edge = 0.0;
    var cut = 0.0;
    for (var k = 0u; k < count; k++) {
        let offset = (f32(k) - f32(count - 1u) * 0.5) * SPACING;
        let delay = f32(k) * BEAT;
        let tk = clamp((t - delay) / max(1.0 - delay, 0.01), 0.0, 1.0);
        let s = dot(p, heading) / half;
        let v = dot(p, side) - offset;
        let bow = 0.22 * half * (1.0 - s * s);
        let thick = 0.1 * half * (1.0 - s * s) + pixel;
        let d = v - bow;
        let within = step(abs(s), 1.0);
        let head = -1.0 + 2.0 * pow(clamp(tk / SWEEP, 0.0, 1.0), 0.7);
        let drawn = step(s, head) * within * step(0.001, tk);
        let gone = 1.0 - floor(clamp((tk - SWEEP) / (1.0 - SWEEP), 0.0, 0.999) * 3.0) / 3.0;
        edge = max(edge, step(abs(d), thick) * drawn * gone);
        core = max(core, step(abs(d), thick * 0.45) * drawn * gone);
        let tip = heading * head * half + side * (offset + 0.22 * half * (1.0 - head * head));
        let square = abs(p - tip) / pixel;
        core = max(core, step(max(square.x, square.y), 1.5) * step(0.001, tk) * step(tk, SWEEP));
        let scar = step(abs(d), pixel) * within * step(SWEEP * 0.6, tk) * (1.0 - smoothstep(0.7, 1.0, tk));
        cut = max(cut, scar);
    }
    let marked = cut * image.a * select(0.0, 1.0, on_sprite);
    let scarred = over(vec4<f32>(0.08, 0.0, 0.06, marked * 0.55), image);
    let rgb = mix(params.color.rgb, vec3<f32>(1.0), core);
    return over(vec4<f32>(rgb, max(core, edge) * params.color.a), scarred);
}
