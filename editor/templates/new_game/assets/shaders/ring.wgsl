// A ring that spreads from the site's point out past the rim of its body: a crisp bright line with
// a banded wake fading behind it, round on a circle or a sprite, a growing rectangle on one, an
// arc fanning down a cone, a bar running the length of a line or a path. It runs on a little past
// the rim and dissolves over the margin. Drawn on the pixel grid; the site's own image shows
// through it.
//   color  the ring; its alpha scales it
//   width  how thick the ring is, as a fraction of the way from the point to the rim
fn vfx(uv: vec2<f32>) -> vec4<f32> {
    let suv = site_uv(pixel_uv(uv));
    let image = sample(suv);
    let reach = site_reach(suv);
    let head = site.progress * 1.3;
    let width = max(params.width, 0.02);
    let behind = reach - head;
    let line = step(abs(behind), width * 0.5);
    let wake = floor(smoothstep(-width * 2.0, 0.0, behind) * step(behind, 0.0) * 3.0) / 3.0 * 0.4;
    let out = 1.0 - floor(clamp(site_distance(suv) / 0.5, 0.0, 1.0) * 2.0) / 2.0;
    let on_ground = site.kind == SITE_AREA || site.kind == SITE_PATH;
    let kept = select(1.0, out, on_ground);
    let spent = 1.0 - smoothstep(0.7, 1.0, site.progress);
    let glow = max(line, wake) * kept * spent * params.color.a;
    let rgb = mix(params.color.rgb, vec3<f32>(1.0), line * 0.5);
    return over(vec4<f32>(rgb, glow), image);
}
