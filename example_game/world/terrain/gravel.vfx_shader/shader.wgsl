// Stones strewn over the painted cells, as the prop's own shape, in the ground's colour shaded
// darker at their base. They stand firm in the wind.
fn sway(world: vec3<f32>, tip: f32) -> vec3<f32> {
    return world;
}

fn vfx(uv: vec2<f32>) -> vec4<f32> {
    let lit = sample(uv);
    return vec4<f32>(lit.rgb * mix(params.shade, 1.0, tip()), lit.a);
}
