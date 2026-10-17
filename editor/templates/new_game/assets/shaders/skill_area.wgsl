// The ground a skill is aimed at, as a clean ring of light in the shape of the area: a crisp
// one-pixel rim, a glow that steps away from it on both sides, a breath of light pulsing along
// it and a floor so faint the ground stays readable. Kept strictly to the area, since this is the
// shape the player reads. hostile is set for a skill that harms, which trades the calm colors for
// the hostile ones.
fn vfx(uv: vec2<f32>) -> vec4<f32> {
    let suv = site_uv(pixel_uv(uv));
    let distance = site_distance(suv);
    let pixel = site.tiles.x / max(site.size.x, 1.0);
    let inside = step(distance, 0.0);
    let tick = floor(site.time * 12.0) / 12.0;
    let color = mix(params.color, params.hostile_color, clamp(params.hostile, 0.0, 1.0));
    let rim_color = mix(params.rim, params.hostile_rim, clamp(params.hostile, 0.0, 1.0));
    let breath = 0.8 + 0.2 * step(0.5, fract(tick * 0.8));
    let rim = step(abs(distance), pixel);
    let inner = floor((1.0 - clamp(-distance / 0.3, 0.0, 0.999)) * 3.0) / 3.0 * inside * 0.45;
    let outer = floor((1.0 - clamp(distance / 0.2, 0.0, 0.999)) * 2.0) / 2.0 * (1.0 - inside) * 0.35;
    let floor_glow = inside * 0.07;
    let glow = max(max(inner, outer), floor_glow) * color.a * breath;
    let edge = rim * rim_color.a;
    let rgb = mix(color.rgb, rim_color.rgb, rim);
    return vec4<f32>(rgb, clamp(max(glow, edge) * params.strength, 0.0, 1.0));
}
