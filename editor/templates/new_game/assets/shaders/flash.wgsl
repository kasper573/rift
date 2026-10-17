// A burst of color over the site's image that dies away as the layer plays out. A site without
// an image flashes a sheet in its shape.
fn vfx(uv: vec2<f32>) -> vec4<f32> {
    let pixel = sheet(site_uv(uv));
    let flash = params.strength * (1.0 - site.progress);
    return vec4<f32>(mix(pixel.rgb, params.color.rgb, clamp(flash, 0.0, 1.0)), pixel.a);
}
