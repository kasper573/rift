// A plant swaying with the wind by `suppleness`, rooted where it stands and leaning the more the
// higher up it grows, in its own colours.
fn sway(world: vec3<f32>, uv: vec2<f32>) -> vec3<f32> {
    let height = max(world.y - origin().y, 0.0);
    let shift = wind_lean(world.xz) * params.suppleness * min(height * 0.3, 1.0);
    return world + vec3<f32>(shift.x, 0.0, shift.y);
}

fn vfx(uv: vec2<f32>) -> vec4<f32> {
    return sample(uv);
}
