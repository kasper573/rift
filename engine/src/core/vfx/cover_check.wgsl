// The cover header as the validator sees it, in plain WGSL: the same bindings and helpers, and
// entries that call what a cover shader must define, `sway` and `vfx`.
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) uv_b: vec2<f32>,
}

@group(3) @binding(100) var<uniform> site: Site;
@group(3) @binding(101) var<uniform> params: Params;

var<private> lit: vec4<f32>;
var<private> painted: f32;
var<private> up: f32;
var<private> salt: f32;
var<private> part: u32;

fn sample(at: vec2<f32>) -> vec4<f32> {
    return lit;
}

fn weight() -> f32 {
    return painted;
}

fn tip() -> f32 {
    return up;
}

fn depth() -> f32 {
    return up;
}

fn seed() -> f32 {
    return salt;
}

fn piece() -> u32 {
    return part;
}

fn mark(uv_b: vec2<f32>) {
    up = uv_b.x;
    painted = fract(uv_b.y);
    let steps = floor(uv_b.y);
    part = u32(steps / 256.0);
    salt = (steps - f32(part) * 256.0) / 255.0;
}

@vertex
fn vertex(
    @location(0) position: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) uv_b: vec2<f32>,
) -> VertexOutput {
    var out: VertexOutput;
    mark(uv_b);
    out.world_position = vec4<f32>(sway(position, up), 1.0);
    out.position = out.world_position;
    out.world_normal = vec3<f32>(0.0, 1.0, 0.0);
    out.uv = uv;
    out.uv_b = uv_b;
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    mark(in.uv_b);
    return vfx(in.uv);
}
