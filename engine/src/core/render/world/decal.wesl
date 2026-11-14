import bevy_pbr::render::{
    forward_io::VertexOutput,
    mesh_view_bindings::globals,
};

struct Paint {
    color: vec4<f32>,
    look: u32,
    width: f32,
    dashes: f32,
}

@group(3) @binding(0) var<uniform> paint: Paint;

const RING: u32 = 0u;
const TAU: f32 = 6.2831855;
const SHADE: f32 = 0.6;

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
        let rim = smoothstep(0.6, 0.68, reach) * (1.0 - smoothstep(0.76, 0.84, reach));
        let lit = paint.color.a * max(0.45 * pow(1.0 - clamp(reach, 0.0, 1.0), 1.6), rim);
        let shade = paint.color.a * SHADE * smoothstep(0.2, 0.6, reach) * (1.0 - smoothstep(0.82, 1.0, reach));
        let cover = lit + shade * (1.0 - lit);
        return vec4<f32>(paint.color.rgb * lit / max(cover, 0.0001), cover);
    }
    return vec4<f32>(paint.color.rgb, paint.color.a * alpha);
}
