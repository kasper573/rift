import bevy_pbr::render::forward_io::VertexOutput;

struct Paint {
    color: vec4<f32>,
    look: u32,
    width: f32,
    dashes: f32,
    time: f32,
}

@group(3) @binding(0) var<uniform> paint: Paint;

const RING: u32 = 0u;
const TAU: f32 = 6.2831855;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let from_center = (in.uv - 0.5) * 2.0;
    let reach = length(from_center);
    var alpha = 0.0;
    if paint.look == RING {
        let inner = 1.0 - paint.width;
        let band = smoothstep(inner - 0.06, inner, reach) * (1.0 - smoothstep(0.94, 1.0, reach));
        var dash = 1.0;
        if paint.dashes > 0.0 {
            let angle = atan2(from_center.y, from_center.x) + paint.time * 1.5;
            dash = step(0.5, fract(angle / TAU * paint.dashes));
        }
        alpha = band * dash;
    } else {
        alpha = pow(1.0 - clamp(reach, 0.0, 1.0), 1.6);
    }
    return vec4<f32>(paint.color.rgb, paint.color.a * alpha);
}
