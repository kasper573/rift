// Blades that lean with the wind and part round whoever walks through them, shading from the
// ground's own colour at the root to the tip colour. suppleness scales how far they bend.
fn sway(world: vec3<f32>, tip: f32) -> vec3<f32> {
    let shift = (wind_lean(world.xz) + tread_push(world.xz)) * params.suppleness;
    return lean(world, tip, shift);
}

fn vfx(uv: vec2<f32>) -> vec4<f32> {
    let lit = sample(uv);
    let up = uv.y * uv.y;
    return vec4<f32>(mix(lit.rgb, params.tip.rgb * lit.rgb * 1.6, up), lit.a);
}
