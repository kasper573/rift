// A column of light up a tube: bright at its root, streaked with light running up it, fading
// toward the top and at its sides.

fn fx(i: FxIn) -> vec4<f32> {
    let up = i.uv.y;
    let facing = abs(dot(i.normal, i.view));
    let n = noise2(vec2<f32>(i.uv.x * params.streaks, up * 2.0 - i.age * params.speed + i.seed));
    let streak = smoothstep(0.45, 0.8, n);
    let height = (1.0 - smoothstep(0.0, params.top, up)) * smoothstep(0.0, 0.04, up);
    let swell = smoothstep(0.0, 0.15, i.life) * (1.0 - smoothstep(0.65, 1.0, i.life));
    let a = height * swell * (0.25 + 0.75 * streak) * (0.3 + 0.7 * facing);
    return vec4<f32>(params.color.rgb * (1.0 + streak), a);
}
