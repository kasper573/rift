import bevy_pbr::{
    prepass::io::FragmentOutput,
    render::{
        pbr_prepass_functions::calculate_motion_vector,
        view_transformations::position_world_to_clip,
    },
};
import engine::core::render::world::gpu::patch::placed;

struct Vertex {
    @location(0) position: vec3<f32>,
}

struct Placed {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @if(UNCLIPPED_DEPTH_ORTHO_EMULATION)
    @location(1) unclipped_depth: f32,
}

@vertex
fn vertex(in: Vertex) -> Placed {
    let world = placed(in.position);
    var out: Placed;
    out.clip = position_world_to_clip(world.xyz);
    out.world_position = world;
    // Without the device leaving depth unclipped, what lies nearer the light than its shadow
    // map's near plane is clamped onto it here and given its true depth in the fragment.
    @if(UNCLIPPED_DEPTH_ORTHO_EMULATION) {
        out.unclipped_depth = out.clip.z;
        out.clip.z = min(out.clip.z, 1.0);
    }
    return out;
}

@if(PREPASS_FRAGMENT)
@fragment
fn fragment(in: Placed) -> FragmentOutput {
    var out: FragmentOutput;
    @if(NORMAL_PREPASS) {
        out.normal = vec4<f32>(0.5, 1.0, 0.5, 1.0);
    }
    // The ground stands still, so it moves on screen only as the camera does.
    @if(MOTION_VECTOR_PREPASS) {
        out.motion_vector = calculate_motion_vector(in.world_position, in.world_position);
    }
    @if(UNCLIPPED_DEPTH_ORTHO_EMULATION) {
        out.frag_depth = in.unclipped_depth;
    }
    return out;
}
