// The weather as the engine tells a shader of it: the wind as a direction scaled by its strength
// (calm at 0, a gale at 1), the time, how far through its day the game is (0 midnight, 0.5 noon;
// noon when the game keeps no clock), how much cloud covers the sky (0 none, 1 clouded over), how
// much water it holds (0 fair-weather cloud, 1 storm cloud) and how many bodies tread through the
// cover. `wind_lean` turns the wind into the metres a blade's tip shifts. The engine paints the
// weather into a picture a texel per value, and `read_weather` reads it out as each entry of the
// header starts.

struct Weather {
    wind: vec2<f32>,
    time: f32,
    treads: u32,
    daylight: f32,
    clouds: f32,
    water: f32,
}

@group(3) @binding(102) var weather_picture: texture_2d<f32>;

var<private> weather: Weather;

fn read_weather() {
    let blowing = textureLoad(weather_picture, vec2<i32>(0, 0), 0);
    let sky = textureLoad(weather_picture, vec2<i32>(1, 0), 0);
    weather.wind = blowing.xy;
    weather.time = blowing.z;
    weather.treads = u32(blowing.w);
    weather.daylight = sky.x;
    weather.clouds = sky.y;
    weather.water = sky.z;
}

// How far a blade's tip leans in a gale, in metres.
const GALE_LEAN: f32 = 0.45;

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
