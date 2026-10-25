import bevy_pbr::{
    render::pbr_prepass_functions,
    prepass::io as prepass_io,
};

import engine::core::render::world::dither::{dither_discard, fade_visibility};
import engine::core::render::world::prepass_write::prepass_output;

struct Toon {
    focus: vec2<f32>,
    shown: f32,
    outlined: f32,
    flat: f32,
    focus_radius: f32,
    padding: vec2<f32>,
}

@group(3) @binding(100) var<uniform> toon: Toon;

// The fade's dither belongs to the picture a camera takes, whose prepass writes normals: a pixel
// the main pass discards must not write depth either, or it would hide what lies behind it. A
// shadow map writes depth alone and never fades: what fades still stands there and keeps its
// whole shadow, and the clearing is a circle of the picture, meaningless in the map. Where the
// device cannot leave depth unclipped, bevy runs the fragment stage for the map too, so the pass
// is told apart by what it writes, not by whether it has a fragment stage.
@if(NORMAL_PREPASS)
fn fade(pixel: vec2<f32>) {
    dither_discard(pixel, fade_visibility(pixel, toon.shown, toon.focus, toon.focus_radius));
}
@else
fn fade(pixel: vec2<f32>) {}

@if(PREPASS_FRAGMENT)
@fragment
fn fragment(
    in: prepass_io::VertexOutput,
    @builtin(front_facing) is_front: bool,
) -> prepass_io::FragmentOutput {
    fade(in.position.xy);
    return prepass_output(in, is_front);
}
@else
@fragment
fn fragment(in: prepass_io::VertexOutput) {
    fade(in.position.xy);
    pbr_prepass_functions::prepass_sample_color_and_alpha_discard(in);
}
