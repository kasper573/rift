// A fireball landing on its site: a blast front races from the site's point to the rim of its
// body, flames roar up behind it in ragged tongues and lick just past the edge, then die down to
// embers and smoke. The flames fill whatever the body is: a circle, a cone, a line, the length of
// a path, or the sprite it engulfs; embers and smoke lift off the body and drift out over the
// margin, where they dissolve. Drawn on the pixel grid in banded color, animated at twelve frames
// a second, so it sits with the sprites rather than over them.
//   heat   how fiercely the flames burn: 1 is a full fireball, less a smoulder
//   smoke  the color of the smoke left behind
fn vfx(uv: vec2<f32>) -> vec4<f32> {
    let suv = site_uv(pixel_uv(uv));
    let image = sample(suv);
    let p = site_local(suv);
    let reach = site_reach(suv);
    let on_sprite = site.kind == SITE_ACTOR || site.kind == SITE_OBJECT;
    let rim = select(site_distance(suv), 0.4 - burning(suv), on_sprite);
    let heat = max(params.heat, 0.0);
    let t = site.progress;
    let tick = floor(site.time * 12.0) / 12.0;
    let front = smoothstep(0.0, 0.25, t) * 1.08;
    let q = suv * site.tiles;
    let rising = fbm(q * 1.7 + vec2<f32>(0.0, tick * 2.6));
    let flicker = fbm(q * 3.4 + vec2<f32>(tick * 0.5, tick * 5.0));
    let lick = (rising - 0.5) * 0.4;
    let lit = step(reach - 0.1, front);
    let edge = step(rim - lick, 0.0);
    let dying = 1.0 - smoothstep(0.5, 0.9, t);
    let starve = 0.3 + 0.45 * smoothstep(0.3, 0.85, t) + 0.2 * clamp(reach, 0.0, 1.0);
    let tongue = step(starve, rising * 0.75 + flicker * 0.45);
    let flame = lit * edge * tongue * heat * dying;
    let ring = step(abs(reach - front), 0.14) * (1.0 - smoothstep(0.15, 0.4, t)) * edge;
    let core = (1.0 - smoothstep(0.0, 0.6, reach)) * lit * edge * (1.0 - smoothstep(0.0, 0.5, t));
    let fire = max(max(flame, ring), core);
    let glow = (rising * 0.75 + flicker * 0.45 - starve) * 2.2 + (1.0 - clamp(reach, 0.0, 1.0)) * 0.5;
    let hot = (bands(clamp(glow, 0.0, 1.0), 4.0) * 0.9 + 0.1) * flame + ring * 1.3 + core * 1.3;
    let color = ramp(hot);
    let sparks = embers(suv, t, tick) * heat * (1.0 - smoothstep(0.7, 1.0, t));
    let puff = smoke(suv, p, t, tick);
    let fire_alpha = max(fire * select(1.0, 0.85, on_sprite), sparks);
    let rgb = mix(params.smoke.rgb, mix(color, vec3<f32>(1.0, 0.85, 0.5), sparks), clamp(fire_alpha * 1.6, 0.0, 1.0));
    let blaze = vec4<f32>(rgb, max(fire_alpha, puff * params.smoke.a));
    return over(blaze, image);
}

// How much of a sprite burns at a site uv: its own pixels and the air just above them, so the
// flames hug the body and rise off it rather than fill the sprite's frame.
fn burning(suv: vec2<f32>) -> f32 {
    var body = 0.0;
    for (var k = 0u; k < 4u; k++) {
        let below = suv + vec2<f32>(0.0, f32(k) * 0.2 / max(site.tiles.y, 0.01));
        body = max(body, sample(below).a * (1.0 - f32(k) * 0.2));
    }
    return body;
}

// Pixel-sized embers born across the body that lift off it and wink out on the margin.
fn embers(suv: vec2<f32>, t: f32, tick: f32) -> f32 {
    let pixel = 1.0 / max(site.size * site.overscan, vec2<f32>(1.0));
    var light = 0.0;
    for (var i = 0u; i < 18u; i++) {
        let seed = f32(i) * 7.31;
        let birth = vec2<f32>(hash(seed + 5.3), hash(seed + 13.7));
        let life = fract(tick * (0.35 + 0.3 * hash(seed + 1.7)) + hash(seed + 3.1));
        let lift = vec2<f32>((hash(seed + 9.2) - 0.5) * 0.6, -life * 1.8) / max(site.tiles, vec2<f32>(0.01));
        let at = birth + lift;
        let d = abs(suv - at) / pixel;
        let square = step(max(d.x, d.y), 1.0);
        let glowing = step(0.0, 0.3 - life) + step(life, 0.75) * step(0.5, hash(seed + floor(tick * 12.0)));
        light = max(light, square * site_cover(birth) * glowing * step(0.2, t));
    }
    return light;
}

// Smoke rising off the burnt body in soft banded puffs: each puff is born where the flames were
// and drifts up and out over the margin as it thins.
fn smoke(suv: vec2<f32>, p: vec2<f32>, t: f32, tick: f32) -> f32 {
    let risen = smoothstep(0.3, 1.0, t) * 1.4;
    let wander = (fbm(suv * 3.0 + tick * 0.2) - 0.5) * 0.3;
    let source = suv + vec2<f32>(wander, risen) / max(site.tiles, vec2<f32>(0.01));
    let puffs = fbm(p * 1.6 + vec2<f32>(0.3, -tick * 1.1 + risen));
    let thick = smoothstep(0.35, 0.75, t) * (1.0 - smoothstep(0.75, 1.0, t));
    let body = smoothstep(0.55, 0.7, puffs) * burnt(source) * thick;
    return bands(body, 2.0) * 0.3;
}

// Whether a site uv lies on ground the flames covered: a sprite's burning pixels, else the body
// and the lick past its edge.
fn burnt(suv: vec2<f32>) -> f32 {
    if site.kind == SITE_ACTOR || site.kind == SITE_OBJECT {
        return step(0.3, burning(suv));
    }
    return step(site_distance(suv), 0.2);
}

// Deep red through orange to yellow as a flame gets hotter, in the four flat tones of pixel fire;
// white only at the blast itself.
fn ramp(heat: f32) -> vec3<f32> {
    let ember = vec3<f32>(0.5, 0.04, 0.0);
    let red = vec3<f32>(0.95, 0.18, 0.02);
    let orange = vec3<f32>(1.0, 0.45, 0.05);
    let yellow = vec3<f32>(1.0, 0.82, 0.25);
    let white = vec3<f32>(1.0, 0.97, 0.85);
    if heat > 1.2 {
        return white;
    } else if heat > 0.75 {
        return yellow;
    } else if heat > 0.5 {
        return orange;
    } else if heat > 0.25 {
        return red;
    }
    return ember;
}

fn hash(x: f32) -> f32 {
    return fract(sin(x * 12.9898) * 43758.5453);
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

fn fbm(p: vec2<f32>) -> f32 {
    var value = 0.0;
    var amplitude = 0.55;
    var at = p;
    for (var i = 0; i < 4; i++) {
        value += amplitude * noise(at);
        at = at * 2.1 + vec2<f32>(1.7, 9.2);
        amplitude *= 0.5;
    }
    return value;
}
