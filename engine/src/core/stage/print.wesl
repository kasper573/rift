// A card's surface on the stage. Its triangles are the interface's own, so their colours are
// read and blended the way the interface does it: texture and vertex colour multiplied as
// they are written, with the colour already weighed by how opaque it is. Lit by nothing, a
// pixel comes out exactly as the interface would have drawn it. The lamp's light is added by
// Bevy's own lighting, over the share of the light that falls evenly.

import bevy_pbr::render::{
    forward_io::{VertexOutput, FragmentOutput},
    pbr_fragment::pbr_input_from_standard_material,
};
import engine::core::render::world::shade::lighting;

struct PrintLight {
    clip: vec4<f32>,
    lamp: vec3<f32>,
    even: f32,
    foil: f32,
}

@group(3) @binding(100) var print_texture: texture_2d<f32>;
@group(3) @binding(101) var print_sampler: sampler;
@group(3) @binding(102) var<uniform> light: PrintLight;
@group(3) @binding(103) var foil_texture: texture_2d<f32>;
@group(3) @binding(104) var foil_sampler: sampler;

const PI: f32 = 3.1415927;
const TAU: f32 = 6.2831855;
// How far the light's angle on a foil turns from one run of the rainbow to the next.
const FOIL_PERIOD: f32 = 0.6;
// How far a foil's pattern shifts its colours, as a share of that.
const FOIL_PATTERN: f32 = 0.15;
// The least and most of the print's own light a foil's colours let through, and how much light
// of their own they add.
const FOIL_TINT: vec2<f32> = vec2<f32>(0.75, 1.5);
const FOIL_GLOW: f32 = 0.1;

fn linear_from_gamma(srgb: vec3<f32>) -> vec3<f32> {
    let low = srgb / vec3<f32>(12.92);
    let high = pow((srgb + vec3<f32>(0.055)) / vec3<f32>(1.055), vec3<f32>(2.4));
    return select(high, low, srgb < vec3<f32>(0.04045));
}

fn gamma_from_linear(rgb: vec3<f32>) -> vec3<f32> {
    let low = rgb * vec3<f32>(12.92);
    let high = vec3<f32>(1.055) * pow(rgb, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    return select(high, low, rgb < vec3<f32>(0.0031308));
}

fn rainbow(hue: f32) -> vec3<f32> {
    return 0.5 + 0.5 * cos(TAU * (hue + vec3<f32>(0.0, 0.33, 0.67)));
}

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) is_front: bool) -> FragmentOutput {
    let sampled = textureSample(print_texture, print_sampler, in.uv);
    let pattern = textureSample(foil_texture, foil_sampler, in.uv_b).r;
    var pbr_input = pbr_input_from_standard_material(in, is_front);
    let at = in.uv_b;
    if at.x < light.clip.x || at.y < light.clip.y || at.x >= light.clip.z || at.y >= light.clip.w {
        discard;
    }
    let painted = vec4<f32>(gamma_from_linear(sampled.rgb), sampled.a) * in.color;
    let cover = painted.a;
    let ink = linear_from_gamma(painted.rgb);
    let flat = ink / max(cover, 0.0001);

    pbr_input.material.base_color = vec4<f32>(flat, 1.0);
    var lit = flat * light.even + lighting(pbr_input).rgb;

    if light.foil > 0.0 {
        // A foil is ruled with fine grooves that run aslant across the card. The lamp's light
        // comes off them split into its colours, by how far the surface is turned across the
        // grooves from mirroring the lamp to the eye, and comes round again every so far.
        let to_lamp = normalize(light.lamp - in.world_position.xyz);
        let half_way = normalize(to_lamp + pbr_input.V);
        let rightward = normalize(in.world_tangent.xyz);
        let upward = cross(normalize(in.world_normal), rightward);
        let turned = dot(half_way, rightward - upward) * 0.70710678;
        let run = fract(turned / FOIL_PERIOD + (pattern - 0.5) * FOIL_PATTERN) / light.foil;
        if run < 1.0 {
            let shows = pow(sin(PI * run), 2.0) * saturate((1.0 - light.even) * 4.0);
            let colours = rainbow(run);
            let tint = mix(vec3<f32>(FOIL_TINT.x), vec3<f32>(FOIL_TINT.y), colours);
            lit = mix(lit, lit * tint + colours * FOIL_GLOW, shows);
        }
    }

    var out: FragmentOutput;
    out.color = vec4<f32>(lit * cover, cover);
    return out;
}
