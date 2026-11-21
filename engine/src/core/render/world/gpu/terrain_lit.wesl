import bevy_pbr::render::{
    mesh_types::{MESH_FLAGS_SHADOW_RECEIVER_BIT, MESH_FLAGS_SIGN_DETERMINANT_MODEL_3X3_BIT},
    mesh_view_bindings::{globals, view},
    pbr_functions::{calculate_view, main_pass_post_lighting_processing},
    pbr_lighting::perceptualRoughnessToRoughness,
    pbr_types::{pbr_input_new, STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT},
    view_transformations::position_world_to_clip,
};
@if(SCREEN_SPACE_AMBIENT_OCCLUSION)
import bevy_pbr::render::mesh_view_bindings::screen_space_ambient_occlusion_texture;
@if(SCREEN_SPACE_AMBIENT_OCCLUSION)
import bevy_pbr::ssao::utils::ssao_multibounce;
import engine::core::render::world::gpu::terrain::{
    ground, ground_point, paint_of, patch_point, shore_at, water_level, water_of,
};
import engine::core::render::world::shade::{caster_way, lighting};

// The terrains' pictures as one stack; for each terrain its colour and tile in the first row of
// `layers`, its picture's place in the stack in the second and how it looks soaked in the third;
// and how rough and how reflective the ground is dry.
@group(3) @binding(100) var pictures: texture_2d_array<f32>;
@group(3) @binding(101) var pictures_sampler: sampler;
@group(3) @binding(103) var layers: texture_2d<f32>;
@group(3) @binding(104) var<uniform> finish: vec2<f32>;
// What the props standing over the ground look like from above, a layer a band of their
// extent: their colour with how much of the ground they cover, and how wide they are as a share
// of the widest told.
@group(3) @binding(105) var canopy: texture_2d_array<f32>;
@group(3) @binding(106) var canopy_extent: texture_2d_array<f32>;
@group(3) @binding(107) var canopy_sampler: sampler;
// For each of the area's waters how fast it fades each colour, in the first row.
@group(3) @binding(108) var waters: texture_2d<f32>;

// The deepest ground under the water's level that may yet lie behind the waterline, where the
// water's map ends over it, in metres.
const BESIDE: f32 = 1.0;
// How far behind such a waterline the ground lies wet, in metres.
const LAPPED_BACK: f32 = 0.3;
// How deep under water wet ground has lost its gloss, in metres.
const GLOSS_DROWNS: f32 = 0.03;
// How high over a waterline the ground lies wet, in metres: a hand's height.
const LAPPED: f32 = 0.12;
// The light the water's surface gathers on a bed just under it, in times the light that falls
// there, how wide its net is in metres, and how deep the brightest of it lies: nearer the
// surface the light has not yet gathered, deeper it has spread again.
const NET_BRIGHTEST: f32 = 0.6;
const NET_SPAN: f32 = 0.55;
const NET_GATHERS: f32 = 0.35;

struct Vertex {
    @builtin(instance_index) instance: u32,
    @location(0) position: vec3<f32>,
}

struct Placed {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) @interpolate(flat) stride: u32,
    // How high the water's surface stands over the point, in metres.
    @location(3) water: f32,
}

// What the terrains lay at a point: their colour, and how they look soaked, the share of their
// colour left in x and how rough they lie in y, as far as rain wets them in z.
struct Laid {
    color: vec3<f32>,
    wet: vec3<f32>,
}

@vertex
fn vertex(in: Vertex) -> Placed {
    let point = ground_point(in.position, in.instance);
    let world = vec4<f32>(point.world, 1.0);
    let water = water_level(patch_point(in.position, in.instance).sample);
    return Placed(position_world_to_clip(point.world), world, point.normal, point.stride, water);
}

@fragment
fn fragment(in: Placed) -> @location(0) vec4<f32> {
    read_weather();
    var input = pbr_input_new();
    input.material.flags |= STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT;
    input.material.metallic = 0.0;
    input.material.perceptual_roughness = finish.x;
    input.material.reflectance = vec3<f32>(finish.y);
    input.flags = MESH_FLAGS_SHADOW_RECEIVER_BIT | MESH_FLAGS_SIGN_DETERMINANT_MODEL_3X3_BIT;
    input.frag_coord = in.clip;
    input.world_position = in.world_position;
    input.world_normal = normalize(in.world_normal);
    input.N = input.world_normal;
    input.is_orthographic = view.clip_from_view[3].w == 1.0;
    input.V = calculate_view(in.world_position, input.is_orthographic);
    // As bevy's standard material reads its occlusion: from the view's ambient occlusion under
    // a white base, before the ground paints its own colour in.
    @if(SCREEN_SPACE_AMBIENT_OCCLUSION) {
        let ssao_at = vec2<i32>(in.clip.xy - view.viewport.xy);
        let ssao = textureLoad(screen_space_ambient_occlusion_texture, ssao_at, 0i).r;
        input.diffuse_occlusion = min(vec3<f32>(1.0), ssao_multibounce(ssao, vec3<f32>(1.0)));
        let n_dot_v = max(dot(input.N, input.V), 0.0001);
        let roughness = perceptualRoughnessToRoughness(finish.x);
        input.specular_occlusion =
            saturate(pow(n_dot_v + ssao, exp2(-16.0 * roughness - 1.0)) - 1.0 + ssao);
    }
    let laid = painted(in.world_position.xz, in.stride);
    let under = in.water - in.world_position.y;
    // Ground under the water's level is under water only out from the waterline, and ground
    // lies wet only as far behind the waterline as the water laps: a point of the ground knows
    // the water's level no farther from the water than that.
    var sunk = under;
    var lapped = 1.0 - smoothstep(0.0, LAPPED, -under);
    if lapped > 0.0 && under < BESIDE {
        let offshore = shore_at(in.world_position.xz);
        if offshore <= 0.0 {
            sunk = 0.0;
            lapped *= 1.0 - smoothstep(0.0, LAPPED_BACK, -offshore);
        }
    }
    let soaked = max(wetness(in.world_position.xyz), lapped) * laid.wet.z;
    let over = canopied(in.world_position.xyz);
    let color = mix(laid.color, over.rgb, over.a) * (1.0 + dancing(in.world_position.xyz, sunk));
    input.material.base_color = vec4<f32>(color * mix(1.0, laid.wet.x, soaked), 1.0);
    // The water over sunk ground is what mirrors: the ground under it shows no gloss of its own.
    let glossy = soaked * (1.0 - smoothstep(0.0, GLOSS_DROWNS, sunk));
    input.material.perceptual_roughness = mix(finish.x, laid.wet.y, glossy);
    return main_pass_post_lighting_processing(input, lighting(input));
}

// How much brighter the light the water's surface gathers makes the bed at `at`, `under` metres
// of water: a net of bright lines drifting over the bed where the sun or the moon shines
// through the surface, brightest a little under it and gone where the water fades the light
// away or the sky is overcast.
fn dancing(at: vec3<f32>, under: f32) -> f32 {
    if under <= 0.0 {
        return 0.0;
    }
    let pixel = vec2<u32>(max((at.xz - ground.origin) * f32(ground.samples_per_metre), vec2<f32>(0.0)));
    let water = min(i32(water_of(min(pixel, ground.samples - 2u))), i32(textureDimensions(waters).x) - 1);
    let fading = textureLoad(waters, vec2<i32>(water, 0), 0).rgb;
    let through = exp(-max(fading.r, max(fading.g, fading.b)) * under);
    let gathered = under / NET_GATHERS * exp(1.0 - under / NET_GATHERS);
    let shining = max(caster_way().y, 0.0) * (1.0 - weather.clouds);
    // Light slanting down through the water lands off to the side of where it went in.
    let landed = (at.xz - caster_way().xz * under) / NET_SPAN;
    let drift = weather.time * 0.35;
    let one = dancing_noise(landed + vec2<f32>(drift, drift * 0.6));
    let other = dancing_noise(landed * 1.3 - vec2<f32>(drift * 0.7, drift) + 11.0);
    let line = 1.0 - smoothstep(0.0, 0.12, abs(one - other));
    return NET_BRIGHTEST * line * line * gathered * through * shining;
}

fn dancing_noise(at: vec2<f32>) -> f32 {
    let cell = floor(at);
    let f = fract(at);
    let s = f * f * (3.0 - 2.0 * f);
    let a = hash2(cell);
    let b = hash2(cell + vec2<f32>(1.0, 0.0));
    let c = hash2(cell + vec2<f32>(0.0, 1.0));
    let d = hash2(cell + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, s.x), mix(c, d, s.x), s.y);
}

// What the terrains lay at `at` on a patch laid every `stride` samples: those of the four paint
// pixels round it at that stride, each weighed by how near it lies, as the paint blends wherever
// the game reads it.
fn painted(at: vec2<f32>, stride: u32) -> Laid {
    let pixel = (at - ground.origin) * f32(ground.samples_per_metre) / f32(stride) - 0.5;
    let corner = floor(pixel);
    let t = pixel - corner;
    let a = terrain_at(corner, vec2<f32>(0.0, 0.0), stride, at);
    let b = terrain_at(corner, vec2<f32>(1.0, 0.0), stride, at);
    let c = terrain_at(corner, vec2<f32>(0.0, 1.0), stride, at);
    let d = terrain_at(corner, vec2<f32>(1.0, 1.0), stride, at);
    return Laid(
        mix(mix(a.color, b.color, t.x), mix(c.color, d.color, t.x), t.y),
        mix(mix(a.wet, b.wet, t.x), mix(c.wet, d.wet, t.x), t.y),
    );
}

// What the terrain painted at the pixel `step` from `corner`, a pixel every `stride` of the
// paint's, lays at `at`: its colour under its picture, repeating every tile, and how it looks
// soaked.
fn terrain_at(corner: vec2<f32>, step: vec2<f32>, stride: u32, at: vec2<f32>) -> Laid {
    let coarse = vec2<i32>(corner + step);
    let half = i32(stride / 2u);
    let fine = coarse * i32(stride) + vec2<i32>(half);
    let last = vec2<i32>(ground.samples) - 2;
    let painted = paint_of(vec2<u32>(clamp(fine, vec2<i32>(0), last)));
    let terrain = min(i32(painted), i32(textureDimensions(layers).x) - 1);
    let look = textureLoad(layers, vec2<i32>(terrain, 0), 0);
    let picture = i32(textureLoad(layers, vec2<i32>(terrain, 1), 0).r);
    let wet = textureLoad(layers, vec2<i32>(terrain, 2), 0).xyz;
    return Laid(look.rgb * textureSample(pictures, pictures_sampler, at / look.w, picture).rgb, wet);
}

// The colour of the props standing over `at`, as far as the view thins them away: each band of
// them whole where its typical prop spans no more than the grain, from where it spans twice
// that, and every band whole past the view's horizon, from where the props thin toward it, by
// how much of the ground it covers. Up close, where the props themselves stand, nothing shows of
// it.
fn canopied(at: vec3<f32>) -> vec4<f32> {
    let uv = (at.xz - ground.origin) / (ground.canopy_texel * vec2<f32>(textureDimensions(canopy)));
    let away = max(distance(at, view.world_position), 1e-3);
    let pixels_per_metre = view.viewport.w * 0.5 * view.clip_from_view[1][1] / away;
    var past = 0.0;
    if ground.horizon > 0.0 {
        past = clamp((away - ground.horizon_start) / (ground.horizon - ground.horizon_start), 0.0, 1.0);
    }
    var color = vec3<f32>(0.0);
    var cover = 0.0;
    for (var band = 0; band < i32(textureNumLayers(canopy)); band++) {
        let look = textureSample(canopy, canopy_sampler, uv, band);
        let extent = textureSample(canopy_extent, canopy_sampler, uv, band).r * ground.canopy_extent_max;
        let across = extent * pixels_per_metre;
        let thinned = max(clamp((2.0 * ground.grain_pixels - across) / ground.grain_pixels, 0.0, 1.0), past);
        let hidden = look.a * thinned;
        color += look.rgb * hidden;
        cover += hidden;
    }
    if cover <= 0.0 {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(color / cover, min(cover, 1.0));
}
