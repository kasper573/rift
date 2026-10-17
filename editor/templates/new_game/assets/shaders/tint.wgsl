// Blends the site's image toward a color for the layer's whole duration, throbbing between
// nothing and the full amount `pulse` times a second when pulse is set. A site without an image
// tints a sheet in its shape.
fn vfx(uv: vec2<f32>) -> vec4<f32> {
    let pixel = sheet(site_uv(uv));
    let throb = select(1.0, 0.5 + 0.5 * sin(site.time * params.pulse * 6.2831853), params.pulse > 0.0);
    return vec4<f32>(mix(pixel.rgb, params.color.rgb, clamp(params.amount * throb, 0.0, 1.0)), pixel.a);
}
