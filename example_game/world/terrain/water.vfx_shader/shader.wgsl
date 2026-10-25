// Water lying flat at its level over the painted cells: bright and clear, its surface a net of
// drifting cells where the light gathers, each cell a shade deeper in the middle and seamed in
// pale light, with bands of foam breaking along the shore in dashes and rings spreading now and
// then where a drop fell. The wind drives it: the cells and the foam drift downwind, faster the
// harder it blows, and the surface swells under the gusts.
fn cell_noise(p: vec2<f32>) -> vec2<f32> {
    let i = floor(p);
    let f = fract(p);
    var nearest = 8.0;
    var second = 8.0;
    for (var y = -1; y <= 1; y++) {
        for (var x = -1; x <= 1; x++) {
            let cell = i + vec2<f32>(f32(x), f32(y));
            let jitter = vec2<f32>(hash2(cell), hash2(cell + vec2<f32>(7.0, 3.0)));
            let d = length(cell + 0.5 + (jitter - 0.5) * 0.9 - p);
            if d < nearest {
                second = nearest;
                nearest = d;
            } else if d < second {
                second = d;
            }
        }
    }
    return vec2<f32>(nearest, second - nearest);
}

fn ring(metres: vec2<f32>, seed: f32, tick: f32) -> f32 {
    let life = fract(tick * 0.18 + hash1(seed));
    let at = vec2<f32>(hash1(seed + 1.0), hash1(seed + 2.0)) * site.metres;
    let r = length(metres - at);
    let radius = life * 1.6;
    let band = smoothstep(0.1, 0.0, abs(r - radius)) * (1.0 - life) * step(0.05, life);
    return band;
}

fn sway(world: vec3<f32>, tip: f32) -> vec3<f32> {
    let swell = length(wind_lean(world.xz)) * 0.06;
    return vec3<f32>(world.x, world.y + swell, world.z);
}

fn vfx(uv: vec2<f32>) -> vec4<f32> {
    let light = light();
    let metres = uv * site.metres;
    let strength = length(weather.wind);
    var along = vec2<f32>(0.8, 0.6);
    if strength > 0.0 {
        along = weather.wind / strength;
    }
    let drift = along * weather.time * (0.1 + 0.4 * strength);
    let d = depth();
    let cells = cell_noise(metres * 0.6 - drift);
    let seam = smoothstep(0.0, 0.12, cells.y);
    let hollow = smoothstep(0.0, 0.9, cells.x);
    let body = mix(params.shallow.rgb, params.deep.rgb, smoothstep(0.0, 1.4, d)) * light;
    let crest = params.crest.rgb * light;
    var color = mix(body * (1.0 - 0.1 * hollow), crest, (1.0 - seam) * 0.35);
    let shore = 1.0 - smoothstep(0.0, 0.05 + 0.5 * params.foam, d);
    let lap = cell_noise(metres * 2.2 + drift * 1.5 + vec2<f32>(11.0, 5.0)).y;
    let dashes = step(0.55, lap) * smoothstep(0.1, 0.6, shore);
    let foam = max(smoothstep(0.75, 0.95, shore), dashes * 0.9);
    var rings = 0.0;
    for (var i = 0; i < 5; i++) {
        rings = max(rings, ring(metres, f32(i) * 13.7 + 1.0, weather.time));
    }
    color = mix(color, crest, max(foam, rings * 0.8));
    let alpha = mix(0.7, params.opacity, smoothstep(0.0, 0.5, d));
    return vec4<f32>(color, max(alpha, foam));
}
