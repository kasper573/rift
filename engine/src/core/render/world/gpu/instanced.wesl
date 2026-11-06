// The vertex stage of a prop drawn from the arenas in a world material that no shader of the
// game's draws: the vertex as bevy's material shaders read it, of the instance its command draws
// as its `instance_index`th. The fragment reads bevy's mesh data at the first entry, which stands
// for every instance alike.

@if(PREPASS_PIPELINE)
import bevy_pbr::prepass::io::{Vertex, VertexOutput};
@else
import bevy_pbr::render::forward_io::{Vertex, VertexOutput};
import bevy_pbr::render::view_transformations::position_world_to_clip;
import engine::core::render::world::gpu::instance::{placement, turned, world_of};

@vertex
fn vertex(in: Vertex) -> VertexOutput {
    let placed = placement(in.instance_index);
    let world = world_of(placed, in.position);
    var out: VertexOutput;
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
    return out;
}
