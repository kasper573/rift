// A soft pool of light on the ground, stirred by noise, swelling in and fading out.

fn fx(i: FxIn) -> vec4<f32> {
    let p = i.local.xz * 2.0;
    let r = length(p);
    let n = fbm2(p * 2.5 + vec2<f32>(i.age * 0.4 * params.stir, -i.age * 0.3 * params.stir) + i.seed);
    let pool = 1.0 - smoothstep(params.edge * 0.4, 1.0, r + (n - 0.5) * 0.3);
    let swell = smoothstep(0.0, 0.2, i.life) * (1.0 - smoothstep(0.6, 1.0, i.life));
    return vec4<f32>(params.color.rgb, pool * swell * (0.6 + 0.4 * n));
}
