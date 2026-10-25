// The header of a shader that covers a ground: what the material grows or lies under is its mesh,
// a sheet over the painted cells, blades or strewn copies of a prop, laid out by the engine from
// the paint. The shader says how a vertex moves (`sway`, from the weather the engine tells it of)
// and what colour a pixel is (`vfx`), where `sample` is the cover lit in the ground's colour,
// `light` the light falling there (what a white face would be lit to, for a shader that paints
// in colours of its own), `weight` how heavily the material is painted there, `tip` how far up a blade or a strewn prop
// the point stands, 0 at its root and 1 at its top, `depth` how many metres of ground lie under a
// sheet laid at a level, `piece` which of the cover's pieces the point belongs to (0 on a sheet)
// and `seed` a number of the blade's or the copy's own, 0 on a sheet. The cover's colours come
// from its vertices: a blade's own, a copy's tint over its prop's, a sheet's ground colour.
@if(PREPASS_PIPELINE)
import bevy_pbr::prepass::io::{Vertex, VertexOutput, FragmentOutput, decompress_vertex};
@else
import bevy_pbr::render::forward_io::{Vertex, VertexOutput, FragmentOutput, decompress_vertex};
import bevy_pbr::render::{mesh_functions, view_transformations::position_world_to_clip};
@if(PREPASS_PIPELINE)
import engine::core::render::world::prepass_write::prepass_output;
@else
import engine::core::render::world::shade::lit_color;

@group(3) @binding(100) var<uniform> site: Site;
@group(3) @binding(101) var<uniform> params: Params;

var<private> lit: vec4<f32>;
var<private> shine: vec3<f32>;
var<private> painted: f32;
var<private> up: f32;
var<private> salt: f32;
var<private> part: u32;

fn sample(at: vec2<f32>) -> vec4<f32> {
    return lit;
}

fn light() -> vec3<f32> {
    return shine;
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
fn vertex(vertex_in: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let vertex = decompress_vertex(vertex_in, vertex_in.instance_index);
    let world_from_local = mesh_functions::get_world_from_local(vertex_in.instance_index);
    let placed = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));
    painted = 1.0;
    up = 0.0;
    salt = 0.0;
    part = 0u;
    @if(VERTEX_UVS_A) {
        out.uv = vertex.uv;
    }
    @if(VERTEX_UVS_B) {
        mark(vertex.uv_b);
        out.uv_b = vertex.uv_b;
    }
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
        out.color = vertex.color;
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
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    lit = lit_color(in, is_front);
    shine = lit.rgb;
    @if(VERTEX_COLORS) {
        shine = lit.rgb / max(in.color.rgb, vec3<f32>(0.001));
    }
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
    read_weather();
    var out: FragmentOutput;
    out.color = vfx(uv);
    return out;
}
