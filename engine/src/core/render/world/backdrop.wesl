// The far end of every ray the camera looks along, drawn in the prepass alone: it tells the
// frames' smoothing how the sky moved across the screen as the camera turned, and draws nothing.

import bevy_pbr::render::mesh_view_bindings::view;
import bevy_pbr::render::view_transformations::position_world_to_clip;
@if(PREPASS_PIPELINE)
import bevy_pbr::prepass::io::FragmentOutput;
@if(MOTION_VECTOR_PREPASS)
import bevy_pbr::render::pbr_prepass_functions::calculate_motion_vector;

struct Backdrop {
    @builtin(position) position: vec4<f32>,
    @location(0) way: vec3<f32>,
}

@if(PREPASS_PIPELINE)
@vertex
fn vertex(@location(0) position: vec3<f32>) -> Backdrop {
    var out: Backdrop;
    out.way = position;
    var clip = position_world_to_clip(view.world_position + position);
    clip.z = 0.0;
    out.position = clip;
    return out;
}
@else
@vertex
fn vertex(@location(0) position: vec3<f32>) -> Backdrop {
    var out: Backdrop;
    out.way = position;
    // Past the edge of what is drawn: the main pass has nothing of the backdrop to fill.
    out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    return out;
}

@if(PREPASS_PIPELINE && PREPASS_FRAGMENT)
@fragment
fn fragment(in: Backdrop) -> FragmentOutput {
    var out: FragmentOutput;
    @if(NORMAL_PREPASS) {
        out.normal = vec4<f32>(0.5, 0.5, 1.0, 1.0);
    }
    @if(MOTION_VECTOR_PREPASS) {
        // A way with no place: only the camera's turning moves it over the screen.
        let far = vec4<f32>(normalize(in.way), 0.0);
        out.motion_vector = calculate_motion_vector(far, far);
    }
    return out;
}

@if(!PREPASS_PIPELINE)
@fragment
fn fragment(in: Backdrop) -> @location(0) vec4<f32> {
    discard;
}
