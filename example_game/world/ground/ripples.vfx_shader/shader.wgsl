// Water lying flat at its level over the painted cells: the ground's colour, clearer and lighter
// where the bottom is near, darkening towards the deep colour, with foam breaking unevenly along
// the shore, as wide as `foam` allows, and thin threads of light wandering over the deeper water, drawn along the ridges of
// drifting noise so no two are alike. The wind drives it all: the surface swells under the gusts,
// the threads and the foam drift downwind, faster the harder it blows, and in a dead calm the
// water lies still.
fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn ripple(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(
        mix(hash(i), hash(i + vec2<f32>(1.0, 0.0)), u.x),
        mix(hash(i + vec2<f32>(0.0, 1.0)), hash(i + vec2<f32>(1.0, 1.0)), u.x),
        u.y,
    );
}

fn sway(world: vec3<f32>, tip: f32) -> vec3<f32> {
    let swell = length(wind_lean(world.xz)) * 0.08;
    return vec3<f32>(world.x, world.y + swell, world.z);
}

fn vfx(uv: vec2<f32>) -> vec4<f32> {
    let lit = sample(uv);
    let metres = uv * site.metres;
    let strength = length(weather.wind);
    var along = vec2<f32>(0.8, 0.6);
    if strength > 0.0 {
        along = weather.wind / strength;
    }
    let drift = along * weather.time * (0.15 + 0.6 * strength);
    let d = depth();
    let coarse = ripple(metres * 0.6 - drift);
    let fine = ripple(metres * 1.3 + drift * 0.6 + vec2<f32>(17.0, 31.0));
    let ridge = 1.0 - abs((0.65 * coarse + 0.35 * fine) * 2.0 - 1.0);
    let glint = smoothstep(0.94, 0.985, ridge) * smoothstep(0.4, 1.2, d) * (0.22 + 0.2 * strength);
    let shore = 1.0 - smoothstep(0.0, 0.08 + 0.5 * params.foam, d);
    let lap = ripple(metres * 0.7 - drift * 2.5);
    let foam = smoothstep(0.6, 0.75, shore + 0.3 * lap * params.foam);
    let body = mix(lit.rgb, params.deep.rgb, smoothstep(0.1, 1.6, d));
    let color = mix(body, params.crest.rgb, max(foam, glint));
    let alpha = mix(0.55, params.opacity, smoothstep(0.0, 0.6, d));
    return vec4<f32>(color, max(alpha, foam));
}
