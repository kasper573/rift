// Jolts the site's image about, calming as the layer ends. Amplitude is a fraction of the site.
// A site without an image shakes a sheet in its shape.
fn vfx(uv: vec2<f32>) -> vec4<f32> {
    let jolt = vec2<f32>(sin(site.time * 43.0), cos(site.time * 37.0)) * params.amplitude * (1.0 - site.progress);
    return sheet(site_uv(uv) + jolt);
}
