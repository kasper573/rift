// A floating body's part of the lid it keeps over the water it displaces: a mesh drawn into the
// depth alone, whatever of it lies under the level of the water raised to that level. The mesh's
// tag holds the level, the bits of its height in metres.

import bevy_pbr::render::forward_io::{Vertex, decompress_vertex};
import bevy_pbr::render::mesh_functions::{get_tag, get_world_from_local, mesh_position_local_to_world};
import bevy_pbr::render::view_transformations::position_world_to_clip;
@if(SKINNED)
import bevy_pbr::render::skinning::skin_model;

// How far over the water's level the lid lies, in metres: clear of the water's own depth.
const OVER_THE_WATER: f32 = 0.01;

@vertex
fn vertex(packed: Vertex) -> @builtin(position) vec4<f32> {
    let vertex = decompress_vertex(packed, packed.instance_index);
    @if(SKINNED)
    let world_from_local = skin_model(vertex.joint_indices, vertex.joint_weights, packed.instance_index);
    @else
    let world_from_local = get_world_from_local(packed.instance_index);
    var world = mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));
    let level = bitcast<f32>(get_tag(packed.instance_index));
    world.y = max(world.y, level + OVER_THE_WATER);
    return position_world_to_clip(world.xyz);
}

@fragment
fn fragment() -> @location(0) vec4<f32> {
    return vec4<f32>(0.0);
}
