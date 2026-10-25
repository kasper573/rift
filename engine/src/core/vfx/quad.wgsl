// The header of a shader drawn on a quad: over the presented frame, on a patch of ground or on a
// board facing the camera. The quad's own image is sampled through `sample`.
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
}

@group(GROUP) @binding(0) var site_texture: texture_2d<f32>;
@group(GROUP) @binding(1) var site_sampler: sampler;
@group(GROUP) @binding(2) var<uniform> site: Site;
@group(GROUP) @binding(3) var<uniform> params: Params;

// The site's own image at a site uv, transparent outside it.
fn sample(at: vec2<f32>) -> vec4<f32> {
    let inside = f32(all(at >= vec2<f32>(0.0)) && all(at <= vec2<f32>(1.0)));
    let texel = mix(site.rect.xy, site.rect.zw, clamp(at, vec2<f32>(0.0), vec2<f32>(1.0)));
    return textureSample(site_texture, site_sampler, texel) * inside;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let painted = vfx(in.uv);
    return vec4<f32>(painted.rgb, painted.a * quad_guard(in.uv));
}
