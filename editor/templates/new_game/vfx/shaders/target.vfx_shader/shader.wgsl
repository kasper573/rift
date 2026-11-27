// The mark on each actor an aimed skill would land on: a rim of light round its silhouette in the
// aim's own colours, blue for a friendly cast and red for a hostile one, that snaps on with the
// aim and breathes while it holds, faster when hostile.

fn fx(i: FxIn) -> vec4<f32> {
    let hostile = clamp(params.hostile, 0.0, 1.0);
    let color = mix(params.color, params.hostile_color, hostile);
    let open = 1.0 - pow(1.0 - clamp(i.age / 0.12, 0.0, 1.0), 3.0);
    let rim = pow(1.0 - abs(dot(i.normal, i.view)), 2.5);
    let breath = 0.8 + 0.2 * sin(i.age * mix(3.0, 5.0, hostile));
    let a = clamp(rim * breath * open * params.strength, 0.0, 1.0);
    return vec4<f32>(color.rgb * 2.0, a);
}
