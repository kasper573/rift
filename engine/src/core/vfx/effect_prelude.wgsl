// The helpers every effect shader is composed with: hashes and noise, the shapes of light and
// fire, and how a coat reads the part or the ground it is drawn over.
const PI: f32 = 3.14159265;
const TAU: f32 = 6.2831853;

fn hash11(p: f32) -> f32 {
    var x = fract(p * 0.1031);
    x = x * (x + 33.33);
    return fract(x * (x + x));
}

fn hash21(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

fn hash22(p: vec2<f32>) -> vec2<f32> {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * vec3<f32>(0.1031, 0.1030, 0.0973));
    p3 = p3 + dot(p3, p3.yzx + 33.33);
    return fract((p3.xx + p3.yz) * p3.zy);
}

fn hash31(p: vec3<f32>) -> f32 {
    var p3 = fract(p * 0.1031);
    p3 = p3 + dot(p3, p3.zyx + 31.32);
    return fract((p3.x + p3.y) * p3.z);
}

// Value noise in 2D, 0 to 1.
fn noise2(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(
        mix(hash21(i), hash21(i + vec2<f32>(1.0, 0.0)), u.x),
        mix(hash21(i + vec2<f32>(0.0, 1.0)), hash21(i + vec2<f32>(1.0, 1.0)), u.x),
        u.y,
    );
}

// Value noise in 3D, 0 to 1.
fn noise3(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = mix(hash31(i), hash31(i + vec3<f32>(1.0, 0.0, 0.0)), u.x);
    let b = mix(hash31(i + vec3<f32>(0.0, 1.0, 0.0)), hash31(i + vec3<f32>(1.0, 1.0, 0.0)), u.x);
    let c = mix(hash31(i + vec3<f32>(0.0, 0.0, 1.0)), hash31(i + vec3<f32>(1.0, 0.0, 1.0)), u.x);
    let d = mix(hash31(i + vec3<f32>(0.0, 1.0, 1.0)), hash31(i + vec3<f32>(1.0, 1.0, 1.0)), u.x);
    return mix(mix(a, b, u.y), mix(c, d, u.y), u.z);
}

fn fbm2(p: vec2<f32>) -> f32 {
    var sum = 0.0;
    var amp = 0.5;
    var q = p;
    for (var k = 0; k < 4; k = k + 1) {
        sum = sum + amp * noise2(q);
        q = q * 2.03 + vec2<f32>(17.1, 9.2);
        amp = amp * 0.5;
    }
    return sum / 0.9375;
}

fn fbm3(p: vec3<f32>) -> f32 {
    var sum = 0.0;
    var amp = 0.5;
    var q = p;
    for (var k = 0; k < 4; k = k + 1) {
        sum = sum + amp * noise3(q);
        q = q * 2.03 + vec3<f32>(17.1, 9.2, 4.7);
        amp = amp * 0.5;
    }
    return sum / 0.9375;
}

// Distance to the nearest cell point in 2D, and to the second nearest.
fn voronoi2(p: vec2<f32>) -> vec2<f32> {
    let i = floor(p);
    let f = fract(p);
    var d1 = 8.0;
    var d2 = 8.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let g = vec2<f32>(f32(x), f32(y));
            let o = hash22(i + g);
            let d = length(g + o - f);
            if d < d1 {
                d2 = d1;
                d1 = d;
            } else if d < d2 {
                d2 = d;
            }
        }
    }
    return vec2<f32>(d1, d2);
}

fn remap01(x: f32, a: f32, b: f32) -> f32 {
    return clamp((x - a) / (b - a), 0.0, 1.0);
}

// 1 inside [a, b] of x, with soft edges `soft` wide.
fn window(x: f32, a: f32, b: f32, soft: f32) -> f32 {
    return smoothstep(a - soft, a, x) * (1.0 - smoothstep(b, b + soft, x));
}

fn fresnel(i: FxIn, power: f32) -> f32 {
    return pow(1.0 - abs(dot(i.normal, i.view)), power);
}

// Erodes a mask by a noise value as the threshold climbs: what dissolves first goes first.
fn erode(mask: f32, noise: f32, threshold: f32, soft: f32) -> f32 {
    return mask * smoothstep(threshold, threshold + soft, noise);
}

// A hot gradient: from `cool` at 0 through `warm` to `hot` at 1.
fn ramp3(t: f32, cool: vec3<f32>, warm: vec3<f32>, hot: vec3<f32>) -> vec3<f32> {
    let x = clamp(t, 0.0, 1.0);
    return select(mix(warm, hot, (x - 0.5) * 2.0), mix(cool, warm, x * 2.0), x < 0.5);
}

// Distance to the nearest cell point in 3D, and to the second nearest.
fn voronoi3(p: vec3<f32>) -> vec2<f32> {
    let i = floor(p);
    let f = fract(p);
    var d1 = 8.0;
    var d2 = 8.0;
    for (var z = -1; z <= 1; z = z + 1) {
        for (var y = -1; y <= 1; y = y + 1) {
            for (var x = -1; x <= 1; x = x + 1) {
                let g = vec3<f32>(f32(x), f32(y), f32(z));
                let c = i + g;
                let o = vec3<f32>(hash31(c), hash31(c + 19.1), hash31(c + 41.7));
                let d = length(g + o - f);
                if d < d1 {
                    d2 = d1;
                    d1 = d;
                } else if d < d2 {
                    d2 = d;
                }
            }
        }
    }
    return vec2<f32>(d1, d2);
}

// A coat's point in its part's own frame, in metres from the part's middle.
fn coat_pos(i: FxIn) -> vec3<f32> {
    return i.local * i.size;
}

// How far a coat is seen by its silhouette: the fresnel on a model, everywhere on the ground.
fn coat_rim(i: FxIn, power: f32) -> f32 {
    if i.on > 1.5 {
        return 1.0;
    }
    return fresnel(i, power);
}

// A coat eased in over its first `rise` seconds and out over its last `fall`.
fn coat_env(i: FxIn, rise: f32, fall: f32) -> f32 {
    return smoothstep(0.0, rise, i.age) * (1.0 - smoothstep(frame.life.z - fall, frame.life.z, i.age));
}

// On the ground, how far inside its patch a point is: 1 well within, 0 past its ragged edge.
fn patch_mask(i: FxIn, ragged: f32) -> f32 {
    let r = length(i.local.xz) * 2.0;
    let n = fbm2(i.local.xz * 6.0 + i.seed);
    return 1.0 - smoothstep(0.55, 0.95, r + (n - 0.5) * ragged);
}

// A point to read patterns at: on a model, where it is in metres; on the ground, the ground
// with height standing for the noise that lifts things off it.
fn coat_point(i: FxIn) -> vec3<f32> {
    if i.on > 1.5 {
        return vec3<f32>(i.world.x, 0.0, i.world.z);
    }
    return i.world;
}
