// Specks of light drifting up through the site: embers over a burning actor, motes of blessing,
// bubbles of poison. Over a sprite each mote is born on the sprite's own pixels and rises off it,
// brightest when born on the body; over an area or along a path it is born anywhere on the body
// and rises up out of it, over the margin, where it fades. Each is a pixel square that twinkles, rising at its own pace, over and over for
// as long as the layer plays. Drawn over the site's own image when it has one.
//   color   the motes' color; its alpha scales them all
//   count   how many motes are in the air at once
//   speed   how fast they rise, in site heights per second over a sprite, tiles per second on the ground
//   size    a mote's radius, as a fraction of the site's width over a sprite, of a tile on the ground
//   spread  how wide the swarm is, as a fraction of the site's width
fn vfx(uv: vec2<f32>) -> vec4<f32> {
    let suv = site_uv(pixel_uv(uv));
    let image = sample(suv);
    let count = u32(clamp(params.count, 1.0, 64.0));
    let on_ground = site.kind == SITE_AREA || site.kind == SITE_PATH;
    let pixel = 1.0 / max(site.size, vec2<f32>(1.0));
    let across = select(site.size.x, site.size.x / max(site.tiles.x, 0.01), on_ground);
    let radius = max(round(max(params.size, 0.005) * across), 1.0);
    let tick = floor(site.time * 12.0) / 12.0;
    var light = 0.0;
    for (var i = 0u; i < count; i++) {
        let seed = f32(i) * 7.31;
        let pace = 0.6 + 0.8 * hash(seed + 1.7);
        let life = fract(tick * params.speed * pace * 0.5 + hash(seed + 3.1));
        let sway = sin(tick * (1.5 + hash(seed + 9.2) * 2.0) + seed) * 0.06;
        var at = vec2<f32>(0.0);
        var born = 1.0;
        if on_ground {
            let birth = vec2<f32>(hash(seed + 5.3), hash(seed + 13.7));
            born = site_cover(birth);
            at = birth + vec2<f32>(sway, -life * 1.5 / max(site.tiles.y, 0.01));
        } else {
            let lane = (hash(seed + 5.3) - 0.5) * params.spread;
            let birth = vec2<f32>(0.5 + lane, 0.25 + 0.7 * hash(seed + 13.7));
            born = max(site_cover(birth), 0.35);
            at = birth + vec2<f32>(sway, -life * 0.6);
        }
        let d = abs(suv - at) / pixel;
        let twinkle = step(0.35, hash(seed + floor(tick * 12.0) * 0.37));
        let square = step(max(d.x, d.y), radius - 0.5 + twinkle * 0.5);
        light = max(light, square * born * step(life, 0.85) * bands(sin(life * 3.1415926), 2.0));
    }
    let settling = 1.0 - smoothstep(0.75, 1.0, site.progress);
    let glow = clamp(light, 0.0, 1.0) * settling * params.color.a;
    return over(vec4<f32>(params.color.rgb, glow), image);
}

fn hash(x: f32) -> f32 {
    return fract(sin(x * 12.9898) * 43758.5453);
}
