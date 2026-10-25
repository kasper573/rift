// The weather as the engine tells a cover shader of it: the wind as a direction scaled by its
// strength (calm at 0, a gale at 1), the time, how far through its day the game is (0 midnight,
// 0.5 noon; noon when the game keeps no clock), how much cloud covers the sky (0 clear, 1
// overcast), and the bodies treading through the cover, each a ground point (x east, y south) and
// the radius it flattens. `wind_lean` and `tread_push` turn those into the metres a blade's tip
// shifts, and `lean` bends a vertex of a blade that far, by how high up the blade it stands.

const TREADS: u32 = 24u;

struct Weather {
    wind: vec2<f32>,
    time: f32,
    treads: u32,
    daylight: f32,
    overcast: f32,
    tread: array<vec4<f32>, TREADS>,
}

@group(3) @binding(102) var<uniform> weather: Weather;

// How far a blade's tip leans in a gale, in metres.
const GALE_LEAN: f32 = 0.45;
// How far a tread pushes a blade standing right under it, as a share of the tread's radius.
const TREAD_PUSH: f32 = 0.9;

// The wind at a ground point, gusting in waves that travel along it.
fn wind_lean(ground: vec2<f32>) -> vec2<f32> {
    let strength = length(weather.wind);
    if strength <= 0.0 {
        return vec2<f32>(0.0);
    }
    let along = weather.wind / strength;
    let across = vec2<f32>(-along.y, along.x);
    let phase = dot(ground, along) * 0.8 - weather.time * (1.2 + 1.8 * strength);
    let ripple = dot(ground, across) * 1.7 + weather.time * 0.7;
    let gust = 0.55 + 0.35 * sin(phase) + 0.1 * sin(phase * 2.6 + ripple);
    return along * GALE_LEAN * strength * gust;
}

// How far the bodies treading nearby push a blade at a ground point away from them.
fn tread_push(ground: vec2<f32>) -> vec2<f32> {
    var push = vec2<f32>(0.0);
    for (var i = 0u; i < weather.treads; i++) {
        let tread = weather.tread[i];
        let away = ground - tread.xy;
        let apart = length(away);
        if apart < tread.z && apart > 1e-4 {
            push += away / apart * (1.0 - apart / tread.z) * tread.z * TREAD_PUSH;
        }
    }
    return push;
}

// A vertex `tip` of the way up its blade (0 at the root, 1 at the tip), bent so the tip shifts
// `shift` metres over the ground and the blade dips as it leans.
fn lean(world: vec3<f32>, tip: f32, shift: vec2<f32>) -> vec3<f32> {
    let weight = tip * tip;
    let moved = shift * weight;
    return vec3<f32>(world.x + moved.x, world.y - length(moved) * 0.6, world.z + moved.y);
}
