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
// its vertices: a blade's own, a copy's tint over its prop's, a sheet's ground colour. A blade's
// or a merged copy's vertex carries its root; an instanced copy stands at its instance's origin.
@if(PREPASS_PIPELINE)
import bevy_pbr::prepass::io::{Vertex, VertexOutput, FragmentOutput, decompress_vertex};
@else
import bevy_pbr::render::forward_io::{Vertex, VertexOutput, FragmentOutput, decompress_vertex};
import bevy_pbr::render::{mesh_functions, view_transformations::position_world_to_clip};
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
    on: u32,
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

// A strewn copy's marks as a sheet's or a blade's vertices carry them: how far up its prop it
// stands from its mesh, its weight and seed from its tag, and its piece from its material.
fn copy_marks(measure: f32, tag: u32) -> vec2<f32> {
    let weight = min(f32(tag & 255u) / 255.0, 255.0 / 256.0);
    let seed = f32((tag >> 8u) & 255u);
    return vec2<f32>(measure, weight + seed + f32(strewn.piece) * 256.0);
}

@if(ROOTED)
@vertex
fn vertex(vertex_in: Vertex, @location(14) rooted: vec3<f32>) -> Output {
    return handed_on(stand(vertex_in, vec4<f32>(rooted, 1.0)));
}

@if(WATERED)
@vertex
fn vertex(vertex_in: Vertex, @location(15) water: vec3<f32>) -> Output {
    ashore = water.x;
    falling = water.yz;
    return handed_on(stand(vertex_in, vec4<f32>(0.0)));
}

@if(!ROOTED && !WATERED)
@vertex
fn vertex(vertex_in: Vertex) -> Output {
    return handed_on(stand(vertex_in, select(vec4<f32>(0.0), vec4<f32>(0.0, 0.0, 0.0, 1.0), strewn.on != 0u)));
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

// The vertex standing in the world, swayed about its root: `rooted`, in the mesh's own space, or
// the vertex itself where `rooted.w` is 0.
fn stand(vertex_in: Vertex, rooted: vec4<f32>) -> VertexOutput {
    var out: VertexOutput;
    let vertex = decompress_vertex(vertex_in, vertex_in.instance_index);
    let world_from_local = mesh_functions::get_world_from_local(vertex_in.instance_index);
    let placed = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));
    stem = placed.xyz;
    if rooted.w > 0.5 {
        stem = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(rooted.xyz, 1.0)).xyz;
    }
    painted = 1.0;
    up = 0.0;
    salt = 0.0;
    part = 0u;
    @if(VERTEX_UVS_A) {
        out.uv = vertex.uv;
    }
    var tint = vec4<f32>(1.0);
    @if(VERTEX_UVS_B) {
        var marks = vertex.uv_b;
        if strewn.on != 0u {
            let tag = mesh_functions::get_tag(vertex_in.instance_index);
            marks = copy_marks(vertex.uv_b.x, tag);
            tint = mix(strewn.color_from, strewn.color_to, f32((tag >> 16u) & 255u) / 255.0);
        }
        mark(marks);
        out.uv_b = marks;
    }
    spot = placed.xyz;
    read_weather();
    let world = vec4<f32>(sway(placed.xyz, up), 1.0);
    out.world_position = world;
    out.position = position_world_to_clip(world.xyz);
    @if(PREPASS_PIPELINE) {
        @if(UNCLIPPED_DEPTH_ORTHO_EMULATION) {
            out.unclipped_depth = out.position.z;
            out.position.z = min(out.position.z, 1.0);
        }
        @if(NORMAL_PREPASS_OR_DEFERRED_PREPASS && VERTEX_NORMALS) {
            out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex_in.instance_index);
        }
        @if(MOTION_VECTOR_PREPASS) {
            out.previous_world_position = world;
        }
    }
    @else {
        @if(VERTEX_NORMALS) {
            out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex_in.instance_index);
        }
    }
    @if(VERTEX_COLORS) {
        out.color = vertex.color * tint;
    }
    @if(VERTEX_OUTPUT_INSTANCE_INDEX) {
        out.instance_index = vertex_in.instance_index;
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
    var pbr_input = pbr_input_from_standard_material(in, is_front);
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
    painted = 1.0;
    up = 0.0;
    salt = 0.0;
    part = 0u;
    var uv = vec2<f32>(0.5);
    @if(VERTEX_UVS_A) {
        uv = in.uv;
    }
    @if(VERTEX_UVS_B) {
        mark(in.uv_b);
    }
    spot = in.world_position.xyz;
    read_weather();
    var out: FragmentOutput;
    out.color = main_pass_post_lighting_processing(pbr_input, vfx(uv));
    return out;
}
