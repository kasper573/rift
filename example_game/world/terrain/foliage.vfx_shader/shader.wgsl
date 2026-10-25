// Things strewn over the ground, flowers, ferns, whatever the cover names, in their own colours,
// leaning with the wind and parting round whoever walks through, by suppleness.
fn sway(world: vec3<f32>, tip: f32) -> vec3<f32> {
    let shift = (wind_lean(world.xz) + tread_push(world.xz)) * params.suppleness;
    return lean(world, tip, shift);
}

fn vfx(uv: vec2<f32>) -> vec4<f32> {
    return sample(uv);
}
