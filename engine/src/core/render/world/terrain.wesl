// The ground's heights as a shader reads them: a picture a texel per sample, `samples_per_metre`
// to the metre from `origin`, blended between the four samples round a point, so a mesh settled
// on them lies on the ground the way everything else drawn on it does.
struct Ground {
    origin: vec2<f32>,
    samples_per_metre: f32,
    lift: f32,
}

fn ground_height(heights: texture_2d<f32>, ground: Ground, at: vec2<f32>) -> f32 {
    let last = vec2<f32>(textureDimensions(heights)) - 1.0;
    let sample = clamp((at - ground.origin) * ground.samples_per_metre, vec2<f32>(0.0), last);
    let corner = floor(sample);
    let t = sample - corner;
    let near = vec2<i32>(corner);
    let far = vec2<i32>(min(corner + 1.0, last));
    let top = mix(sample_height(heights, near.x, near.y), sample_height(heights, far.x, near.y), t.x);
    let bottom = mix(sample_height(heights, near.x, far.y), sample_height(heights, far.x, far.y), t.x);
    return mix(top, bottom, t.y);
}

fn sample_height(heights: texture_2d<f32>, column: i32, row: i32) -> f32 {
    return textureLoad(heights, vec2<i32>(column, row), 0).r;
}
