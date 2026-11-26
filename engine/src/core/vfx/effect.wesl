// The header of an effect shader: a shape, a coat or a patch of ground of a vfx, drawn in the
// world. The shader's `fx` is handed where the pixel lies, how its face turns and where the layer
// is in its run, and returns the colour drawn there in linear light, above 1 glowing, and how much
// of what lies behind it it covers.
import bevy_pbr::render::forward_io::VertexOutput;
import bevy_pbr::render::mesh_view_bindings::{globals, view};
import bevy_pbr::render::view_transformations::{frag_coord_to_ndc, position_ndc_to_world};
@if(DEPTH_PREPASS)
import bevy_pbr::prepass::utils::prepass_depth;
import engine::core::vfx::render::effect_frame::{Fx, frame};

struct FxIn {
    // The shape's own uvs: across its sweep and its width.
    uv: vec2<f32>,
    world: vec3<f32>,
    // Where the pixel lies in the layer's own frame, its extent spanning -0.5 to 0.5.
    local: vec3<f32>,
    normal: vec3<f32>,
    // The way toward the eye.
    view: vec3<f32>,
    // How far through its run the layer is, 0 to 1.
    life: f32,
    // Seconds the layer has played, counted on through every cycle of a vfx that loops.
    age: f32,
    seed: f32,
    front: bool,
    pixel: vec4<f32>,
    // The layer's extent in metres: a coat's part, a ground's area.
    size: vec3<f32>,
    // What a coat is drawn on: 1 a model's surface, 2 the ground; 0 for a shape of its own.
    on: f32,
}

@group(3) @binding(1) var<uniform> params: Params;

// How far the surface behind this fragment lies beyond it, in metres; far off where none.
fn depth_gap(pixel: vec4<f32>, world: vec3<f32>) -> f32 {
    @if(DEPTH_PREPASS) {
        let depth = prepass_depth(pixel, 0u);
        if depth > 0.0 {
            let behind = position_ndc_to_world(vec3<f32>(frag_coord_to_ndc(pixel).xy, depth));
            return distance(behind, view.world_position) - distance(world, view.world_position);
        }
    }
    return 1000.0;
}

fn unrotate(q: vec4<f32>, v: vec3<f32>) -> vec3<f32> {
    let p = vec4<f32>(-q.xyz, q.w);
    let t = 2.0 * cross(p.xyz, v);
    return v + p.w * t + cross(p.xyz, t);
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    var i: FxIn;
    @if(VERTEX_UVS_A)
    i.uv = in.uv;
    i.world = in.world_position.xyz;
    i.local = unrotate(frame.rotation, i.world - frame.origin.xyz) / max(frame.scale.xyz, vec3<f32>(1e-4));
    i.normal = normalize(in.world_normal) * select(-1.0, 1.0, front);
    i.view = normalize(view.world_position - i.world);
    i.life = frame.life.x;
    i.age = frame.life.y;
    i.seed = frame.scale.w;
    i.front = front;
    i.pixel = in.position;
    i.size = frame.scale.xyz;
    i.on = frame.extra.y;
    let c = fx(i);
    var a = clamp(c.a, 0.0, 1.0) * (1.0 - frame.extra.z);
    if frame.mask.w > 0.0 {
        a = a * smoothstep(frame.mask.w, frame.mask.w * 0.45, distance(i.world, frame.mask.xyz));
    }
    if frame.life.w > 0.0 {
        a = a * clamp(depth_gap(in.position, i.world) / frame.life.w, 0.0, 1.0);
    }
    @if(FX_ADD) {
        return vec4<f32>(max(c.rgb, vec3<f32>(0.0)) * a, 0.0);
    }
    return vec4<f32>(max(c.rgb, vec3<f32>(0.0)), a);
}
