// Ripples the site's image sideways, settling as the layer ends. Amplitude is a fraction of the
// site. A site without an image ripples a sheet in its shape.
fn vfx(uv: vec2<f32>) -> vec4<f32> {
    let at = site_uv(uv);
    let wobble = sin(at.y * params.frequency + site.time * 14.0) * params.amplitude * (1.0 - site.progress);
    return sheet(vec2<f32>(at.x + wobble, at.y));
}
