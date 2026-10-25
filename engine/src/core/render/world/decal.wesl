import bevy_pbr::render::{
    forward_io::{Vertex, VertexOutput, decompress_vertex},
    mesh_functions,
    mesh_view_bindings::globals,
    view_transformations::position_world_to_clip,
};
import engine::core::render::world::terrain::{Ground, ground_height};

struct Paint {
    color: vec4<f32>,
    look: u32,
    width: f32,
    dashes: f32,
}

@group(3) @binding(0) var<uniform> paint: Paint;
@group(3) @binding(1) var<uniform> ground: Ground;
@group(3) @binding(2) var heights: texture_2d<f32>;

const RING: u32 = 0u;
const TAU: f32 = 6.2831855;

@vertex
fn vertex(vertex_in: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let vertex = decompress_vertex(vertex_in, vertex_in.instance_index);
    let world_from_local = mesh_functions::get_world_from_local(vertex_in.instance_index);
    var world = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(vertex.position, 1.0));
    world.y = ground_height(heights, ground, world.xz) + ground.lift;
    out.world_position = world;
    out.position = position_world_to_clip(world.xyz);
    out.world_normal = vec3<f32>(0.0, 1.0, 0.0);
    @if(VERTEX_UVS_A) {
        out.uv = vertex.uv;
    }
    @if(VERTEX_OUTPUT_INSTANCE_INDEX) {
        out.instance_index = vertex_in.instance_index;
    }
    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    var uv = vec2<f32>(0.5);
    @if(VERTEX_UVS_A) {
        uv = in.uv;
    }
    let from_center = (uv - 0.5) * 2.0;
    let reach = length(from_center);
    var alpha = 0.0;
    if paint.look == RING {
        let inner = 1.0 - paint.width;
        let band = smoothstep(inner - 0.06, inner, reach) * (1.0 - smoothstep(0.94, 1.0, reach));
        var dash = 1.0;
        if paint.dashes > 0.0 {
            let angle = atan2(from_center.y, from_center.x) + globals.time * 1.5;
            dash = step(0.5, fract(angle / TAU * paint.dashes));
        }
        alpha = band * dash;
    } else {
        alpha = pow(1.0 - clamp(reach, 0.0, 1.0), 1.6);
    }
    return vec4<f32>(paint.color.rgb, paint.color.a * alpha);
}
