// The sky beyond the air. By day, a sky shading from `day_horizon` up to `day_zenith`, as bright
// as `day_light` times the light the world is lit by, covering the air by `day` while the light
// stands high, so the air's own colours show as it sets. By night, a sky shading from `night_horizon` up to `night_zenith`, as
// bright as `night_light`, glowing `dusk` low toward the sun as it sets and rises by `dusk_glow`; stars as bright as `stars`, `star_count` of the sky's cells holding
// one, thickest along a band of the Milky Way glowing by `milky_way`; and the full moon, `moon_size`
// degrees across in `moon`, its glow spreading round it by `moon_glow`.
// The weather's clouds as heaps of round billows drifting at the clouds' height and rising at most
// `height` over it: big billows `billow` metres apart, each as large as the field is cloudy under
// it, smaller ones heaped on their tops by `fringe`, all melted into one another and cut flat
// underneath, dissolving into the air as they lie toward `reach` metres off. Light through a heap
// falls off with the water it crosses, `thickness` of it at the weather's driest and more as the
// cloud holds more, scatters forward round the sun or moon into a bright rim, and the sky lights
// the rest, by `sun` and `sky`; the light is laid on in flat bands by `bands`, the shade tinted
// toward `shadow`, as a painter lays it on. High brushed streaks in `streak`, as thick as
// `streaks`, run the way `streak_turn` degrees from north.
fn vfx(uv: vec2<f32>) -> vec4<f32> {
    let way = look();
    var sky = vec4<f32>(0.0);
    let dark = night();
    let painted = params.day * (1.0 - dark) * smoothstep(0.0, 0.3, light_way().y);
    if painted > 0.0 {
        let rise = sqrt(clamp(way.y, 0.0, 1.0));
        let day = mix(params.day_horizon.rgb, params.day_zenith.rgb, rise);
        let strength = luminance(light_color());
        sky = laid(sky, day * strength * params.day_light, painted);
    }
    if dark > 0.0 {
        let rise = sqrt(clamp(way.y, 0.0, 1.0));
        sky = laid(sky, mix(params.night_horizon.rgb, params.night_zenith.rgb, rise) * params.night_light, dark);
        let shown = dark * dark * dark * smoothstep(-0.02, 0.15, way.y);
        sky = vec4<f32>(sky.rgb + (starlight(way) + milky_way(way)) * shown, sky.a);
        sky = vec4<f32>(sky.rgb + twilight(way, dark), sky.a);
    }
    sky = moon(sky, way, dark);
    let streak = streaks(way) * params.streaks * (1.0 - dark);
    sky = laid(sky, params.streak.rgb * (light_color() * 0.7 + sky_light(vec3<f32>(0.0, 1.0, 0.0)) * 2.0), streak);
    let heap = trace(way);
    if heap.w > 0.0 {
        sky = laid(sky, lit(heap.xyz, way), heap.w);
    }
    if sky.a <= 0.0 {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(sky.rgb / sky.a, sky.a);
}

// The glow low over the horizon where the sun has just set or is about to rise, warmest toward
// the sun, strongest halfway into the dusk.
fn twilight(way: vec3<f32>, dark: f32) -> vec3<f32> {
    let sun = -moon_way();
    let level = max(way.y, 0.0);
    let flat_way = normalize(vec2<f32>(way.x, way.z) + vec2<f32>(1e-5));
    let flat_sun = normalize(vec2<f32>(sun.x, sun.z) + vec2<f32>(1e-5));
    let toward = dot(flat_way, flat_sun) * 0.5 + 0.5;
    let band = exp(-level / 0.12) * (0.25 + 0.75 * toward * toward);
    let strength = 4.0 * dark * (1.0 - dark);
    return params.dusk.rgb * band * strength * params.dusk_glow;
}

// A layer of `color` covering `cover` laid over what lies behind it, kept as light already
// weighed by its cover.
fn laid(behind: vec4<f32>, color: vec3<f32>, cover: f32) -> vec4<f32> {
    return vec4<f32>(color * cover + behind.rgb * (1.0 - cover), cover + behind.a * (1.0 - cover));
}

// The stars that way: in two sizes of cell over the sky, some cells hold a star at a spot of its
// own, as bright as it is, twinkling.
fn starlight(way: vec3<f32>) -> vec3<f32> {
    var light = vec3<f32>(0.0);
    let band = milky_band(way);
    for (var layer = 0u; layer < 2u; layer++) {
        let cells = 110.0 * f32(layer + 1u);
        let at = way * cells;
        let cell = floor(at);
        let draw = scatter(vec3<i32>(cell) + vec3<i32>(i32(layer) * 4099));
        let count = params.star_count * (0.6 + 1.4 * band);
        if draw.w > count {
            continue;
        }
        let spot = cell + 0.25 + 0.5 * draw.xyz;
        let gap = length(at - spot) / (0.22 + 0.1 * f32(layer));
        let bright = pow(1.0 - draw.w / max(count, 1e-4), 3.0) * 1.5 + 0.08;
        let twinkle = 0.75 + 0.25 * sin(weather.time * (2.0 + 3.0 * draw.x) + draw.y * 40.0);
        let hue = mix(vec3<f32>(1.0, 0.82, 0.68), vec3<f32>(0.75, 0.85, 1.0), draw.z);
        light += hue * bright * twinkle * exp(-gap * gap * 3.0);
    }
    return light * params.stars;
}

// How near a way lies to the Milky Way's band, tilted across the sky, 1 along its heart.
fn milky_band(way: vec3<f32>) -> f32 {
    let across = normalize(vec3<f32>(0.35, 0.45, 0.82));
    let off = dot(way, across);
    return exp(-off * off / 0.03);
}

// The Milky Way's glow that way, mottled with dust.
fn milky_way(way: vec3<f32>) -> vec3<f32> {
    let band = milky_band(way);
    if band < 0.01 {
        return vec3<f32>(0.0);
    }
    let mottle = fbm(vec2<f32>(dot(way, vec3<f32>(0.8, 0.1, -0.35)), way.y) * 9.0);
    let dust = smoothstep(0.35, 0.75, fbm(vec2<f32>(way.x + way.z, way.y - way.x) * 14.0));
    let glow = band * mottle * (1.0 - 0.6 * dust);
    return vec3<f32>(0.55, 0.62, 0.85) * glow * params.milky_way * 0.12;
}

// The full moon, always opposite the sun: a disc of `moon`, its seas darker, and a glow round it
// lighting the air near it by night.
fn moon(behind: vec4<f32>, way: vec3<f32>, dark: f32) -> vec4<f32> {
    let toward = moon_way();
    if toward.y < -0.1 {
        return behind;
    }
    let turn = dot(way, toward);
    let off = acos(clamp(turn, -1.0, 1.0));
    let radius = radians(params.moon_size) * 0.5;
    var sky = behind;
    let glow = exp(-off / (radius * 4.0)) * 0.5 + exp(-off / (radius * 18.0)) * 0.25;
    sky = vec4<f32>(sky.rgb + params.moon.rgb * glow * params.moon_glow * dark * 0.3, sky.a);
    let disc = 1.0 - smoothstep(radius * 0.94, radius, off);
    if disc <= 0.0 {
        return sky;
    }
    let side = normalize(cross(toward, vec3<f32>(0.0, 1.0, 0.0)));
    let up = cross(side, toward);
    let face = vec2<f32>(dot(way, side), dot(way, up)) / radius;
    let seas = smoothstep(0.42, 0.62, fbm(face * 1.6 + vec2<f32>(3.1, 7.7)));
    let craters = smoothstep(0.55, 0.8, fbm(face * 6.0 + vec2<f32>(11.3, 2.9)));
    let rim = sqrt(max(1.0 - dot(face, face), 0.0));
    let surface = params.moon.rgb * (1.0 - 0.38 * seas - 0.12 * craters) * mix(0.75, 1.0, rim) * 0.75;
    return laid(sky, surface, disc * smoothstep(-0.1, 0.02, toward.y + way.y - turn * toward.y));
}

// Four numbers drawn from a cell of the sky, each in 0..1.
fn scatter(cell: vec3<i32>) -> vec4<f32> {
    var v = vec3<u32>(cell) * 1664525u + 1013904223u;
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v = v ^ (v >> vec3<u32>(16u));
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    let w = (v.x ^ v.y ^ v.z) * 2654435761u;
    return vec4<f32>(vec4<u32>(v, w) >> vec4<u32>(8u)) / 16777216.0;
}

// How far a point of the world lies outside the clouds, in metres, negative within.
fn heap(point: vec3<f32>) -> f32 {
    let big = smooth_min(
        billows(point, params.billow, 0.0, 0.0),
        billows(point, params.billow * 0.7, 0.0, 19.0),
        params.billow * 0.3
    );
    let near_big = big - params.billow * 0.22;
    let resting = -smooth_min(-billows(point, params.billow * 0.45, 1.0, 37.0), -near_big, params.billow * 0.2);
    let melted = smooth_min(big, resting, params.billow * 0.2);
    let carried = point.xz - cloud_drift();
    let lumps = fbm(vec2<f32>(carried.x + point.y * 0.7, carried.y - point.y * 0.4) / (params.billow * 0.22)) - 0.5;
    let base = cloud_base() + (fbm(carried / (params.billow * 0.6)) - 0.5) * params.billow * 0.12;
    return -smooth_min(-(melted - lumps * params.billow * 0.14), point.y - base, params.billow * 0.12);
}

// The nearest of the round billows on a lattice of cells `size` across drifting with the field,
// melted together: each sits in its own cell at its own place, growing as the field turns cloudy
// under it and slowly swelling and shrinking as clouds do. `riding` 0 sits a billow low on the
// clouds' base, 1 lifts it onto the top of the big billows there; `salt` sets the lattice apart
// from another's.
fn billows(point: vec3<f32>, size: f32, riding: f32, salt: f32) -> f32 {
    let drift = cloud_drift();
    let at = (point.xz - drift) / size;
    let cell = floor(at);
    var nearest = 1e6;
    for (var dy = -1; dy <= 1; dy++) {
        for (var dx = -1; dx <= 1; dx++) {
            let near = cell + vec2<f32>(f32(dx), f32(dy)) + vec2<f32>(salt);
            let spot = (near - vec2<f32>(salt) + 0.3 + 0.4 * vec2<f32>(hash2(near), hash2(near + vec2<f32>(7.1, 3.9)))) * size + drift;
            let cloudy = cloud_at(vec3<f32>(spot.x, cloud_base(), spot.y)).x;
            let growing = smoothstep(0.12 + 0.2 * riding, 0.32 + 0.2 * riding, cloudy);
            if growing <= 0.0 {
                continue;
            }
            let breathing = 1.0 + 0.12 * sin(weather.time * 0.05 + hash2(near + vec2<f32>(5.7, 1.3)) * 6.2832);
            let grown = (0.35 + 0.35 * cloudy) * (0.85 + 0.3 * hash2(near + vec2<f32>(2.3, 9.4))) * growing * breathing;
            let radius = size * grown * mix(1.0, params.fringe, riding);
            if radius < 0.5 {
                continue;
            }
            let lift = cloud_base() + mix(radius * 0.5, params.billow * (0.2 + 0.45 * cloudy) * params.height / 45.0, riding);
            let gap = distance(point, vec3<f32>(spot.x, lift, spot.y)) - radius;
            nearest = smooth_min(nearest, gap, size * 0.25);
        }
    }
    return nearest;
}

// Where a ray from the eye the given way first meets the clouds, and how much of the pixel they
// cover there, fading with the distance into the air; no cover where it meets none.
fn trace(way: vec3<f32>) -> vec4<f32> {
    if way.y <= 0.004 {
        return vec4<f32>(0.0);
    }
    let origin = eye();
    let enter = (cloud_base() - params.billow * 0.1 - origin.y) / way.y;
    let far = params.reach * 1.6;
    if enter > far {
        return vec4<f32>(0.0);
    }
    let leave = min((cloud_base() + params.height + params.billow - origin.y) / way.y, far);
    var t = enter;
    var before = enter;
    for (var step = 0; step < 72; step++) {
        let point = origin + way * t;
        let gap = heap(point);
        if gap < 0.002 * t {
            t = settle(origin, way, before, t);
            let fade = exp(-pow(t / params.reach, 2.0));
            return vec4<f32>(origin + way * t, fade);
        }
        before = t;
        t += max(gap * 0.85, 0.01 * t);
        if t > leave {
            break;
        }
    }
    return vec4<f32>(0.0);
}

// Where between `outside` and `inside` along the ray its face lies, halving the stretch it could
// lie on, so a stride that overshot the face doesn't leave it stepped.
fn settle(origin: vec3<f32>, way: vec3<f32>, outside: f32, inside: f32) -> f32 {
    var near = outside;
    var far = inside;
    for (var i = 0; i < 6; i++) {
        let middle = (near + far) * 0.5;
        if heap(origin + way * middle) < 0.0 {
            far = middle;
        } else {
            near = middle;
        }
    }
    return far;
}

// The light a point on a heap's face sends the eye: the sun through the water between it and the
// sun, on the face as it turns to the sun and scattered forward round it, and the sky's light on
// the face from where it faces, banded as a painter lays light on.
fn lit(point: vec3<f32>, way: vec3<f32>) -> vec3<f32> {
    let face = facing(point);
    var water = 0.0;
    for (var i = 1; i <= 5; i++) {
        let along = f32(i) * params.billow * 0.25;
        water += max(-heap(point + face * 0.5 + light_way() * along), 0.0);
    }
    let wet = 1.0 + 5.0 * weather.water;
    let through = exp(-water * params.thickness * wet * 0.2);
    let turn = dot(way, light_way());
    let toward = dot(face, light_way()) * 0.5 + 0.5;
    let rim = min(henyey_greenstein(turn, 0.6), 4.0) * 0.25 * (1.0 - toward) * through;
    let sunlit = light_color() * (toward * through + rim) * params.sun;
    let skylit = sky_light(face) * params.sky * 2.2;
    let albedo = mix(1.0, 0.35, weather.water);
    let level = luminance((sunlit + skylit) * albedo);
    let share = luminance(sunlit) / max(luminance(sunlit + skylit), 1e-4);
    let banded = mix(share, step_soft(share), params.bands);
    let shade = params.shadow.rgb * luminance(skylit * albedo) * 1.1;
    let tint = mix(vec3<f32>(1.0), light_color() / max(luminance(light_color()), 1e-4), 0.5);
    return mix(shade, vec3<f32>(level) * tint, banded);
}

// The way a heap's face turns at a point.
fn facing(point: vec3<f32>) -> vec3<f32> {
    let e = 0.6;
    let a = vec3<f32>(1.0, -1.0, -1.0);
    let b = vec3<f32>(-1.0, -1.0, 1.0);
    let c = vec3<f32>(-1.0, 1.0, -1.0);
    let d = vec3<f32>(1.0, 1.0, 1.0);
    return normalize(
        a * heap(point + a * e) + b * heap(point + b * e) + c * heap(point + c * e) + d * heap(point + d * e)
    );
}

fn smooth_min(a: f32, b: f32, k: f32) -> f32 {
    let h = clamp(0.5 + 0.5 * (b - a) / k, 0.0, 1.0);
    return mix(b, a, h) - k * h * (1.0 - h);
}

// Long thin strokes of high cloud on a plane far overhead, so they run together toward a point on
// the horizon as streaks across a sky do.
fn streaks(way: vec3<f32>) -> f32 {
    if way.y <= 0.02 || params.streaks <= 0.0 {
        return 0.0;
    }
    let turn = radians(params.streak_turn);
    let along = vec2<f32>(sin(turn), -cos(turn));
    let across = vec2<f32>(-along.y, along.x);
    let plane = way.xz / way.y * 3.0;
    let a = dot(plane, along);
    let c = dot(plane, across);
    let drift = weather.time * 0.004;
    let bend = c + 0.6 * sin(a * 0.25);
    let lane = fbm(vec2<f32>(a * 0.05 + drift, bend * 1.1));
    let grain = fbm(vec2<f32>(a * 0.3, bend * 7.0));
    let stroke = smoothstep(0.5, 0.72, lane) * smoothstep(0.3, 0.7, grain);
    let patchy = smoothstep(0.3, 0.55, fbm(plane * 0.1 + vec2<f32>(7.0, 3.0)));
    return stroke * patchy * smoothstep(0.03, 0.25, way.y) * (1.0 - smoothstep(0.75, 0.98, way.y));
}

fn henyey_greenstein(turn: f32, g: f32) -> f32 {
    let g2 = g * g;
    return (1.0 - g2) / pow(1.0 + g2 - 2.0 * g * turn, 1.5);
}

fn luminance(color: vec3<f32>) -> f32 {
    return dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// A share of light laid on in flat bands with soft edges, as a painter's shade, light and
// highlight.
fn step_soft(level: f32) -> f32 {
    return 0.15 + smoothstep(0.3, 0.38, level) * 0.6 + smoothstep(0.8, 0.88, level) * 0.25;
}

fn smooth_noise(at: vec2<f32>) -> f32 {
    let cell = floor(at);
    let f = fract(at);
    let s = f * f * (3.0 - 2.0 * f);
    let a = hash2(cell);
    let b = hash2(cell + vec2<f32>(1.0, 0.0));
    let c = hash2(cell + vec2<f32>(0.0, 1.0));
    let d = hash2(cell + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, s.x), mix(c, d, s.x), s.y);
}

fn fbm(at: vec2<f32>) -> f32 {
    var sum = 0.0;
    var weight = 0.5;
    var p = at;
    for (var octave = 0; octave < 4; octave++) {
        sum += weight * smooth_noise(p);
        p = p * 2.03 + vec2<f32>(17.1, 9.7);
        weight *= 0.5;
    }
    return sum / 0.9375;
}
