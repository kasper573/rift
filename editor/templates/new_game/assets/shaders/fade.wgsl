// Fades the site's image toward a color as the layer plays out. A site without an image fades a
// sheet in its shape.
fn vfx(uv: vec2<f32>) -> vec4<f32> {
    let pixel = sheet(site_uv(uv));
    let faded = clamp(site.progress * params.strength, 0.0, 1.0);
    return vec4<f32>(mix(pixel.rgb, params.color.rgb, faded), pixel.a);
}
