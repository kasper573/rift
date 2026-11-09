// How a cover's blades bend, about where each stands (`root`): `tread_push` gives how far the
// bodies treading through push a blade's tip aside, and `lean` bends a vertex of the blade so its
// tip shifts that far, the blade curving over in an arc and keeping its length. A body treads on
// its limbs, each a bone with the flesh round it, and a blade bends only as far as it must to
// clear the limbs that reach it, so a body raised over the cover leaves it standing. Water reads the
// same limbs where they break its surface (`limb_crossing`), and where each body lately stood
// (`trail`), for the rings and the wake it leaves.

// How far over a blade bends at most, in radians from upright: it never lies quite flat.
const MOST_BEND: f32 = 1.4;

// How far the limbs of the bodies treading nearby push aside the tip of the blade whose point
// `world` stands `tip` of the way up it, in metres over the ground, the cover feeling each limb
// `brush` metres beyond its flesh as the body brushes through. The blade is pushed as a whole,
// from where it stands: a strewn copy spreads wide round its root, and pushed point by point the
// points either side of a limb would part and tear its faces across it.
fn tread_push(world: vec3<f32>, tip: f32, brush: f32) -> vec2<f32> {
    let base = root();
    let height = blade_height(world, tip, base);
    var push = vec2<f32>(0.0);
    if height <= 0.0 {
        return push;
    }
    for (var tread = 0u; tread < weather.treads; tread++) {
        let body = weather_texel(weather_parts.z + tread);
        if distance(base.xz, body.xy) > body.z + brush {
            continue;
        }
        let first = weather_parts.z + TREADS + tread * LIMBS_PER_TREAD * 2u;
        for (var limb = 0u; limb < u32(body.w); limb++) {
            let head = weather_texel(first + limb * 2u);
            let end = weather_texel(first + limb * 2u + 1u);
            push += clearing(base, height, head.xyz, end.xyz, head.w + brush);
        }
    }
    return push;
}

// The vertex at `world`, `tip` of the way up its blade, bent so the blade's tip leans `shift`
// metres over the ground: the blade curves over in an arc of its own length about its root, as
// far as a slight bend that shifts its tip so far, never past `MOST_BEND`.
fn lean(world: vec3<f32>, tip: f32, shift: vec2<f32>) -> vec3<f32> {
    let base = root();
    let height = blade_height(world, tip, base);
    let amount = length(shift);
    if height <= 0.0 || amount < 1e-5 {
        return world;
    }
    let bend = min(2.0 * amount / height, MOST_BEND);
    let way = shift / amount;
    let turned = bend * tip;
    let along = height / bend * (1.0 - cos(turned));
    let up = height / bend * sin(turned);
    return vec3<f32>(world.x + way.x * along, base.y + up, world.z + way.y * along);
}

// How tall the blade stands whose point `world` stands `tip` of the way up it from `base`; 0
// at its root, where nothing bends.
fn blade_height(world: vec3<f32>, tip: f32, base: vec3<f32>) -> f32 {
    let rise = world.y - base.y;
    if tip <= 0.0 || rise <= 0.0 {
        return 0.0;
    }
    return rise / tip;
}

// How far the limb from `head` to `end` with `girth` metres of flesh round it pushes aside the tip
// of a blade standing `height` tall at `base`: as far as a slight bend that turns the blade from
// upright just far enough to clear the limb's flesh at the point of the bone nearest the blade.
fn clearing(base: vec3<f32>, height: f32, head: vec3<f32>, end: vec3<f32>, girth: f32) -> vec2<f32> {
    let bone = end - head;
    let flat = dot(bone.xz, bone.xz);
    var share = select(1.0, 0.0, head.y < end.y);
    if flat > 1e-6 {
        share = clamp(dot(base.xz - head.xz, bone.xz) / flat, 0.0, 1.0);
    }
    let nearest = head + bone * share;
    let away = base.xz - nearest.xz;
    let apart = length(away);
    let above = nearest.y - base.y;
    let reach = length(vec2<f32>(apart, above));
    if reach - girth > height || above + girth < 0.0 {
        return vec2<f32>(0.0);
    }
    var way = vec2<f32>(1.0, 0.0);
    if apart > 1e-4 {
        way = away / apart;
    } else if flat > 1e-6 {
        way = normalize(vec2<f32>(-bone.z, bone.x));
    }
    let rising = atan2(above, apart);
    var bend = mix(MOST_BEND, rising, reach / girth);
    if reach > girth {
        bend = rising - acos(girth / reach);
    }
    bend = clamp(bend, 0.0, MOST_BEND);
    return way * bend * height / 2.0;
}

// Where the `tread`th body treading nearby stood the `point`th place back along its trail, the
// newest first, a place every `TRAIL_EVERY` seconds: its ground point in xy and the moment it
// stood there in z, with w 1, or nothing past the trail's end.
fn trail(tread: u32, point: u32) -> vec4<f32> {
    if tread >= weather.treads || point >= TRAIL_POINTS {
        return vec4<f32>(0.0);
    }
    let first = weather_parts.z + TREADS + TREADS * LIMBS_PER_TREAD * 2u + tread * TRAIL_POINTS;
    return weather_texel(first + point);
}

// How far `at`, a ground point, lies from the flesh of the nearest limb of a body treading nearby
// that breaks through a surface at `level`, as water's: where the limb's bone crosses the level, or
// its lowest end lies under it, less its flesh; `far` where no limb breaks through within `far`.
fn limb_crossing(at: vec2<f32>, level: f32, far: f32) -> f32 {
    var nearest = far;
    for (var tread = 0u; tread < weather.treads; tread++) {
        let body = weather_texel(weather_parts.z + tread);
        if distance(at, body.xy) > body.z + far {
            continue;
        }
        let first = weather_parts.z + TREADS + tread * LIMBS_PER_TREAD * 2u;
        for (var limb = 0u; limb < u32(body.w); limb++) {
            let head = weather_texel(first + limb * 2u);
            let end = weather_texel(first + limb * 2u + 1u);
            let low = select(end.xyz, head.xyz, head.y < end.y);
            let high = select(head.xyz, end.xyz, head.y < end.y);
            if low.y - head.w > level || high.y + head.w < level {
                continue;
            }
            var crossing = low.xz;
            if high.y > level && high.y - low.y > 1e-4 {
                crossing = mix(low.xz, high.xz, clamp((level - low.y) / (high.y - low.y), 0.0, 1.0));
            }
            nearest = min(nearest, max(distance(at, crossing) - head.w, 0.0));
        }
    }
    return nearest;
}
