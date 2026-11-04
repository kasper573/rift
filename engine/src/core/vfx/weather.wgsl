// The weather as the engine tells a shader of it: the wind as a direction scaled by its strength
// (calm at 0, a gale at 1), the time, how far through its day the game is (0 midnight, 0.5 noon;
// noon when the game keeps no clock), how much cloud covers the sky (0 none, 1 clouded over), how
// much water it holds (0 fair-weather cloud, 1 storm cloud), how hard it rains under the cloud
// that holds the most (0 none, 1 a cloudburst), how brightly lightning lights the world this
// moment (0 none, 1 a strike close by) and how many bodies tread through the cover. `wind_lean`
// turns the wind into the metres a blade's tip shifts. Round the camera's subject the engine
// tells of the rain over each stretch of ground (`rain_field`), how hard it rains at a point and
// how wet the ground lies there (`raining`, `wetness`), and where roofs and canopies shelter it
// (`sheltered`). The engine paints the weather into a picture a texel per value, read in order
// row by row (`weather_texel`), and `read_weather` reads it out as each entry of the header starts.

struct Weather {
    wind: vec2<f32>,
    time: f32,
    treads: u32,
    daylight: f32,
    clouds: f32,
    water: f32,
    rain: f32,
    flash: f32,
}

@group(3) @binding(102) var weather_picture: texture_2d<f32>;

var<private> weather: Weather;
// Where the first texel of the rain field, the shelter and the treads lie in the picture, in x, y
// and z.
var<private> weather_parts: vec3<u32>;
// Where the rain field and the shelter lie on the ground: their north-west corner in xy, a
// cell's side in metres in z and the cells each way in w, none where there is no field.
var<private> rain_cells: vec4<f32>;
var<private> shelter_cells: vec4<f32>;
// The wettest ground and the hardest rain anywhere over the rain field, in x and y.
var<private> rain_most: vec2<f32>;

fn weather_texel(index: u32) -> vec4<f32> {
    let width = textureDimensions(weather_picture).x;
    return textureLoad(weather_picture, vec2<i32>(i32(index % width), i32(index / width)), 0);
}

fn read_weather() {
    let blowing = weather_texel(0u);
    let sky = weather_texel(1u);
    let storm = weather_texel(2u);
    weather.wind = blowing.xy;
    weather.time = blowing.z;
    weather.treads = u32(blowing.w);
    weather.daylight = sky.x;
    weather.clouds = sky.y;
    weather.water = sky.z;
    weather.rain = sky.w;
    weather.flash = storm.x;
    weather_parts = vec3<u32>(u32(storm.y), u32(storm.z), u32(storm.w));
    rain_cells = weather_texel(3u);
    shelter_cells = weather_texel(4u);
    rain_most = weather_texel(5u).xy;
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

// The rain over a ground point (x east, y south): the share of the downpour that falls there by
// the lie of the land in x, how wet the ground lies in y and how hard it rains in z, each 0 to 1,
// read between the field's cells and held at its edge's beyond it. Where there is no field, the
// whole downpour would fall, and the ground lies dry under no rain.
fn rain_field(at: vec2<f32>) -> vec3<f32> {
    if rain_cells.w < 1.0 {
        return vec3<f32>(1.0, 0.0, 0.0);
    }
    let cell = (at - rain_cells.xy) / rain_cells.z - 0.5;
    let corner = floor(cell);
    let t = cell - corner;
    let upper = mix(rain_cell(corner), rain_cell(corner + vec2<f32>(1.0, 0.0)), t.x);
    let lower = mix(rain_cell(corner + vec2<f32>(0.0, 1.0)), rain_cell(corner + vec2<f32>(1.0, 1.0)), t.x);
    return mix(upper, lower, t.y);
}

// How far a point lies under shelter from the rain: 1 under a roof or a canopy, 0 under the open
// sky, between the two at a shelter's edge.
fn sheltered(point: vec3<f32>) -> f32 {
    if shelter_cells.w < 1.0 {
        return 0.0;
    }
    let cell = (point.xz - shelter_cells.xy) / shelter_cells.z - 0.5;
    let corner = floor(cell);
    let t = cell - corner;
    let upper = mix(covered(corner, point.y), covered(corner + vec2<f32>(1.0, 0.0), point.y), t.x);
    let lower = mix(covered(corner + vec2<f32>(0.0, 1.0), point.y), covered(corner + vec2<f32>(1.0, 1.0), point.y), t.x);
    return mix(upper, lower, t.y);
}

// How hard the rain falls on a point, 0 none to 1 a cloudburst: none under shelter.
fn raining(point: vec3<f32>) -> f32 {
    if rain_most.y <= 0.0 {
        return 0.0;
    }
    let rain = rain_field(point.xz).z;
    if rain <= 0.0 {
        return 0.0;
    }
    return rain * (1.0 - sheltered(point));
}

// How wet a surface at a point lies, 0 dry to 1 soaked: dry under shelter.
fn wetness(point: vec3<f32>) -> f32 {
    if rain_most.x <= 0.0 {
        return 0.0;
    }
    let wet = rain_field(point.xz).y;
    if wet <= 0.0 {
        return 0.0;
    }
    return wet * (1.0 - sheltered(point));
}

fn rain_cell(at: vec2<f32>) -> vec3<f32> {
    let side = i32(rain_cells.w);
    let cell = clamp(vec2<i32>(at), vec2<i32>(0), vec2<i32>(side - 1));
    return weather_texel(weather_parts.x + u32(cell.y * side + cell.x)).xyz;
}

// Whether a point `height` up at the shelter's cell `at` lies under the top of what stands there:
// 1 sheltered, 0 open.
fn covered(at: vec2<f32>, height: f32) -> f32 {
    let side = i32(shelter_cells.w);
    let cell = vec2<i32>(at);
    if any(cell < vec2<i32>(0)) || any(cell >= vec2<i32>(side)) {
        return 0.0;
    }
    let index = u32(cell.y * side + cell.x);
    let top = weather_texel(weather_parts.y + index / 4u)[index % 4u];
    return select(0.0, 1.0, height < top - SHELTER_DEPTH);
}
