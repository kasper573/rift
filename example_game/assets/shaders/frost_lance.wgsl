// Lances of ice thrust across the site: each grows from its start, slow to leave and quick to
// land, strikes with a burst of ice at its far end (a crisp ring, a flash, splinters flying on
// over the margin and a patch of frost left on the ground) and then crumbles, its shaft
// breaking into pieces that scatter and vanish. How many lances, and where, follows the body:
// a circle sends them out from its centre in every direction, a rectangle a rank of them in
// unison along its facing, a cone a fan from its point, a line one or several side by side,
// a path a single lance snaking through every bend, a sprite one lance driven into its middle.
// Drawn on the pixel grid.
//   color   the ice; its alpha scales the whole effect
//   shards  how many splinters each lance bursts into where it lands
const MAX_LANCES: u32 = 12u;
const THRUST: f32 = 0.35;
const PIECE: f32 = 0.35;

fn vfx(uv: vec2<f32>) -> vec4<f32> {
    let suv = site_uv(pixel_uv(uv));
    let image = sample(suv);
    let p = site_local(suv);
    let pixel = site.tiles.x / max(site.size.x, 1.0);
    let t = site.progress;
    var hit = vec4<f32>(0.0);
    if site.kind == SITE_PATH && site.point_count >= 2u {
        let near = path_nearest(suv);
        let length_ = path_length();
        let end = site_local(path_point(site.point_count - 1u));
        hit = strike(near.y * length_, near.x, length_, t, pixel, 0.0, p - end);
    } else {
        let count = lance_count();
        for (var i = 0u; i < count; i++) {
            let start = lance_start(i, count);
            let dir = lance_direction(i, count);
            let length_ = lance_length();
            let local = p - start;
            let x = dot(local, dir);
            let y = dot(local, vec2<f32>(-dir.y, dir.x));
            hit = max(hit, strike(x, y, length_, t, pixel, f32(i), p - (start + dir * length_)));
        }
    }
    let lingering = 1.0 - smoothstep(0.7, 1.0, t);
    let ice = clamp(max(max(hit.x, hit.y), hit.z), 0.0, 1.0);
    let frost = hit.w * lingering * 0.35;
    let rgb = mix(params.color.rgb, vec3<f32>(1.0), step(0.5, max(hit.y, hit.z)));
    return over(vec4<f32>(rgb, max(ice, frost) * params.color.a), image);
}

// One lance at a point `x` along and `y` across it, `from_end` being the point relative to where
// the lance lands: its shaft, what shines white (an edge line, the tip, the ring and splinters of
// the impact), the flash of the impact and the frost it leaves.
fn strike(x: f32, y: f32, length_: f32, t: f32, pixel: f32, seed: f32, from_end: vec2<f32>) -> vec4<f32> {
    let thrust = pow(clamp(t / THRUST, 0.0, 1.0), 2.0);
    let tip = length_ * thrust;
    let crumbling = max(t - THRUST, 0.0);
    let piece = floor(x / PIECE) + seed * 7.0;
    let delay = 0.05 + 0.3 * hash(piece * 1.7);
    let fall = max(crumbling - delay, 0.0) * step(0.001, crumbling);
    let scatter = select(-1.0, 1.0, hash(piece * 3.1) > 0.5);
    let yy = y - scatter * fall * fall * 10.0;
    let alive = step(fall, 0.25);
    let taper = 1.0 - smoothstep(tip - 0.9, tip, x);
    let half = 0.2 * taper * min(length_ / 1.5, 1.0) + pixel;
    let inside = step(0.0, x) * step(x, tip) * alive;
    let shaft = step(abs(yy), half) * inside;
    let edge = step(abs(yy - half * 0.45), pixel) * step(pixel * 2.5, half);
    let bright = max(edge, step(tip - 0.4, x)) * shaft;
    let since = clamp((t - THRUST) / 0.35, 0.0, 1.0);
    let struck = step(0.001, since) * step(since, 0.999);
    let d = length(from_end);
    let ring = step(abs(d - since * 1.1), 0.08) * struck * step(since, 0.7);
    let flash = floor((1.0 - clamp(d / 0.6, 0.0, 0.999)) * 3.0) / 3.0 * struck * (1.0 - smoothstep(0.0, 0.5, since));
    var splinters = 0.0;
    let count = u32(clamp(params.shards, 1.0, 16.0));
    for (var s = 0u; s < count; s++) {
        let hs = f32(s) * 2.17 + seed * 5.3;
        let heading = normalize(vec2<f32>(hash(hs) - 0.5, hash(hs + 1.0) - 0.5));
        let at = heading * since * (0.6 + 1.2 * hash(hs + 2.0));
        let dd = abs(from_end - at);
        let square = step(max(dd.x, dd.y), pixel * select(1.0, 1.6, since < 0.5));
        splinters = max(splinters, square * struck * step(since, 0.5 + 0.5 * hash(hs + 3.0)));
    }
    let frost = step(d, 0.6 * min(since * 3.0, 1.0)) * step(0.58, noise(from_end * 5.0 + seed)) * step(0.001, since);
    return vec4<f32>(shaft, max(bright, max(ring, splinters)), flash, frost);
}

fn lance_count() -> u32 {
    switch site.shape {
        case 1u: {
            return 8u;
        }
        case 2u, 4u: {
            return clamp(u32(round(site.extent.y / 0.6)), 1u, MAX_LANCES);
        }
        case 3u: {
            return clamp(u32(round(degrees(site.extent.x) / 18.0)), 2u, MAX_LANCES);
        }
        default: {
            return 1u;
        }
    }
}

fn lance_start(index: u32, count: u32) -> vec2<f32> {
    let rank = (f32(index) + 0.5) / f32(count);
    switch site.shape {
        case 2u: {
            return vec2<f32>(-site.extent.x * 0.5, -site.extent.y * 0.5 + rank * site.extent.y);
        }
        case 4u: {
            return vec2<f32>(0.0, -site.extent.y * 0.5 + rank * site.extent.y);
        }
        case 1u, 3u: {
            return vec2<f32>(0.0);
        }
        default: {
            return vec2<f32>(-lance_length(), 0.0);
        }
    }
}

fn lance_direction(index: u32, count: u32) -> vec2<f32> {
    let rank = (f32(index) + 0.5) / f32(count);
    switch site.shape {
        case 1u: {
            let angle = rank * 6.2831853 + 0.3;
            return vec2<f32>(cos(angle), sin(angle));
        }
        case 3u: {
            let angle = (rank - 0.5) * site.extent.x;
            return vec2<f32>(cos(angle), sin(angle));
        }
        default: {
            return vec2<f32>(1.0, 0.0);
        }
    }
}

fn lance_length() -> f32 {
    switch site.shape {
        case 1u, 2u, 4u: {
            return max(site.extent.x, 0.05);
        }
        case 3u: {
            return max(site.extent.y, 0.05);
        }
        default: {
            return site.tiles.x * 0.5 + 1.2;
        }
    }
}

fn hash(x: f32) -> f32 {
    return fract(sin(x * 12.9898) * 43758.5453);
}

fn hash2(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(127.1, 311.7))) * 43758.5453);
}

fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash2(i);
    let b = hash2(i + vec2<f32>(1.0, 0.0));
    let c = hash2(i + vec2<f32>(0.0, 1.0));
    let d = hash2(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}
