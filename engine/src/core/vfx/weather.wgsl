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
// row by row (`weather_texel`), and `read_weather` reads it out as each entry of the header starts;
// the time is the frame's own, from the globals every view binds.

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
// How far the rain is carried over the ground for every metre it falls.
var<private> rain_falls: vec2<f32>;

fn weather_texel(index: u32) -> vec4<f32> {
    let width = textureDimensions(weather_picture).x;
    return textureLoad(weather_picture, vec2<i32>(i32(index % width), i32(index / width)), 0);
}

fn read_weather() {
    let blowing = weather_texel(0u);
    let sky = weather_texel(1u);
    let storm = weather_texel(2u);
    weather.wind = blowing.xy;
    weather.time = globals.time;
    weather.treads = u32(blowing.w);
    weather.daylight = sky.x;
    weather.clouds = sky.y;
    weather.water = sky.z;
    weather.rain = sky.w;
    weather.flash = storm.x;
    weather_parts = vec3<u32>(u32(storm.y), u32(storm.z), u32(storm.w));
    rain_cells = weather_texel(3u);
    shelter_cells = weather_texel(4u);
    let falling = weather_texel(5u);
    rain_most = falling.xy;
    rain_falls = falling.zw;
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

// How far a point lies under shelter from the rain: 1 where a roof, a canopy or anything else
// that shelters stands in the way the rain comes to it, 0 under the open sky. The rain leans with
// the wind, so its way down is followed from upwind: the top and the windward side of what
// stands in it take the rain, and what lies in its lee stays dry. Shelter eases in at a
// shelter's edge and through the first SHELTER_DEPTH under its top, where a crown's outer leaves
// still take the rain.
fn sheltered(point: vec3<f32>) -> f32 {
    if shelter_cells.w < 1.0 {
        return 0.0;
    }
    let leaning = length(rain_falls);
    var nudge = vec2<f32>(0.0);
    if leaning > 0.0 {
        nudge = rain_falls / leaning * shelter_cells.z * 0.5 * min(leaning / LEANING, 1.0);
    }
    let fall = min(shelter_cells.z / max(leaning, 1e-6), SHELTER_STEP);
    var most = 0.0;
    for (var index = 0; index < SHELTER_STEPS; index++) {
        let rise = f32(index) * fall;
        let over = shelter_over(point.xz - nudge - rain_falls * rise);
        most = max(most, over.y * smoothstep(0.0, SHELTER_DEPTH, over.x - point.y - rise));
        if leaning <= 0.0 {
            break;
        }
    }
    return most;
}

// The top of what stands over a ground point in x, as the cells round it slope to it, and how
// much of the point they stand over in y: 1 well under a shelter, 0 under the open sky.
fn shelter_over(ground: vec2<f32>) -> vec2<f32> {
    let cell = (ground - shelter_cells.xy) / shelter_cells.z - 0.5;
    let corner = floor(cell);
    let t = cell - corner;
    let tops = vec4<f32>(
        shelter_top(corner),
        shelter_top(corner + vec2<f32>(1.0, 0.0)),
        shelter_top(corner + vec2<f32>(0.0, 1.0)),
        shelter_top(corner + vec2<f32>(1.0, 1.0)),
    );
    let stands = step(vec4<f32>(OPEN_TOP * 0.5), tops)
        * vec4<f32>((1.0 - t.x) * (1.0 - t.y), t.x * (1.0 - t.y), (1.0 - t.x) * t.y, t.x * t.y);
    let under = stands.x + stands.y + stands.z + stands.w;
    if under <= 0.0 {
        return vec2<f32>(OPEN_TOP, 0.0);
    }
    return vec2<f32>(dot(stands, tops) / under, under);
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

// The height of the top of what stands over the shelter's cell `at`, or OPEN_TOP under the open
// sky.
fn shelter_top(at: vec2<f32>) -> f32 {
    let side = i32(shelter_cells.w);
    let cell = vec2<i32>(at);
    if any(cell < vec2<i32>(0)) || any(cell >= vec2<i32>(side)) {
        return OPEN_TOP;
    }
    let index = u32(cell.y * side + cell.x);
    return weather_texel(weather_parts.y + index / 4u)[index % 4u];
}
