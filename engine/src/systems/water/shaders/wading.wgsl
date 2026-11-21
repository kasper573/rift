// What breaks a water's surface, and what the water shows of it: the flesh of the bodies treading
// nearby where the surface cuts through it (`waterline`); the wakes each of them trails as it
// moves, one from every place it breaks the surface (`wakes`, `wake`); and the rings that spread
// from where the surface is broken (`rings`), whether by a body at rest, by a thing that keeps
// the water round it stirred, as a shoal of fish does, or by rain. A body's flesh is its limbs,
// each a bone with the skin round it, so what the water shows is the shape of what stands or
// floats in it: a wake from each leg in the shallows, one from the hips of whoever wades deep,
// and from a hull one at its bow and more along it to its stern. What breaks the water says
// which rings it sends, so every ring is drawn alike, whatever sent it.

// The flesh nearest a ground point where a surface cuts through it: how far outside it the point
// lies in metres, negative within it, and how fast the body it belongs to travels, in metres a
// second.
struct Waterline {
    away: f32,
    speed: f32,
}

// A wake at a ground point: how far outside the wake's edge the point lies in metres, negative
// within it; how far outside the water the flesh that leaves it passed through (`wash`), negative
// within that; how far back along its way the flesh passed the point (`astern`) and how much
// farther back the wake still runs (`beyond`), in metres; how long ago it passed, in seconds
// (`age`), and how fast (`speed`, metres a second); how deep its body reaches under the surface
// (`draught`); and how steadily it has broken the surface of late, from 0 to 1 (`steady`): a foot
// that leaves the water at every step breaks it less steadily than a hull. Where no wake lies
// near the point, `edge` and `wash` are as far as was asked for and the rest nothing.
struct Wake {
    edge: f32,
    wash: f32,
    astern: f32,
    beyond: f32,
    age: f32,
    speed: f32,
    draught: f32,
    steady: f32,
}

// The flesh nearest `at`, a ground point, of the bodies treading nearby where a surface at
// `level` cuts through it; `far` away where none lies within `far`.
fn waterline(at: vec2<f32>, level: f32, far: f32) -> Waterline {
    var nearest = Waterline(far, 0.0);
    let point = vec3<f32>(at.x, level, at.y);
    for (var tread = 0u; tread < weather.treads; tread++) {
        let body = weather_texel(weather_parts.z + tread);
        if distance(at, body.xy) > body.z + far {
            continue;
        }
        let first = weather_parts.z + TREADS + tread * LIMBS_PER_TREAD * 2u;
        var away = nearest.away;
        for (var limb = 0u; limb < u32(body.w); limb++) {
            let head = weather_texel(first + limb * 2u);
            let end = weather_texel(first + limb * 2u + 1u);
            let bone = end.xyz - head.xyz;
            let long = dot(bone, bone);
            var along = 0.0;
            if long > 1e-8 {
                along = clamp(dot(point - head.xyz, bone) / long, 0.0, 1.0);
            }
            away = min(away, distance(point, head.xyz + bone * along) - mix(head.w, end.w, along));
        }
        if away < nearest.away {
            nearest = Waterline(away, wetted_lead(tread).z);
        }
    }
    return nearest;
}

// How many wakes the `tread`th body treading nearby trails that may come within `far` of `at`, a
// ground point: one from each place it breaks the water, and none where the point lies beyond
// them all.
fn wakes(tread: u32, at: vec2<f32>, far: f32) -> u32 {
    if tread >= weather.treads {
        return 0u;
    }
    let lead = wetted_lead(tread);
    if distance(at, weather_texel(weather_parts.z + tread).xy) > lead.x + far {
        return 0u;
    }
    return u32(lead.y);
}

// The `place`th wake the `tread`th body treading nearby trails, at `at`, a ground point: the
// water one place of its flesh parted, spreading from where that place breaks the surface back
// along the way the body came. Nothing within `far` of the point leaves `edge` at `far`.
fn wake(tread: u32, place: u32, at: vec2<f32>, far: f32) -> Wake {
    var found = Wake(far, far, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    let body = weather_texel(weather_parts.z + tread);
    let lead = wetted_lead(tread);
    let trail = wetted_first() - TREADS * TRAIL_POINTS + tread * TRAIL_POINTS;
    let broken = weather_texel(wetted_first() + TREADS + tread * WETTED_PER_TREAD + place);
    var ahead = body.xy;
    var passed = weather.time;
    var run = 0.0;
    var off = vec2<f32>(0.0);
    var turned = vec2<f32>(0.0);
    for (var point = 0u; point < TRAIL_POINTS; point++) {
        let stood = weather_texel(trail + point);
        if stood.w < 0.5 {
            break;
        }
        let stretch = ahead - stood.xy;
        let long = length(stretch);
        if long < 1e-3 {
            passed = stood.z;
            continue;
        }
        let way = stretch / long;
        if dot(turned, turned) == 0.0 {
            off = vec2<f32>(dot(broken.xy, way), dot(broken.xy, vec2<f32>(-way.y, way.x)));
            turned = way;
        }
        var next = way;
        if point + 1u < TRAIL_POINTS {
            let before = weather_texel(trail + point + 1u);
            let earlier = stood.xy - before.xy;
            if before.w > 0.5 && dot(earlier, earlier) > 1e-6 {
                next = normalize(way + normalize(earlier));
            }
        }
        let fore = ahead + turned * off.x + vec2<f32>(-turned.y, turned.x) * off.y;
        let aft = stood.xy + next * off.x + vec2<f32>(-next.y, next.x) * off.y;
        let wide = vec2<f32>(broken.z + run * WAKE_SPREAD, broken.z + (run + long) * WAKE_SPREAD);
        let lain = spreading(at, fore, aft, wide);
        if lain.x < found.edge {
            found.edge = lain.x;
            found.astern = run + long * lain.y;
            found.age = weather.time - mix(passed, stood.z, lain.y);
            found.speed = long / max(passed - stood.z, 1e-3);
        }
        found.wash = min(found.wash, lain.z - broken.z);
        run += long;
        ahead = stood.xy;
        passed = stood.z;
        turned = next;
    }
    found.beyond = run - found.astern;
    found.draught = lead.w;
    found.steady = broken.w;
    return found;
}

// Where the first texel of what leads the treads' wakes lies: after their trails', and before
// the places each breaks the water.
fn wetted_first() -> u32 {
    return weather_parts.z + TREADS + TREADS * LIMBS_PER_TREAD * 2u + TREADS * TRAIL_POINTS;
}

// What leads the `tread`th body's wake: how far from the body the wake reaches at most in x, how
// many places the body breaks the water in y, how fast it travels in z and how deep it reaches
// under the surface in w.
fn wetted_lead(tread: u32) -> vec4<f32> {
    return weather_texel(wetted_first() + tread);
}

// How far `at` lies outside a stretch of wake, negative within it, in x; how far along the
// stretch it lies nearest, 0 at `fore` and 1 at `aft`, in y; and how far from the line the
// stretch runs along, in z. The stretch runs from `fore` to `aft` and reaches `wide.x` either
// side of its start and `wide.y` of its end, round at both.
fn spreading(at: vec2<f32>, fore: vec2<f32>, aft: vec2<f32>, wide: vec2<f32>) -> vec3<f32> {
    let stretch = aft - fore;
    let long = dot(stretch, stretch);
    let off = at - fore;
    let along = dot(off, stretch) / long;
    let across = abs(off.x * stretch.y - off.y * stretch.x) / long;
    let nearest = clamp(along, 0.0, 1.0);
    let beyond = along - nearest;
    let apart = sqrt(long * (across * across + beyond * beyond));
    let widening = wide.x - wide.y;
    let slope = vec2<f32>(sqrt(max(long - widening * widening, 0.0)), widening);
    let side = slope.x * along - slope.y * across;
    if side < 0.0 {
        return vec3<f32>(sqrt(long * (across * across + along * along)) - wide.x, 0.0, apart);
    }
    if side > slope.x {
        let past = along - 1.0;
        return vec3<f32>(sqrt(long * (across * across + past * past)) - wide.y, 1.0, apart);
    }
    return vec3<f32>(dot(slope, vec2<f32>(across, along)) - wide.x, nearest, apart);
}


// A ring spreading over the water at a ground point: how far the point lies from the ring's
// line, in metres (`off`); how far through its life the ring is, 0 as it starts to 1 as it is
// spent (`age`); and how strongly what sent it has it drawn, 0 to 1 (`strength`). Where no ring
// passes near, its strength is 0.
struct Ring {
    off: f32,
    age: f32,
    strength: f32,
}

// The ring passing nearest `at`, a point of a surface, of all that spread there: from the flesh
// `nearest` to it, while the body it belongs to rests; from the things nearby that keep the
// water round them stirred; and from where the rain strikes. The flesh is to be sought as far
// off as `rest_reach` tells, or the rings of a body at rest stop short.
fn rings(at: vec3<f32>, nearest: Waterline) -> Ring {
    let first = ripples_first();
    let lead = weather_texel(first);
    var ring = Ring(1e9, 1.0, 0.0);
    let resting = lead.y * (1.0 - smoothstep(0.25, 1.0, nearest.speed));
    let outline = weather_texel(first + 1u);
    if resting > 0.0 && nearest.away < outline.x {
        let away = vec2<f32>(max(nearest.away, 0.0), 0.0);
        ring = nearer(ring, ring_from(away, vec2<f32>(0.0), vec4<f32>(outline.xyz, 0.0), resting, 0.0));
    }
    for (var spot = 0u; spot < u32(lead.x); spot++) {
        let stands = weather_texel(first + 3u + spot * 2u);
        let sends = weather_texel(first + 4u + spot * 2u);
        if abs(stands.y - at.y) > SPOT_LEVEL || distance(at.xz, stands.xz) > sends.x + sends.w {
            continue;
        }
        ring = nearer(ring, ring_from(at.xz, stands.xz, sends, stands.w, hash2(stands.xz)));
    }
    let falling = raining(at);
    if lead.z > 0.0 && falling > 0.0 {
        let sends = weather_texel(first + 2u);
        let struck = floor(at.xz / sends.w);
        for (var east = -1; east <= 1; east++) {
            for (var south = -1; south <= 1; south++) {
                let here = struck + vec2<f32>(f32(east), f32(south));
                // Only as many patches are struck as it rains hard.
                if hash2(here + 7.3) > falling {
                    continue;
                }
                let middle = (here + 0.5) * sends.w;
                ring = nearer(ring, ring_from(at.xz, middle, vec4<f32>(sends.xyz, sends.w * 0.5), lead.z, hash2(here)));
            }
        }
    }
    return ring;
}

// How far from the flesh of a body at rest its rings spread, in metres: none where it sends
// none.
fn rest_reach() -> f32 {
    return weather_texel(ripples_first() + 1u).x;
}

// How far over or under a surface a thing that stirs the water may stand and still stir it, in
// metres.
const SPOT_LEVEL: f32 = 0.5;

// Where the first texel of the rings lies: after the places every tread breaks the water.
fn ripples_first() -> u32 {
    return wetted_first() + TREADS + TREADS * WETTED_PER_TREAD;
}

// The nearer of two rings to the point they are told at; a ring that is none loses.
fn nearer(a: Ring, b: Ring) -> Ring {
    if b.strength > 0.0 && (a.strength <= 0.0 || b.off < a.off) {
        return b;
    }
    return a;
}

// The ring passing nearest `at`, of those spreading from round `middle` as `sends` tells: each
// widens to `sends.x` metres over `sends.y` seconds, a new one starts every `sends.z` seconds
// anywhere within `sends.w` metres of the middle, and `seed` sets when and where they start
// apart from another place's.
fn ring_from(at: vec2<f32>, middle: vec2<f32>, sends: vec4<f32>, strength: f32, seed: f32) -> Ring {
    var ring = Ring(1e9, 1.0, 0.0);
    let turn = weather.time / sends.z + seed;
    let count = min(u32(ceil(sends.y / sends.z)), MOST_RINGS);
    for (var back = 0u; back < count; back++) {
        let born = floor(turn) - f32(back);
        let age = (turn - born) * sends.z / sends.y;
        if age >= 1.0 {
            continue;
        }
        let strayed = vec2<f32>(hash2(vec2<f32>(born, seed * 61.7)), hash2(vec2<f32>(seed * 37.1, born))) * 2.0 - 1.0;
        let wide = sends.x * (1.0 - (1.0 - age) * (1.0 - age));
        let off = abs(distance(at, middle + strayed * sends.w) - wide);
        if off < ring.off {
            ring = Ring(off, age, strength);
        }
    }
    return ring;
}
