// A ward wrapped around its site. On a sprite it is the sprite itself that is warded: a
// one-pixel outline hugging its silhouette with a glint running round it, and a faint sheen
// sliding across its pixels, so the barrier reads as wrapped around the body rather than drawn
// over it. On an area or a path it is a shell hugging the edge of the body: a crisp rim with a
// fainter line inside it, a band of facets stepping brighter toward the rim, sheen lines sliding
// across and a faint halo stepping out over the margin. It fades in and out at the layer's ends,
// so a status replaying it as its `during` vfx keeps it up without a seam. Drawn on the pixel grid.
//   color     the ward; its alpha scales it
//   strength  how solid the ward looks: 1 is a full ward
fn vfx(uv: vec2<f32>) -> vec4<f32> {
    let suv = site_uv(pixel_uv(uv));
    let image = sample(suv);
    let tick = floor(site.time * 12.0) / 12.0;
    let seam = smoothstep(0.0, 0.15, site.progress) * (1.0 - smoothstep(0.85, 1.0, site.progress));
    let solid = max(params.strength, 0.0) * seam * params.color.a;
    if site.kind == SITE_ACTOR || site.kind == SITE_OBJECT {
        let px = 1.0 / max(site.size, vec2<f32>(1.0));
        var beside = 0.0;
        for (var i = 0; i < 9; i++) {
            if i == 4 {
                continue;
            }
            let offset = vec2<f32>(f32(i % 3) - 1.0, f32(i / 3) - 1.0);
            beside = max(beside, sample(suv + offset * px).a);
        }
        let outline = step(0.5, beside) * (1.0 - step(0.5, image.a));
        let around = atan2(suv.y - 0.5, suv.x - 0.5) / 6.2831853;
        let glint = step(0.8, fract(around - tick * 0.5));
        let sheen = step(0.93, fract((suv.x + suv.y * 0.6) * 2.0 - tick * 0.4)) * step(0.5, image.a) * 0.3;
        let wash = step(0.5, image.a) * 0.08;
        let glow = max(max(outline * (0.55 + 0.45 * glint), sheen), wash) * solid;
        let rgb = mix(params.color.rgb, vec3<f32>(1.0), glint * outline * 0.6);
        return over(vec4<f32>(rgb, glow), image);
    }
    let pixel = site.tiles.x / max(site.size.x, 1.0);
    let p = site_local(suv);
    let d = site_distance(suv);
    let rim = step(abs(d), pixel);
    let liner = step(abs(d + 3.0 * pixel), pixel) * 0.5;
    let facets = step(0.5, noise(p * 2.5 + tick * 0.3)) * floor(clamp(1.0 + d / 0.6, 0.0, 0.999) * 3.0) / 3.0 * step(d, 0.0) * 0.3;
    let sheen = step(0.9, fract((p.x + p.y) * 1.5 + tick * 0.8)) * step(d, -pixel) * 0.3;
    let halo = (1.0 - floor(clamp(d / 0.6, 0.0, 1.0) * 2.0) / 2.0) * step(0.0, d) * 0.15;
    let glow = clamp(max(max(rim, liner), max(max(facets, sheen), halo)), 0.0, 1.0) * solid;
    let rgb = mix(params.color.rgb, vec3<f32>(1.0), rim * 0.6);
    return over(vec4<f32>(rgb, clamp(glow, 0.0, 1.0)), image);
}

fn hash2(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash2(i);
    let b = hash2(i + vec2<f32>(1.0, 0.0));
    let c = hash2(i + vec2<f32>(0.0, 1.0));
    let d = hash2(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}
