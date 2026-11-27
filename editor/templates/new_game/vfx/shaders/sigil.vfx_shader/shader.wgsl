// A glowing circle drawn on the ground: rims, ticks and turning glyph arcs, inked in from the
// centre out and fading as the spell ends.

fn fx(i: FxIn) -> vec4<f32> {
    let p = i.local.xz * 2.0;
    let round = length(p);
    let boxy = max(abs(p.x), abs(p.y));
    let r = mix(round, boxy, params.square);
    let angle = atan2(p.y, p.x) / TAU + 0.5;
    let ink = smoothstep(0.0, 0.35, i.life) * 1.25;
    let shown = 1.0 - smoothstep(ink - 0.1, ink, r);
    let fade = 1.0 - smoothstep(0.7, 1.0, i.life);
    let line = 0.012;
    var a = 0.0;
    a = a + window(r, 0.94, 0.96, line);
    a = a + window(r, 0.86, 0.87, line) * 0.8;
    a = a + window(r, 0.55, 0.565, line) * 0.7;
    // Ticks between the outer rims.
    let ticks = step(0.75, fract(angle * 48.0)) * window(r, 0.87, 0.94, 0.004);
    a = a + ticks * 0.8;
    // Glyph arcs that turn the other way.
    let turn = fract(angle + i.age * params.spin * 0.1);
    let glyphs = step(0.35, fract(turn * 7.0)) * window(r, 0.62, 0.8, 0.004);
    let cut = step(0.5, noise2(vec2<f32>(floor(turn * 56.0), floor(r * 18.0)) + i.seed));
    a = a + glyphs * cut * 0.5;
    // A soft wash inside.
    a = a + (1.0 - smoothstep(0.0, 0.96, r)) * 0.12;
    return vec4<f32>(params.color.rgb, clamp(a, 0.0, 1.0) * shown * fade * step(r, 1.0));
}
