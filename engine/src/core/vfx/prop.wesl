// The header of a shader that draws a material of a prop: the prop's model is its mesh, as the
// game stands it in the world. The shader says where a vertex stands (`sway`, given where the
// model puts it in the world and its uv, with the weather the engine tells it of) and what
// colour a pixel is (`vfx`). Both read where the point lies in the model's own metres (`local`),
// where the prop stands (`origin`), how much it is grown (`size`) and the point's vertex colour
// (`tint`); a vertex also reads the ways the screen's right and up run in the world
// (`screen_right`, `screen_up`), to turn a card to face the camera. A pixel also reads the
// material lit (`sample`), the light falling there on a white face (`light`), the material's own
// colour unlit (`albedo`), the way its face turns (`normal`) and the way to the eye
// (`toward_eye`). What `vfx` paints is fogged and finished like every pixel of the world, and the
// prop fades where it hides the camera's subject like everything else.
@if(PREPASS_PIPELINE)
import bevy_pbr::prepass::io::{Vertex, VertexOutput, FragmentOutput, decompress_vertex};
@else
import bevy_pbr::render::forward_io::{Vertex, VertexOutput, FragmentOutput, decompress_vertex};
import bevy_pbr::render::{mesh_functions, view_transformations::position_world_to_clip};
import bevy_pbr::render::mesh_view_bindings::view;
import engine::core::render::world::clearing::clear_pixel;
@if(PREPASS_PIPELINE)
import bevy_pbr::render::pbr_prepass_functions;
@if(PREPASS_PIPELINE)
import engine::core::render::world::prepass_write::prepass_output;
@else
import bevy_pbr::render::{pbr_fragment::pbr_input_from_standard_material, pbr_functions::{alpha_discard, main_pass_post_lighting_processing}};
@if(!PREPASS_PIPELINE)
import engine::core::render::world::shade::lighting;

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
    return normalize(view.world_from_view[0].xyz);
}

fn screen_up() -> vec3<f32> {
    return normalize(view.world_from_view[1].xyz);
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

fn stand(world_from_local: mat4x4<f32>, position: vec3<f32>) {
    at_local = position;
    stands = world_from_local[3].xyz;
    grown = length(world_from_local[0].xyz);
}

@vertex
fn vertex(vertex_in: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let vertex = decompress_vertex(vertex_in, vertex_in.instance_index);
    let world_from_local = mesh_functions::get_world_from_local(vertex_in.instance_index);
    stand(world_from_local, vertex.position);
    tinted = vec4<f32>(1.0);
    @if(VERTEX_UVS_A) {
        out.uv = vertex.uv;
    }
    @if(VERTEX_UVS_B) {
        out.uv_b = vertex.uv_b;
    }
    @if(VERTEX_COLORS) {
        tinted = vertex.color;
        out.color = vertex.color;
    }
    var uv = vec2<f32>(0.5);
    @if(VERTEX_UVS_A) {
        uv = vertex.uv;
    }
    read_weather();
    let placed = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));
    let world = vec4<f32>(sway(placed.xyz, uv), 1.0);
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
        @if(NORMAL_PREPASS_OR_DEFERRED_PREPASS && VERTEX_TANGENTS) {
            out.world_tangent = mesh_functions::mesh_tangent_local_to_world(world_from_local, vertex.tangent, vertex_in.instance_index);
        }
        @if(MOTION_VECTOR_PREPASS) {
            out.previous_world_position = world;
        }
    }
    @else {
        @if(VERTEX_NORMALS) {
            out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex_in.instance_index);
        }
        @if(VERTEX_TANGENTS) {
            out.world_tangent = mesh_functions::mesh_tangent_local_to_world(world_from_local, vertex.tangent, vertex_in.instance_index);
        }
    }
    @if(VERTEX_OUTPUT_INSTANCE_INDEX) {
        out.instance_index = vertex_in.instance_index;
    }
    return out;
}

@if(PREPASS_PIPELINE && PREPASS_FRAGMENT)
@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    clear_pixel(in.position.xy);
    return prepass_output(in, is_front);
}
@elif(PREPASS_PIPELINE)
@fragment
fn fragment(in: VertexOutput) {
    pbr_prepass_functions::prepass_sample_color_and_alpha_discard(in);
}
@else
@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    clear_pixel(in.position.xy);
    let world_from_local = mesh_functions::get_world_from_local(in.instance_index);
    let local_from_world = mesh_functions::get_local_from_world(in.instance_index);
    stand(world_from_local, (local_from_world * vec4<f32>(in.world_position.xyz, 1.0)).xyz);
    tinted = vec4<f32>(1.0);
    @if(VERTEX_COLORS) {
        tinted = in.color;
    }
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    let base = alpha_discard(
        pbr_input.material.flags,
        pbr_input.material.alpha_cutoff,
        pbr_input.material.base_color
    );
    own = vec4<f32>(base.rgb / max(tinted.rgb, vec3<f32>(0.001)), base.a);
    facing = pbr_input.N;
    eye_way = pbr_input.V;
    pbr_input.material.base_color = vec4<f32>(1.0, 1.0, 1.0, base.a);
    shine = lighting(pbr_input).rgb;
    lit = vec4<f32>(shine * base.rgb, base.a);
    read_weather();
    var uv = vec2<f32>(0.5);
    @if(VERTEX_UVS_A) {
        uv = in.uv;
    }
    var out: FragmentOutput;
    out.color = main_pass_post_lighting_processing(pbr_input, vfx(uv));
    return out;
}
