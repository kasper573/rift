// The header of a shader that draws a material of a prop: the prop's model is its mesh, as the
// game stands it in the world. The shader says where a vertex stands (`sway`, given where the
// model puts it in the world and its uv, with the weather the engine tells it of) and what
// colour a pixel is (`vfx`). Both read where the point lies in the model's own metres (`local`),
// where the prop stands (`origin`), how much it is grown (`size`) and the point's vertex colour
// (`tint`); a vertex also reads the ways the screen's right and up run in the world
// (`screen_right`, `screen_up`), to turn a card to face the camera. A pixel also reads the
// material lit (`sample`), the light falling there on a white face (`light`), the material's own
// colour unlit (`albedo`), the way its face turns (`normal`), the way to the eye (`toward_eye`)
// and where it lies in the world (`position`), where the weather's helpers tell how wet it
// lies. What `vfx` paints is fogged and finished like every pixel of the world, and the prop
// fades where it hides the camera's subject like everything else. Every prop is drawn from the
// arenas, each vertex passing on to the pixel where its instance stands, how much it is grown and
// how it is turned.
@if(PREPASS_PIPELINE)
import bevy_pbr::prepass::io::{Vertex, UncompressedVertex, VertexOutput, FragmentOutput};
@else
import bevy_pbr::render::forward_io::{Vertex, UncompressedVertex, VertexOutput, FragmentOutput};
import bevy_pbr::render::{mesh_functions, view_transformations::position_world_to_clip};
import bevy_pbr::render::mesh_view_bindings::view;
@if(PREPASS_PIPELINE)
import bevy_pbr::render::pbr_prepass_functions;
@if(PREPASS_PIPELINE)
import engine::core::render::world::prepass_write::prepass_output;
@else
import bevy_pbr::render::{pbr_fragment::pbr_input_from_standard_material, pbr_functions::{alpha_discard, main_pass_post_lighting_processing}};
@if(!PREPASS_PIPELINE)
import engine::core::render::world::shade::lighting;

import engine::core::render::world::gpu::instance::{clear_instance, placement, shown_of, turned, world_of};

@if(PREPASS_PIPELINE)
import bevy_render::globals::Globals;
@else
import bevy_pbr::render::mesh_view_bindings::globals;
// The prepass binds the frame's globals right after the view, where the main pass binds them
// among the lights.
@if(PREPASS_PIPELINE)
@group(0) @binding(1) var<uniform> globals: Globals;

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
var<private> spot: vec3<f32>;

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

fn position() -> vec3<f32> {
    return spot;
}

// Where `world` lies in the model's own metres for an instance standing at `place.xyz`, grown
// `place.w` times and turned by the quaternion `turn`.
fn stand(place: vec4<f32>, turn: vec4<f32>, world: vec3<f32>) {
    let offset = world - place.xyz;
    let axis = -turn.xyz;
    let unturned = offset + 2.0 * cross(axis, cross(axis, offset) + turn.w * offset);
    at_local = unturned / place.w;
    stands = place.xyz;
    grown = place.w;
}

// The vertex output of a prop, with where its instance stands and how far it shows.
struct PropOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
    @location(5) @interpolate(flat) place: vec4<f32>,
    @location(6) @interpolate(flat) turn: vec4<f32>,
    @location(7) unclipped_depth: f32,
    // How far the instance shows and whether the eye stands inside it.
    @location(8) @interpolate(flat) shown: vec2<f32>,
}

// The vertex of the instance the command draws as its `instance_index`th, its normal and tangent
// turned as the instance is, bevy's mesh data read at the first entry, which stands for every
// instance alike.
@vertex
fn vertex(vertex_in: Vertex) -> PropOutput {
    let placed = placement(vertex_in.instance_index);
    var vertex: UncompressedVertex;
    vertex.instance_index = 0u;
    vertex.position = vertex_in.position;
    @if(VERTEX_UVS_A) {
        vertex.uv = vertex_in.uv;
    }
    @if(VERTEX_UVS_B) {
        vertex.uv_b = vertex_in.uv_b;
    }
    @if(VERTEX_NORMALS && (!PREPASS_PIPELINE || NORMAL_PREPASS_OR_DEFERRED_PREPASS)) {
        vertex.normal = turned(placed.turn, vertex_in.normal);
    }
    @if(VERTEX_TANGENTS && (!PREPASS_PIPELINE || NORMAL_PREPASS_OR_DEFERRED_PREPASS)) {
        vertex.tangent = vec4<f32>(turned(placed.turn, vertex_in.tangent.xyz), vertex_in.tangent.w);
    }
    @if(VERTEX_COLORS) {
        vertex.color = vertex_in.color;
    }
    let world = world_of(placed, vertex_in.position);
    let place = vec4<f32>(placed.at, placed.scale);
    stand(place, placed.turn, world);
    return prop_output(vertex_output(vertex, world), place, placed.turn, shown_of(vertex_in.instance_index));
}

// The prop output of `out`, its instance standing at `place.xyz`, grown `place.w` times, turned
// by `turn` and showing as far as `shown` says.
fn prop_output(out: VertexOutput, place: vec4<f32>, turn: vec4<f32>, shown: vec2<f32>) -> PropOutput {
    var prop: PropOutput;
    prop.position = out.position;
    prop.world_position = out.world_position;
    @if(!PREPASS_PIPELINE || NORMAL_PREPASS_OR_DEFERRED_PREPASS) {
        prop.world_normal = out.world_normal;
    }
    @if(VERTEX_UVS_A) {
        prop.uv = out.uv;
    }
    @if(VERTEX_COLORS) {
        prop.color = out.color;
    }
    prop.place = place;
    prop.turn = turn;
    prop.shown = shown;
    @if(PREPASS_PIPELINE && UNCLIPPED_DEPTH_ORTHO_EMULATION) {
        prop.unclipped_depth = out.unclipped_depth;
    }
    return prop;
}

// The vertex output bevy's functions read, of a prop's, its pixel standing where it says.
fn unpacked(prop: PropOutput) -> VertexOutput {
    stand(prop.place, prop.turn, prop.world_position.xyz);
    var out: VertexOutput;
    out.position = prop.position;
    out.world_position = prop.world_position;
    @if(!PREPASS_PIPELINE || NORMAL_PREPASS_OR_DEFERRED_PREPASS) {
        out.world_normal = prop.world_normal;
    }
    @if(VERTEX_UVS_A) {
        out.uv = prop.uv;
    }
    @if(VERTEX_COLORS) {
        out.color = prop.color;
    }
    @if(VERTEX_OUTPUT_INSTANCE_INDEX) {
        out.instance_index = 0u;
    }
    @if(PREPASS_PIPELINE && UNCLIPPED_DEPTH_ORTHO_EMULATION) {
        out.unclipped_depth = prop.unclipped_depth;
    }
    @if(PREPASS_PIPELINE && MOTION_VECTOR_PREPASS) {
        out.previous_world_position = prop.world_position;
    }
    return out;
}

// The vertex output of `vertex` standing at `placed` in the world, swayed.
fn vertex_output(vertex: UncompressedVertex, placed: vec3<f32>) -> VertexOutput {
    var out: VertexOutput;
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
    let world = vec4<f32>(sway(placed, uv), 1.0);
    out.world_position = world;
    out.position = position_world_to_clip(world.xyz);
    @if(PREPASS_PIPELINE) {
        @if(UNCLIPPED_DEPTH_ORTHO_EMULATION) {
            out.unclipped_depth = out.position.z;
            out.position.z = min(out.position.z, 1.0);
        }
        @if(NORMAL_PREPASS_OR_DEFERRED_PREPASS && VERTEX_NORMALS) {
            out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex.instance_index);
        }
        @if(NORMAL_PREPASS_OR_DEFERRED_PREPASS && VERTEX_TANGENTS) {
            out.world_tangent = mesh_functions::mesh_tangent_local_to_world(mesh_functions::get_world_from_local(vertex.instance_index), vertex.tangent, vertex.instance_index);
        }
        @if(MOTION_VECTOR_PREPASS) {
            out.previous_world_position = world;
        }
    }
    @else {
        @if(VERTEX_NORMALS) {
            out.world_normal = mesh_functions::mesh_normal_local_to_world(vertex.normal, vertex.instance_index);
        }
        @if(VERTEX_TANGENTS) {
            out.world_tangent = mesh_functions::mesh_tangent_local_to_world(mesh_functions::get_world_from_local(vertex.instance_index), vertex.tangent, vertex.instance_index);
        }
    }
    @if(VERTEX_OUTPUT_INSTANCE_INDEX) {
        out.instance_index = vertex.instance_index;
    }
    return out;
}

@if(PREPASS_PIPELINE && PREPASS_FRAGMENT)
@fragment
fn fragment(prop: PropOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    clear_instance(prop.position.xy, prop.shown);
    return prepass_output(unpacked(prop), is_front);
}
@elif(PREPASS_PIPELINE)
@fragment
fn fragment(prop: PropOutput) {
    pbr_prepass_functions::prepass_sample_color_and_alpha_discard(unpacked(prop));
}
@else
@fragment
fn fragment(prop: PropOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    clear_instance(prop.position.xy, prop.shown);
    return shade(unpacked(prop), is_front);
}

// The pixel the shader paints, where `stand` has said its piece stands.
@if(!PREPASS_PIPELINE)
fn shade(in: VertexOutput, is_front: bool) -> FragmentOutput {
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
    spot = in.world_position.xyz;
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
