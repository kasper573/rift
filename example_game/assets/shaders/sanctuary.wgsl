// Holy ground over the site: shafts of light stand up out of the floor and reach above the body,
// stepping fainter as they rise over the margin, a one-pixel seam marks the edge of the blessed
// body and pixel motes float up out of it, all fading as the layer ends. The body may be any
// shape, a stretch of path, or the sprite the blessing stands on. Drawn on the pixel grid.
//   color  the light; its alpha scales the whole effect
//   rays   how many shafts of light stand across the body
fn vfx(uv: vec2<f32>) -> vec4<f32> {
    let suv = site_uv(pixel_uv(uv));
    let image = sample(suv);
    let on_sprite = site.kind == SITE_ACTOR || site.kind == SITE_OBJECT;
    let pixel = 1.0 / max(site.size, vec2<f32>(1.0));
    let distance = site_distance(suv);
    let inside = select(step(distance, 0.0), site_cover(suv), on_sprite);
    let q = suv * site.tiles;
    let t = site.progress;
    let tick = floor(site.time * 12.0) / 12.0;
    let rising = smoothstep(0.0, 0.2, t);
    let settling = 1.0 - smoothstep(0.7, 1.0, t);
    let rays_across = max(params.rays, 0.0);
    let lane = fract(suv.x * rays_across + 0.5 + 0.1 * sin(tick * 0.9 + suv.x * 7.0));
    let shaft = step(0.6, lane) * step(lane, 0.85);
    var standing = inside * 0.45;
    let rise = select(0.5, site.tiles.y * 0.12, on_sprite);
    for (var k = 0u; k < 3u; k++) {
        let below = suv + vec2<f32>(0.0, (f32(k) + 1.0) * rise / max(site.tiles.y, 0.01));
        let foot = select(step(site_distance(below), 0.0), site_cover(below), on_sprite);
        standing = max(standing, foot * (0.45 - 0.13 * f32(k)) * (1.0 - inside));
    }
    let beams = shaft * standing;
    let floor_glow = inside * (0.1 + 0.05 * step(0.5, fract((q.x + q.y) * 2.0 - tick * 0.4)));
    let seam = step(abs(distance), pixel.x * site.tiles.x) * f32(!on_sprite) * 0.8;
    var motes = 0.0;
    for (var i = 0u; i < 14u; i++) {
        let seed = f32(i) * 5.17;
        let life = fract(tick * (0.25 + 0.2 * hash(seed + 1.0)) + hash(seed + 2.0));
        let birth = vec2<f32>(hash(seed + 3.0), hash(seed + 4.0));
        let at = birth - vec2<f32>(0.0, life * 1.5 / max(site.tiles.y, 0.01));
        let d = abs(suv - at) / pixel;
        let born = select(step(site_distance(birth), 0.0), site_cover(birth), on_sprite);
        motes = max(motes, step(max(d.x, d.y), 1.0) * born * step(life, 0.8) * step(0.3, hash(seed + floor(tick * 12.0))));
    }
    let glow = clamp(max(max(floor_glow, beams), max(seam, motes)), 0.0, 1.0) * rising * settling;
    let rgb = mix(params.color.rgb, vec3<f32>(1.0, 1.0, 0.95), step(0.5, max(motes, seam)));
    return over(vec4<f32>(rgb, glow * params.color.a), image);
}

fn hash(x: f32) -> f32 {
    return fract(sin(x * 12.9898) * 43758.5453);
}
