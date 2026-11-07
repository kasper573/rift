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
import engine::core::render::world::gpu::terrain::{ground, ground_point, paint_of};
import engine::core::render::world::shade::lighting;

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

struct Vertex {
    @builtin(instance_index) instance: u32,
    @location(0) position: vec3<f32>,
}

struct Placed {
    @builtin(position) clip: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) @interpolate(flat) stride: u32,
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
    return Placed(position_world_to_clip(point.world), world, point.normal, point.stride);
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
    let soaked = wetness(in.world_position.xyz) * laid.wet.z;
    let over = canopied(in.world_position.xyz);
    let color = mix(laid.color, over.rgb, over.a);
    input.material.base_color = vec4<f32>(color * mix(1.0, laid.wet.x, soaked), 1.0);
    input.material.perceptual_roughness = mix(finish.x, laid.wet.y, soaked);
    return main_pass_post_lighting_processing(input, lighting(input));
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
// that, by how much of the ground it covers. Up close, where the props themselves stand, nothing
// shows of it.
fn canopied(at: vec3<f32>) -> vec4<f32> {
    let uv = (at.xz - ground.origin) / (ground.canopy_texel * vec2<f32>(textureDimensions(canopy)));
    let away = max(distance(at, view.world_position), 1e-3);
    let pixels_per_metre = view.viewport.w * 0.5 * view.clip_from_view[1][1] / away;
    var color = vec3<f32>(0.0);
    var cover = 0.0;
    for (var band = 0; band < i32(textureNumLayers(canopy)); band++) {
        let look = textureSample(canopy, canopy_sampler, uv, band);
        let extent = textureSample(canopy_extent, canopy_sampler, uv, band).r * ground.canopy_extent_max;
        let across = extent * pixels_per_metre;
        let thinned = clamp((2.0 * ground.grain_pixels - across) / ground.grain_pixels, 0.0, 1.0);
        let hidden = look.a * thinned;
        color += look.rgb * hidden;
        cover += hidden;
    }
    if cover <= 0.0 {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(color / cover, min(cover, 1.0));
}
