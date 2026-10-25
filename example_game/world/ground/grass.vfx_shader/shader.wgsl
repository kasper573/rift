// A carpet of blades that lean with the wind and part round whoever walks through them, the
// blades (the cover's first piece) shading from their own colour at the root toward the tip
// colour; anything else strewn among them, flowers say, keeps its colours. suppleness scales how
// far it all bends.
fn sway(world: vec3<f32>, tip: f32) -> vec3<f32> {
    let shift = (wind_lean(world.xz) + tread_push(world.xz)) * params.suppleness;
    return lean(world, tip, shift);
}

fn vfx(uv: vec2<f32>) -> vec4<f32> {
    let lit = sample(uv);
    if piece() != 0u {
        return lit;
    }
    let up = uv.y * uv.y;
    return vec4<f32>(mix(lit.rgb, mix(lit.rgb, params.tip.rgb, 0.7), up), lit.a);
}
