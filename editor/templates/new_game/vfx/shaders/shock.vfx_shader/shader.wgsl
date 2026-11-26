// A ring of force racing outward: bright at its leading edge, breaking up as it thins.

fn fx(i: FxIn) -> vec4<f32> {
    let v = i.uv.y;
    let lead = smoothstep(0.0, 0.85, v) * (1.0 - smoothstep(0.9, 1.0, v));
    let n = noise2(vec2<f32>(i.uv.x * 40.0, i.age * 6.0 + i.seed));
    let thin = 1.0 - i.life;
    let a = lead * lead * thin * mix(1.0, n, params.breakup);
    return vec4<f32>(params.color.rgb, a);
}
