// A prop drawn from the arenas in a world material that no shader of the game's draws: the
// vertex as bevy's material shaders read it, of the instance its command draws as its
// `instance_index`th, carrying on to the pixel how far the instance shows. The pixel clears as
// the instance says, then is lit, or written to the prepass, as bevy's material shaders do it,
// reading bevy's mesh data at the first entry, which stands for every instance alike.

@if(PREPASS_PIPELINE)
import bevy_pbr::prepass::io::{Vertex, VertexOutput, FragmentOutput};
@else
import bevy_pbr::render::forward_io::{Vertex, VertexOutput, FragmentOutput};
@if(PREPASS_PIPELINE)
import bevy_pbr::render::pbr_prepass_functions;
@if(PREPASS_PIPELINE && PREPASS_FRAGMENT)
import engine::core::render::world::prepass_write::prepass_output;
@if(!PREPASS_PIPELINE)
import engine::core::render::world::shade::lit_color;
import bevy_pbr::render::view_transformations::position_world_to_clip;
import engine::core::render::world::gpu::instance::{clear_instance, placement, shown_of, turned, world_of};

// Bevy's vertex output, in the places bevy's own shaders read it from, and how far the instance
// shows, which no shader of bevy's reads.
@if(!PREPASS_PIPELINE)
struct InstancedOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
    @if(VERTEX_UVS_A)
    @location(2) uv: vec2<f32>,
    @if(VERTEX_COLORS)
    @location(5) color: vec4<f32>,
    @if(VERTEX_OUTPUT_INSTANCE_INDEX)
    @location(6) @interpolate(flat) instance_index: u32,
    @location(10) @interpolate(flat) shown: vec2<f32>,
}

@if(PREPASS_PIPELINE)
struct InstancedOutput {
    @builtin(position) position: vec4<f32>,
    @if(VERTEX_UVS_A)
    @location(0) uv: vec2<f32>,
    @if(NORMAL_PREPASS_OR_DEFERRED_PREPASS)
    @location(2) world_normal: vec3<f32>,
    @location(4) world_position: vec4<f32>,
    @if(MOTION_VECTOR_PREPASS)
    @location(5) previous_world_position: vec4<f32>,
    @if(UNCLIPPED_DEPTH_ORTHO_EMULATION)
    @location(6) unclipped_depth: f32,
    @if(VERTEX_OUTPUT_INSTANCE_INDEX)
    @location(7) instance_index: u32,
    @if(VERTEX_COLORS)
    @location(8) color: vec4<f32>,
    @location(10) @interpolate(flat) shown: vec2<f32>,
}

@vertex
fn vertex(in: Vertex) -> InstancedOutput {
    let placed = placement(in.instance_index);
    let world = world_of(placed, in.position);
    var out: InstancedOutput;
    out.position = position_world_to_clip(world);
    out.world_position = vec4<f32>(world, 1.0);
    @if(!PREPASS_PIPELINE || NORMAL_PREPASS_OR_DEFERRED_PREPASS) {
        out.world_normal = normalize(turned(placed.turn, in.normal));
    }
    @if(VERTEX_UVS_A) {
        out.uv = in.uv;
    }
    @if(VERTEX_COLORS) {
        out.color = in.color;
    }
    @if(VERTEX_OUTPUT_INSTANCE_INDEX) {
        out.instance_index = 0u;
    }
    @if(PREPASS_PIPELINE && UNCLIPPED_DEPTH_ORTHO_EMULATION) {
        out.unclipped_depth = out.position.z;
        out.position.z = min(out.position.z, 1.0);
    }
    @if(PREPASS_PIPELINE && MOTION_VECTOR_PREPASS) {
        out.previous_world_position = out.world_position;
    }
    out.shown = shown_of(in.instance_index);
    return out;
}

@if(!PREPASS_PIPELINE)
@fragment
fn fragment(in: InstancedOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    clear_instance(in.position.xy, in.shown);
    var out: FragmentOutput;
    out.color = lit_color(bevy_output(in), is_front);
    return out;
}

@if(PREPASS_PIPELINE && PREPASS_FRAGMENT)
@fragment
fn fragment(in: InstancedOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    clear_instance(in.position.xy, in.shown);
    return prepass_output(bevy_output(in), is_front);
}

// Depth alone is a light's shadow map, which a cleared prop keeps whole.
@if(PREPASS_PIPELINE && !PREPASS_FRAGMENT)
@fragment
fn fragment(in: InstancedOutput) {
    pbr_prepass_functions::prepass_sample_color_and_alpha_discard(bevy_output(in));
}

// `in` as bevy's shaders read a vertex's output.
fn bevy_output(in: InstancedOutput) -> VertexOutput {
    var out: VertexOutput;
    out.position = in.position;
    out.world_position = in.world_position;
    @if(!PREPASS_PIPELINE || NORMAL_PREPASS_OR_DEFERRED_PREPASS) {
        out.world_normal = in.world_normal;
    }
    @if(VERTEX_UVS_A) {
        out.uv = in.uv;
    }
    @if(VERTEX_COLORS) {
        out.color = in.color;
    }
    @if(VERTEX_OUTPUT_INSTANCE_INDEX) {
        out.instance_index = in.instance_index;
    }
    @if(PREPASS_PIPELINE && UNCLIPPED_DEPTH_ORTHO_EMULATION) {
        out.unclipped_depth = in.unclipped_depth;
    }
    @if(PREPASS_PIPELINE && MOTION_VECTOR_PREPASS) {
        out.previous_world_position = in.previous_world_position;
    }
    return out;
}
