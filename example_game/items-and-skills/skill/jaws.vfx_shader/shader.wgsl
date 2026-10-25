// A pair of jaws snapping shut on the site: an upper and a lower row of teeth, each set in its
// gum, close in from above and below the site's middle, slow to open and quick to shut, and clamp
// together with a snap, the teeth of one row meshing between the other's. They hold the bite for
// a moment, shivering as they grip, then let go and fade. On a sprite the jaws close over its
// middle and leave the body bruised between the teeth; on an area they close over its point.
// Drawn on the pixel grid in flat tones.
//   color  the teeth
//   gums   the jaws the teeth grow from
//   teeth  how many teeth in each row
//   size   how wide the bite is, in metres
const SNAP: f32 = 0.3;
const SHIVER: f32 = 0.12;
const LET_GO: f32 = 0.62;

fn vfx(uv: vec2<f32>) -> vec4<f32> {
    let suv = site_uv(pixel_uv(uv));
    let image = sample(suv);
    let p = site_local(suv);
    let pixel = site.metres.x / max(site.size.x, 1.0);
    let t = site.progress;
    let on_sprite = site.kind == SITE_ACTOR || site.kind == SITE_OBJECT;
    let width = max(params.size, 0.2);
    let half_w = width * 0.5;
    let closing = pow(clamp(t / SNAP, 0.0, 1.0), 1.7);
    let tooth = 0.22 * width;
    let gap = mix(0.55 * width, tooth * 0.3, closing);
    let shivering = step(SNAP, t) * step(t, SNAP + SHIVER);
    let jitter = shivering * pixel * (step(0.5, fract(site.time * 24.0)) * 2.0 - 1.0);
    let x = p.x;
    let reach = x / half_w;
    let bow = 0.3 * half_w * reach * reach;
    let within = step(abs(reach), 1.0);
    let gum = 0.14 * width + pixel;
    let period = width / max(params.teeth, 1.0);
    let upper = jaw(p.y + gap + bow + jitter, fract((x + half_w) / period), gum, tooth, within);
    let lower = jaw(-(p.y - gap - bow - jitter), fract((x + half_w) / period + 0.5), gum, tooth, within);
    let teeth = max(upper.x, lower.x);
    let tips = max(upper.y, lower.y);
    let gums = max(upper.z, lower.z);
    let rim = max(upper.w, lower.w);
    let snap = step(abs(p.y + jitter), pixel * 1.5) * within * step(SNAP, t) * step(t, SNAP + 0.08);
    let fade = 1.0 - floor(clamp((t - LET_GO) / (1.0 - LET_GO), 0.0, 0.999) * 3.0) / 3.0;
    let bruise = step(abs(p.y), 0.16 * width) * within * image.a * step(SNAP, t) * select(0.0, 1.0, on_sprite);
    let bruised = over(vec4<f32>(0.45, 0.05, 0.12, bruise * 0.45 * fade), image);
    var rgb = mix(params.gums.rgb * 0.55, params.gums.rgb, step(0.5, 1.0 - rim));
    rgb = mix(rgb, params.color.rgb, step(0.5, teeth));
    rgb = mix(rgb, vec3<f32>(1.0), max(step(0.5, tips), snap));
    let a = max(max(max(teeth, gums), rim), snap) * fade * params.color.a;
    return over(vec4<f32>(rgb, a), bruised);
}

// One jaw, `y` metres inward from its gum line (teeth grow toward positive y) and `f` the phase
// along the row: whether the point is on a tooth, on a tooth's white tip, on the gum, or on the
// gum's dark outer rim.
fn jaw(y: f32, f: f32, gum: f32, tooth: f32, within: f32) -> vec4<f32> {
    let pixel = site.metres.x / max(site.size.x, 1.0);
    let point = 1.0 - 2.0 * abs(f - 0.5);
    let height = tooth * point;
    let on_tooth = step(0.0, y) * step(y, height) * within * step(0.2, point);
    let tip = on_tooth * step(height - pixel * 1.5, y) * step(tooth * 0.45, height);
    let on_gum = step(-gum, y) * step(y, 0.0) * within;
    let rim = step(-gum, y) * step(y, -gum + pixel) * within;
    return vec4<f32>(on_tooth, tip, on_gum, rim);
}
