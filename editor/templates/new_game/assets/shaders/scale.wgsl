// Grows or shrinks the site's image about its centre from one scale to another. A site without
// an image scales a sheet in its shape.
fn vfx(uv: vec2<f32>) -> vec4<f32> {
    let scale = max(mix(params.start, params.end, site.progress), 0.01);
    let at = (site_uv(uv) - 0.5) / scale + 0.5;
    return sheet(at);
}
