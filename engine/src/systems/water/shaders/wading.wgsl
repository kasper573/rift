// What breaks a water's surface, and what the water shows of it: the flesh of the bodies treading
// nearby where the surface cuts through it (`waterline`), and the ripples that travel over the
// water from wherever it was broken (`ripple`): by a body's flesh moving through the surface, by
// rain, by a thing that keeps the water round it stirred, as a shoal of fish does. A body's flesh
// is its limbs, each a bone with the skin round it, and the water is broken by how they move, so
// its ripples take the shape of what moves through it: a wake from each leg in the shallows, one
// from the hips of whoever wades deep, a hull's from its bow back along its sides. They are
// waves in the water and not drawings on it: each spreads at the game's one speed from where the
// water was broken, crosses the others and fades, sooner in a wind.

@group(3) @binding(103) var ripple_picture: texture_2d<f32>;
@group(3) @binding(104) var ripple_sampler: sampler;

// How far outside the flesh of the bodies treading nearby `at`, a ground point, lies where a
// surface at `level` cuts through them, in metres, negative within it; `far` where none lies
// within `far`.
fn waterline(at: vec2<f32>, level: f32, far: f32) -> f32 {
    var nearest = far;
    let point = vec3<f32>(at.x, level, at.y);
    for (var tread = 0u; tread < weather.treads; tread++) {
        let body = weather_texel(weather_parts.z + tread);
        if distance(at, body.xy) > body.z + far {
            continue;
        }
        let first = weather_parts.z + TREADS + tread * LIMBS_PER_TREAD * 2u;
        for (var limb = 0u; limb < u32(body.w); limb++) {
            let head = weather_texel(first + limb * 2u);
            let end = weather_texel(first + limb * 2u + 1u);
            let bone = end.xyz - head.xyz;
            let long = dot(bone, bone);
            var along = 0.0;
            if long > 1e-8 {
                along = clamp(dot(point - head.xyz, bone) / long, 0.0, 1.0);
            }
            nearest = min(nearest, distance(point, head.xyz + bone * along) - mix(head.w, end.w, along));
        }
    }
    return nearest;
}

// The ripples at a ground point: how far the water stands over its level there, under it where
// negative, as much as 1 where flesh came into the water at once (`high`); how it slopes there,
// that rise a metre toward east and south (`slope`); how far it stands over the water round it,
// a cell of the field away, which is most along a crest (`peaked`); and how churned it is, from
// still at 0 to all froth at 1 (`churned`).
struct Ripple {
    high: f32,
    slope: vec2<f32>,
    peaked: f32,
    churned: f32,
}

// The ripples at `at`, a ground point. Nothing where no ripple travels, and past the ground the
// ripples are followed over, round the camera's subject.
fn ripple(at: vec2<f32>) -> Ripple {
    let corner = weather_texel(RIPPLE_WINDOWu).xy;
    let texel = 1.0 / RIPPLE_SIDE;
    let uv = (at - corner) / RIPPLE_CELL * texel;
    let inside = min(min(uv.x, uv.y), min(1.0 - uv.x, 1.0 - uv.y));
    if inside <= 0.0 {
        return Ripple(0.0, vec2<f32>(0.0), 0.0, 0.0);
    }
    let here = textureSampleLevel(ripple_picture, ripple_sampler, uv, 0.0).rg;
    let east = textureSampleLevel(ripple_picture, ripple_sampler, uv + vec2<f32>(texel, 0.0), 0.0).r;
    let west = textureSampleLevel(ripple_picture, ripple_sampler, uv - vec2<f32>(texel, 0.0), 0.0).r;
    let south = textureSampleLevel(ripple_picture, ripple_sampler, uv + vec2<f32>(0.0, texel), 0.0).r;
    let north = textureSampleLevel(ripple_picture, ripple_sampler, uv - vec2<f32>(0.0, texel), 0.0).r;
    let rim = smoothstep(0.0, RIPPLE_RIM, inside);
    return Ripple(
        here.r * rim,
        vec2<f32>(east - west, south - north) / (2.0 * RIPPLE_CELL) * rim,
        (here.r - 0.25 * (east + west + south + north)) * rim,
        here.g * rim,
    );
}

// How far in from the rim of the ground the ripples are followed over they fade out, as a share
// of its width.
const RIPPLE_RIM: f32 = 0.06;
