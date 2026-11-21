// The water header as the validator sees it, in plain WGSL: the same bindings and helpers, and
// an entry that calls what a water shader must define, `vfx`.
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(2) uv: vec2<f32>,
}

@group(3) @binding(100) var<uniform> site: Site;
@group(3) @binding(101) var<uniform> params: Params;

struct Water {
    clarity: f32,
    depths: vec3<f32>,
    murk: f32,
}

var<private> deep: f32;
var<private> ashore: f32;
var<private> falling: vec2<f32>;
var<private> spot: vec3<f32>;
var<private> shine: vec3<f32>;

fn depth() -> f32 {
    return deep;
}

fn shore() -> f32 {
    return ashore;
}

fn open() -> f32 {
    return ashore;
}

fn fall() -> vec2<f32> {
    return falling;
}

fn position() -> vec3<f32> {
    return spot;
}

fn eye() -> vec3<f32> {
    return vec3<f32>(0.0, 10.0, 0.0);
}

fn light() -> vec3<f32> {
    return shine;
}

fn bends() -> f32 {
    return 1.0;
}

fn water() -> Water {
    return Water(6.0, vec3<f32>(0.0, 0.1, 0.2), 0.25);
}

fn silt() -> vec4<f32> {
    return vec4<f32>(0.0);
}

fn bed(bend: vec2<f32>) -> vec3<f32> {
    return vec3<f32>(bend, 0.0);
}

fn sample(at: vec2<f32>) -> vec4<f32> {
    return vec4<f32>(bed(vec2<f32>(0.0)), 1.0);
}

fn column(bend: vec2<f32>) -> f32 {
    return deep + bend.x;
}

fn seen_through(behind: vec3<f32>, path: f32) -> vec3<f32> {
    return behind * exp(-path);
}

fn mirrors(normal: vec3<f32>) -> f32 {
    return normal.y;
}

fn mirror(normal: vec3<f32>, rough: f32) -> vec3<f32> {
    return normal * rough;
}

@vertex
fn vertex(@location(0) position: vec3<f32>, @location(2) uv: vec2<f32>) -> VertexOutput {
    var out: VertexOutput;
    out.world_position = vec4<f32>(position, 1.0);
    out.position = out.world_position;
    out.uv = uv;
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    spot = in.world_position.xyz;
    read_weather();
    return vfx(in.uv);
}
