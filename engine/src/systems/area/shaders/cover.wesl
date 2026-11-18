// The header of a shader that covers a ground: what the material grows or lies under is its mesh,
// a sheet over the painted cells, blades or strewn copies of a prop, laid out by the engine from
// the paint. The shader says how a vertex moves (`sway`, from the weather the engine tells it of)
// and what colour a pixel is (`vfx`), where `sample` is the cover lit in the ground's colour,
// `light` the light falling there (what a white face would be lit to, for a shader that paints
// in colours of its own), `albedo` the cover's own colour unlit, as its texture paints it without
// its vertices' colours, `weight` how heavily the material is painted there, `tip` how far up a
// blade or a strewn prop the point stands, 0 at its root and 1 at its top, `position` where it
// stands in the world, `depth` how many metres of water stand over the ground under a sheet on
// water, `shore` how far it lies from the nearest dry ground, up to `SHORE_REACH` metres, `fall`
// how the water's surface falls there in metres a metre toward east and south (x and y: the way
// the water flows, and how steeply), `light_facing(normal)` the light falling on a white face
// turned to `normal` (for a shader that shapes its own faces, such as waves), `sun_way` the way
// toward the light the world is lit by and `eye` where the camera looks from (for the glints off
// such faces), `piece` which of the
// cover's pieces the point belongs to (0 on a sheet), `seed` a number of the blade's or the copy's
// own, 0 on a sheet, and, as a vertex moves, `root`, where its blade or copy stands on the ground
// (a sheet's vertex is its own root). Off water, `depth`, `shore` and `fall` are 0. The cover's colours come from
// its vertices: a blade's own, a copy's tint over its prop's, a sheet's ground colour. A copy is
// an instance of its prop's model the scenery's arena holds, standing at its instance's origin,
// its tint and seed riding on the instance. A sheet is drawn over the ground's patches, its points
// read from the ground as they are drawn, and blades from the blades the GPU laid out for the
// ground; both are lit as a white, rough, blended face of their colour.
@if(PREPASS_PIPELINE)
import bevy_pbr::prepass::io::{VertexOutput, FragmentOutput};
@else
import bevy_pbr::render::forward_io::{VertexOutput, FragmentOutput};
import bevy_pbr::render::view_transformations::position_world_to_clip;
@if(PREPASS_PIPELINE)
import engine::core::render::world::prepass_write::prepass_output;
@else
import bevy_pbr::render::{pbr_fragment::pbr_input_from_standard_material, pbr_functions::{alpha_discard, main_pass_post_lighting_processing}};
@if(!PREPASS_PIPELINE)
import engine::core::render::world::shade::lighting;
@if(!PREPASS_PIPELINE)
import bevy_pbr::render::pbr_types::PbrInput;
import bevy_pbr::render::mesh_view_bindings::view;
@if(!PREPASS_PIPELINE)
import bevy_pbr::render::mesh_view_bindings::lights;
@if(SHEET_PATCH)
import engine::core::render::world::gpu::sheet::{drawn_sheet, sheet_point};
@if(GRASS_BLADE)
import engine::core::render::world::gpu::grass::blade_point;
@if(!SHEET_PATCH && !GRASS_BLADE)
import engine::core::render::world::gpu::instance::{placement, seed_of, turned, world_of};
@if(SHEET_PATCH || GRASS_BLADE)
import bevy_pbr::render::mesh_types::{MESH_FLAGS_SHADOW_RECEIVER_BIT, MESH_FLAGS_SIGN_DETERMINANT_MODEL_3X3_BIT};
@if(SHEET_PATCH || GRASS_BLADE)
import bevy_pbr::render::pbr_functions::calculate_view;
@if(SHEET_PATCH || GRASS_BLADE)
import bevy_pbr::render::pbr_lighting::perceptualRoughnessToRoughness;
@if(SHEET_PATCH || GRASS_BLADE)
import bevy_pbr::render::pbr_types::{pbr_input_new, STANDARD_MATERIAL_FLAGS_ALPHA_MODE_BLEND, STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT};
@if(SCREEN_SPACE_AMBIENT_OCCLUSION)
import bevy_pbr::render::mesh_view_bindings::screen_space_ambient_occlusion_texture;
@if(SCREEN_SPACE_AMBIENT_OCCLUSION)
import bevy_pbr::ssao::utils::ssao_multibounce;

@if(PREPASS_PIPELINE)
import bevy_render::globals::Globals;
@else
import bevy_pbr::render::mesh_view_bindings::globals;
// The prepass binds the frame's globals right after the view, where the main pass binds them
// among the lights.
@if(PREPASS_PIPELINE)
@group(0) @binding(1) var<uniform> globals: Globals;

@group(3) @binding(100) var<uniform> site: Site;
@group(3) @binding(101) var<uniform> params: Params;
@group(3) @binding(103) var<uniform> strewn: Strewn;

struct Strewn {
    color_from: vec4<f32>,
    color_to: vec4<f32>,
    piece: u32,
    height: f32,
}

var<private> lit: vec4<f32>;
var<private> shine: vec3<f32>;
var<private> own: vec4<f32>;
var<private> painted: f32;
var<private> up: f32;
var<private> salt: f32;
var<private> part: u32;
var<private> spot: vec3<f32>;
var<private> stem: vec3<f32>;
var<private> ashore: f32;
var<private> falling: vec2<f32>;
@if(!PREPASS_PIPELINE)
var<private> facing_input: PbrInput;

// The vertex's output as the main pass hands it on: bevy's, with the water a sheet on water
// carries, how far from the shore and how the surface falls.
@if(!PREPASS_PIPELINE)
struct CoverOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
    @if(VERTEX_UVS_A)
    @location(2) uv: vec2<f32>,
    @if(VERTEX_UVS_B)
    @location(3) uv_b: vec2<f32>,
    @if(VERTEX_TANGENTS)
    @location(4) world_tangent: vec4<f32>,
    @if(VERTEX_COLORS)
    @location(5) color: vec4<f32>,
    @if(VERTEX_OUTPUT_INSTANCE_INDEX)
    @location(6) @interpolate(flat) instance_index: u32,
    @if(VISIBILITY_RANGE_DITHER)
    @location(7) @interpolate(flat) visibility_range_dither: i32,
    @location(8) water: vec3<f32>,
}

@if(!PREPASS_PIPELINE)
alias Output = CoverOutput;
@else
alias Output = VertexOutput;

fn sample(at: vec2<f32>) -> vec4<f32> {
    return lit;
}

fn light() -> vec3<f32> {
    return shine;
}

fn albedo() -> vec4<f32> {
    return own;
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

fn position() -> vec3<f32> {
    return spot;
}

fn root() -> vec3<f32> {
    return stem;
}

fn shore() -> f32 {
    return ashore;
}

fn fall() -> vec2<f32> {
    return falling;
}

@if(!PREPASS_PIPELINE)
fn light_facing(normal: vec3<f32>) -> vec3<f32> {
    var input = facing_input;
    let facing = normalize(normal);
    input.N = facing;
    input.world_normal = facing;
    return lighting(input).rgb;
}
@else
fn light_facing(normal: vec3<f32>) -> vec3<f32> {
    return shine;
}

@if(!PREPASS_PIPELINE)
fn sun_way() -> vec3<f32> {
    if lights.n_directional_lights == 0u {
        return vec3<f32>(0.0, 1.0, 0.0);
    }
    return lights.directional_lights[0].direction_to_light;
}
@else
fn sun_way() -> vec3<f32> {
    return vec3<f32>(0.0, 1.0, 0.0);
}

fn eye() -> vec3<f32> {
    return view.world_position;
}

fn mark(uv_b: vec2<f32>) {
    up = uv_b.x;
    painted = fract(uv_b.y);
    let steps = floor(uv_b.y);
    part = u32(steps / 256.0);
    salt = (steps - f32(part) * 256.0) / 255.0;
}

@if(SHEET_PATCH)
@vertex
fn vertex(@builtin(instance_index) instance: u32, @location(0) grid: vec3<f32>) -> Output {
    let point = sheet_point(grid, instance);
    ashore = point.shore;
    falling = point.fall;
    stem = point.world;
    spot = point.world;
    let marks = vec2<f32>(point.depth, min(point.weight, 255.0 / 256.0));
    mark(marks);
    read_weather();
    var out: VertexOutput;
    out.uv = point.uv;
    out.uv_b = marks;
    out.color = drawn_sheet.color;
    out.world_normal = point.normal;
    let world = vec4<f32>(sway(point.world, up), 1.0);
    out.world_position = world;
    out.position = position_world_to_clip(world.xyz);
    @if(VERTEX_OUTPUT_INSTANCE_INDEX) {
        out.instance_index = instance;
    }
    return handed_on(out);
}

@if(GRASS_BLADE)
@vertex
fn vertex(@builtin(vertex_index) index: u32) -> Output {
    let point = blade_point(index);
    ashore = 0.0;
    falling = vec2<f32>(0.0);
    stem = point.root;
    spot = point.world;
    let weight = clamp(point.weight, 0.0, 255.0 / 256.0);
    let steps = floor(clamp(point.seed, 0.0, 1.0) * 255.0) + f32(point.piece) * 256.0;
    let marks = vec2<f32>(point.measure, weight + steps);
    mark(marks);
    read_weather();
    var out: VertexOutput;
    out.uv = point.uv;
    out.uv_b = marks;
    out.color = point.color;
    out.world_normal = point.normal;
    let world = vec4<f32>(sway(point.world, up), 1.0);
    out.world_position = world;
    out.position = select(vec4<f32>(0.0), position_world_to_clip(world.xyz), point.shown);
    @if(VERTEX_OUTPUT_INSTANCE_INDEX) {
        out.instance_index = 0u;
    }
    return handed_on(out);
}

// A strewn copy, standing where the culling kernel drew its instance and swayed about its origin:
// how far up its prop the vertex stands, its weight full, and its seed and tint from the
// instance's seed, the seed in its upper bits as the kernel thins it by and the tint in its lowest
// byte.
// A strewn copy's vertex as the slab of models holds it, of the instance its command draws as its
// `instance_index`th, at the locations bevy's prepass and main pass each lay a mesh out at. Not
// bevy's own vertex, which under `VERTEX_UVS_B` (the marks the copies hand their pixels) would
// read a second uv the slab doesn't hold.
@if(!SHEET_PATCH && !GRASS_BLADE)
struct CopyVertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @if(PREPASS_PIPELINE && VERTEX_UVS_A)
    @location(1) uv: vec2<f32>,
    @if(!PREPASS_PIPELINE && VERTEX_UVS_A)
    @location(2) uv: vec2<f32>,
    @if(PREPASS_PIPELINE && NORMAL_PREPASS_OR_DEFERRED_PREPASS && VERTEX_NORMALS)
    @location(3) normal: vec3<f32>,
    @if(!PREPASS_PIPELINE && VERTEX_NORMALS)
    @location(1) normal: vec3<f32>,
    @if(PREPASS_PIPELINE && VERTEX_COLORS)
    @location(7) color: vec4<f32>,
    @if(!PREPASS_PIPELINE && VERTEX_COLORS)
    @location(5) color: vec4<f32>,
}

@if(!SHEET_PATCH && !GRASS_BLADE)
@vertex
fn vertex(in: CopyVertex) -> Output {
    let placed = placement(in.instance_index);
    let drawn = seed_of(in.instance_index);
    let at = world_of(placed, in.position);
    ashore = 0.0;
    falling = vec2<f32>(0.0);
    stem = placed.at;
    spot = at;
    let measure = clamp(in.position.y / strewn.height, 0.0, 1.0);
    let steps = floor(f32(drawn >> 8u) / 16777216.0 * 255.0) + f32(strewn.piece) * 256.0;
    let marks = vec2<f32>(measure, 255.0 / 256.0 + steps);
    mark(marks);
    read_weather();
    var out: VertexOutput;
    @if(VERTEX_UVS_A) {
        out.uv = in.uv;
    }
    @if(VERTEX_UVS_B) {
        out.uv_b = marks;
    }
    @if(VERTEX_COLORS) {
        let tint = mix(strewn.color_from, strewn.color_to, f32(drawn & 255u) / 255.0);
        out.color = in.color * tint;
    }
    let world = vec4<f32>(sway(at, up), 1.0);
    out.world_position = world;
    out.position = position_world_to_clip(world.xyz);
    @if(VERTEX_NORMALS && (!PREPASS_PIPELINE || NORMAL_PREPASS_OR_DEFERRED_PREPASS)) {
        out.world_normal = normalize(turned(placed.turn, in.normal));
    }
    @if(PREPASS_PIPELINE && UNCLIPPED_DEPTH_ORTHO_EMULATION) {
        out.unclipped_depth = out.position.z;
        out.position.z = min(out.position.z, 1.0);
    }
    @if(PREPASS_PIPELINE && MOTION_VECTOR_PREPASS) {
        out.previous_world_position = world;
    }
    @if(VERTEX_OUTPUT_INSTANCE_INDEX) {
        out.instance_index = 0u;
    }
    return handed_on(out);
}

// The vertex's output as its pass takes it: in the main pass with the water it lies on.
@if(!PREPASS_PIPELINE)
fn handed_on(out: VertexOutput) -> CoverOutput {
    var on: CoverOutput;
    on.position = out.position;
    on.world_position = out.world_position;
    on.world_normal = out.world_normal;
    @if(VERTEX_UVS_A) {
        on.uv = out.uv;
    }
    @if(VERTEX_UVS_B) {
        on.uv_b = out.uv_b;
    }
    @if(VERTEX_TANGENTS) {
        on.world_tangent = out.world_tangent;
    }
    @if(VERTEX_COLORS) {
        on.color = out.color;
    }
    @if(VERTEX_OUTPUT_INSTANCE_INDEX) {
        on.instance_index = out.instance_index;
    }
    @if(VISIBILITY_RANGE_DITHER) {
        on.visibility_range_dither = out.visibility_range_dither;
    }
    on.water = vec3<f32>(ashore, falling);
    return on;
}
@else
fn handed_on(out: VertexOutput) -> VertexOutput {
    return out;
}

// What the fragment lights: a mesh's material as bevy's standard material reads it, or a sheet or
// a blade lit as a white, rough face of its colour.
@if(!PREPASS_PIPELINE && (SHEET_PATCH || GRASS_BLADE))
fn material_input(in: VertexOutput, is_front: bool) -> PbrInput {
    var input = pbr_input_new();
    input.flags = MESH_FLAGS_SHADOW_RECEIVER_BIT | MESH_FLAGS_SIGN_DETERMINANT_MODEL_3X3_BIT;
    input.material.flags =
        STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT | STANDARD_MATERIAL_FLAGS_ALPHA_MODE_BLEND;
    input.material.base_color = in.color;
    input.material.perceptual_roughness = 1.0;
    input.material.metallic = 0.0;
    input.material.reflectance = vec3<f32>(0.0);
    input.frag_coord = in.position;
    input.world_position = in.world_position;
    input.world_normal = normalize(in.world_normal);
    input.N = input.world_normal;
    input.is_orthographic = view.clip_from_view[3].w == 1.0;
    input.V = calculate_view(in.world_position, input.is_orthographic);
    @if(SCREEN_SPACE_AMBIENT_OCCLUSION) {
        let ssao_at = vec2<i32>(in.position.xy - view.viewport.xy);
        let ssao = textureLoad(screen_space_ambient_occlusion_texture, ssao_at, 0i).r;
        input.diffuse_occlusion = min(vec3<f32>(1.0), ssao_multibounce(ssao, in.color.rgb));
        let n_dot_v = max(dot(input.N, input.V), 0.0001);
        let roughness = perceptualRoughnessToRoughness(1.0);
        input.specular_occlusion =
            saturate(pow(n_dot_v + ssao, exp2(-16.0 * roughness - 1.0)) - 1.0 + ssao);
    }
    return input;
}
@elif(!PREPASS_PIPELINE)
fn material_input(in: VertexOutput, is_front: bool) -> PbrInput {
    return pbr_input_from_standard_material(in, is_front);
}

// The fragment's input as bevy's lighting reads it.
@if(!PREPASS_PIPELINE)
fn taken_in(on: CoverOutput) -> VertexOutput {
    var out: VertexOutput;
    out.position = on.position;
    out.world_position = on.world_position;
    out.world_normal = on.world_normal;
    @if(VERTEX_UVS_A) {
        out.uv = on.uv;
    }
    @if(VERTEX_UVS_B) {
        out.uv_b = on.uv_b;
    }
    @if(VERTEX_TANGENTS) {
        out.world_tangent = on.world_tangent;
    }
    @if(VERTEX_COLORS) {
        out.color = on.color;
    }
    @if(VERTEX_OUTPUT_INSTANCE_INDEX) {
        out.instance_index = on.instance_index;
    }
    @if(VISIBILITY_RANGE_DITHER) {
        out.visibility_range_dither = on.visibility_range_dither;
    }
    return out;
}

@if(PREPASS_PIPELINE && PREPASS_FRAGMENT)
@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    return prepass_output(in, is_front);
}
@elif(PREPASS_PIPELINE)
@fragment
fn fragment(in: VertexOutput) {}
@else
@fragment
fn fragment(on: CoverOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    let in = taken_in(on);
    ashore = on.water.x;
    falling = on.water.yz;
    painted = 1.0;
    up = 0.0;
    salt = 0.0;
    part = 0u;
    @if(VERTEX_UVS_B) {
        mark(in.uv_b);
    }
    // The lighting is most of what a pixel costs, and a sheet lies over every pixel of its
    // patches, those its terrain isn't painted on included: where the paint gives the cover
    // nothing, it has nothing to lay, and the pixel is dropped before the lighting is paid for.
    if painted <= 0.0 {
        discard;
    }
    var pbr_input = material_input(in, is_front);
    let base = alpha_discard(
        pbr_input.material.flags,
        pbr_input.material.alpha_cutoff,
        pbr_input.material.base_color
    );
    var tinted = vec4<f32>(1.0);
    @if(VERTEX_COLORS) {
        tinted = in.color;
    }
    own = vec4<f32>(base.rgb / max(tinted.rgb, vec3<f32>(0.001)), base.a);
    pbr_input.material.base_color = vec4<f32>(1.0, 1.0, 1.0, base.a);
    facing_input = pbr_input;
    shine = lighting(pbr_input).rgb;
    lit = vec4<f32>(shine * base.rgb, base.a);
    var uv = vec2<f32>(0.5);
    @if(VERTEX_UVS_A) {
        uv = in.uv;
    }
    spot = in.world_position.xyz;
    read_weather();
    var out: FragmentOutput;
    out.color = main_pass_post_lighting_processing(pbr_input, vfx(uv));
    return out;
}
