// The prop header as the validator sees it, in plain WGSL: the same bindings and helpers, and
// entries that call what a prop shader must define, `sway` and `vfx`.
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) uv: vec2<f32>,
}

@group(3) @binding(101) var<uniform> params: Params;
@group(3) @binding(103) var<uniform> site: Site;

var<private> at_local: vec3<f32>;
var<private> stands: vec3<f32>;
var<private> grown: f32;
var<private> tinted: vec4<f32>;
var<private> lit: vec4<f32>;
var<private> shine: vec3<f32>;
var<private> own: vec4<f32>;
var<private> facing: vec3<f32>;
var<private> eye_way: vec3<f32>;
var<private> right_way: vec3<f32>;
var<private> up_way: vec3<f32>;

fn local() -> vec3<f32> {
    return at_local;
}

fn origin() -> vec3<f32> {
    return stands;
}

fn size() -> f32 {
    return grown;
}

fn tint() -> vec4<f32> {
    return tinted;
}

fn screen_right() -> vec3<f32> {
    return right_way;
}

fn screen_up() -> vec3<f32> {
    return up_way;
}

fn sample(at: vec2<f32>) -> vec4<f32> {
    return lit;
}

fn light() -> vec3<f32> {
    return shine;
}

fn albedo() -> vec4<f32> {
    return own;
}

fn normal() -> vec3<f32> {
    return facing;
}

fn toward_eye() -> vec3<f32> {
    return eye_way;
}

@vertex
fn vertex(@location(0) position: vec3<f32>, @location(2) uv: vec2<f32>) -> VertexOutput {
    var out: VertexOutput;
    read_weather();
    out.world_position = vec4<f32>(sway(position, uv), 1.0);
    out.position = out.world_position;
    out.uv = uv;
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    read_weather();
    return vfx(in.uv);
}
