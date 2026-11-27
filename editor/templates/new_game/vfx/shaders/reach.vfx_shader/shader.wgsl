// How far a ranged skill reaches from its caster: a thin dashed ring at that distance that
// turns slowly, so the player sees whether the cast lands at once or walks them in first.

fn fx(i: FxIn) -> vec4<f32> {
    let p = vec2<f32>(i.local.x * i.size.x, i.local.z * i.size.z);
    let r = length(p);
    let d = r - frame.site.x;
    let px = max(fwidth(d), 1e-4);
    let line = 1.0 - smoothstep(0.01, 0.01 + px * 1.5, abs(d));
    let arc = atan2(p.x, p.y) * frame.site.x;
    let dash = smoothstep(0.35, 0.4, fract(arc / 0.3 + i.age * 0.15)) * (1.0 - smoothstep(0.9, 0.95, fract(arc / 0.3 + i.age * 0.15)));
    let open = clamp(i.age / 0.15, 0.0, 1.0);
    return vec4<f32>(params.color.rgb * 1.6, line * dash * open * params.strength);
}
