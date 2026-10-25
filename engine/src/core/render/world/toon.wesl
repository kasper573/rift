import bevy_pbr::{
    render::{
        forward_io::{VertexOutput, FragmentOutput},
        mesh_view_bindings::view,
        view_transformations::depth_ndc_to_view_z,
    },
    prepass::utils as prepass_utils,
};

import engine::core::render::world::dither::dither_discard;
import engine::core::render::world::toon_shade::toon_color;

struct Toon {
    shown: f32,
    outlined: f32,
    flat: f32,
    padding: f32,
}

@group(3) @binding(100) var<uniform> toon: Toon;

const INK: f32 = 0.22;
// A neighbouring pixel nearer than this many metres lies on a nearer thing: its edge is inked.
const DEPTH_EDGE: f32 = 0.35;
// Neighbouring normals turned further apart than this crease.
const CREASE: f32 = 0.6;

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    dither_discard(in.position.xy, toon.shown);
    var color = toon_color(in, is_front, toon.flat);
    if toon.outlined > 0.5 && inked(in.position) {
        color = vec4<f32>(color.rgb * INK, color.a);
    }
    var out: FragmentOutput;
    out.color = color;
    return out;
}

// Whether the pixel lies on an edge of the picture: a neighbour nearer than it by a margin, or one
// at the same depth facing another way, counted on one side only so a crease is one pixel wide.
fn inked(frag_coord: vec4<f32>) -> bool {
@if(DEPTH_PREPASS && NORMAL_PREPASS) {
    let size = vec2<f32>(view.viewport.zw);
    let here = -depth_ndc_to_view_z(prepass_utils::prepass_depth(frag_coord, 0u));
    let normal = prepass_utils::prepass_normal(frag_coord, 0u);
    let steps = array<vec2<f32>, 4>(
        vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0), vec2<f32>(-1.0, 0.0), vec2<f32>(0.0, -1.0)
    );
    for (var i = 0u; i < 4u; i++) {
        let at = frag_coord.xy + steps[i];
        if any(at < vec2<f32>(0.0)) || any(at >= size) {
            continue;
        }
        let beside = vec4<f32>(at, frag_coord.zw);
        let there = -depth_ndc_to_view_z(prepass_utils::prepass_depth(beside, 0u));
        if here - there > DEPTH_EDGE {
            return true;
        }
        let forward = i < 2u;
        if forward && abs(here - there) <= DEPTH_EDGE
            && dot(normal, prepass_utils::prepass_normal(beside, 0u)) < CREASE {
            return true;
        }
    }
}
    return false;
}
