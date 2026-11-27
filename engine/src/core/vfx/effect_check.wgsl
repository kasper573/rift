// The effect header as the validator sees it, in plain WGSL: the same bindings, input and helpers,
// the view as far as an effect reads it, and an entry that calls what an effect shader must
// define, `fx`.
struct Fx {
    origin: vec4<f32>,
    rotation: vec4<f32>,
    scale: vec4<f32>,
    life: vec4<f32>,
    site: vec4<f32>,
    extra: vec4<f32>,
    mask: vec4<f32>,
}

struct FxIn {
    uv: vec2<f32>,
    world: vec3<f32>,
    local: vec3<f32>,
    normal: vec3<f32>,
    view: vec3<f32>,
    life: f32,
    age: f32,
    seed: f32,
    front: bool,
    pixel: vec4<f32>,
    size: vec3<f32>,
    on: f32,
}

struct View {
    world_position: vec3<f32>,
}

@group(0) @binding(0) var<uniform> view: View;
@group(3) @binding(0) var<uniform> frame: Fx;
@group(3) @binding(1) var<uniform> params: Params;

fn depth_gap(pixel: vec4<f32>, world: vec3<f32>) -> f32 {
    return 1000.0;
}

fn unrotate(q: vec4<f32>, v: vec3<f32>) -> vec3<f32> {
    let p = vec4<f32>(-q.xyz, q.w);
    let t = 2.0 * cross(p.xyz, v);
    return v + p.w * t + cross(p.xyz, t);
}

@fragment
fn fragment(@builtin(position) pixel: vec4<f32>) -> @location(0) vec4<f32> {
    var i: FxIn;
    i.pixel = pixel;
    return fx(i);
}
