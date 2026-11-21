// The header of a shader that paints a body of water: the sheet an area lays on the water's
// surface wherever one of its waters lies, drawn once everything solid is, over a copy of that
// picture. The engine's part is the optics, and the shader composes them in `vfx`, which says
// what colour a pixel of the surface is and, in alpha, how much of the surface is there at all:
//
// `bed(bend)` is the picture behind the surface where a line of sight bent by `bend` lands, as
// lit and shadowed in this very frame, with whatever stands in the water; a line that would
// land on something in front of the surface falls back to the unbent one, which is also what
// `sample` gives, as the image the shader is handed to work on. `column(bend)` is how
// many metres of water the light crosses on the way to that point of the bed and back to the
// eye. `seen_through(behind, path)` is what is left of `behind` after `path` metres of this
// water, each colour fading at its own rate, with the water's own body lit by the hour's light
// laid over it. `water()` is the water the point belongs to, waters that meet blended across
// the metre they meet in, and `silt()` what the bed gives it: its colour, and its silt in a.
//
// `mirror(normal, rough)` is what the mirror of a face turned to `normal` shows, as rough as
// `rough` (0 a still pond): the sky's picture, the land and the things the mirrored line of
// sight meets on screen, and the sun's or the moon's glint. The line is followed off the surface
// as it lies, so the image of the land holds still, and the face's own turn then shakes where
// the image is read. `mirrors(normal)` is how much of a
// mirror the face is from where the eye stands. `bends()` is 0 for a player who has stilled the
// bending of what lies under water, 1 otherwise.
//
// As a sheet has them: `depth()` how many metres of water stand over the ground, `shore()` how
// far the point lies from the nearest dry ground, up to `SHORE_REACH` metres, `fall()` how the
// surface falls there in metres a metre toward east and south, `position()` where the point lies
// in the world, `eye()` where the camera looks from, `light()` what the hour lights a white,
// level face to there, and the weather.
import bevy_pbr::render::forward_io::FragmentOutput;
import bevy_pbr::render::mesh_types::{MESH_FLAGS_SHADOW_RECEIVER_BIT, MESH_FLAGS_SIGN_DETERMINANT_MODEL_3X3_BIT};
import bevy_pbr::render::mesh_view_bindings::{globals, lights, view};
import bevy_pbr::render::mesh_view_types::DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT;
import bevy_pbr::render::pbr_functions::{calculate_view, main_pass_post_lighting_processing};
import bevy_pbr::render::pbr_types::{pbr_input_new, STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT};
import bevy_pbr::render::shadows::fetch_directional_shadow;
import bevy_pbr::render::view_transformations::{frag_coord_to_uv, ndc_to_uv, position_ndc_to_world, position_world_to_clip, position_world_to_ndc, position_world_to_view, uv_to_ndc};
@if(VIEW_TRANSMISSION_TEXTURE)
import bevy_pbr::render::mesh_view_bindings::{view_transmission_sampler, view_transmission_texture};
@if(DEPTH_PREPASS)
import bevy_pbr::render::mesh_view_bindings::depth_prepass_texture;
@if(DEPTH_PREPASS)
import bevy_pbr::ssr::raymarch::{depth_ray_march_from_cs, depth_ray_march_march, depth_ray_march_new_from_depth, depth_ray_march_to_ws_dir};
import engine::core::render::world::gpu::sheet::{drawn_sheet, sheet_point, sky_picture, sky_picture_sampler};
import engine::core::render::world::shade::{caster_way, lighting};
import engine::core::render::world::sky_picture::sky_round;

@group(3) @binding(100) var<uniform> site: Site;
@group(3) @binding(101) var<uniform> params: Params;

// The water a point belongs to: how deep a white bed still shows through it, the colour of its
// depths under a white light, and how much light it sends back from within.
struct Water {
    clarity: f32,
    depths: vec3<f32>,
    murk: f32,
}

// How many lengths of a water's fading leave a twentieth of a thing's contrast.
const SEEN_LENGTHS: f32 = 3.0;
// How much faster a bed all silt fades the light through the water over it, a metre, and within
// how many metres of depth its silt hangs: a third of it is left that far over the bed.
const SILT_FADING: f32 = 3.0;
const SILT_HANGS: f32 = 1.5;
// How far across the screen a bent line of sight lands from the unbent one, in screen heights,
// for a bend of one over a metre of water seen from a metre off; and the deepest water that
// bends it any farther.
const BEND_REACH: f32 = 0.5;
const BEND_DEEPEST: f32 = 2.0;
// What water bends light by, as glass does: how much slower light is in it than in air.
const WATER_SLOWS: f32 = 1.333;
// How much of a mirror water is seen straight on.
const MIRRORS_STRAIGHT: f32 = 0.02;
// The least a mirrored line of sight climbs: a mirror never shows what lies under it.
const MIRROR_CLIMBS: f32 = 0.02;
// The sky's picture spans a half turn over this many pixels at its finest size.
const SKY_PICTURE_SIDE: f32 = 256.0;
// How rough a face no longer holds an image of the land, and from how rough it starts to lose
// it.
const IMAGE_LOST: f32 = 0.35;
const IMAGE_FADES: f32 = 0.15;
// How many steps a mirrored line of sight is followed across the screen in, and how thick the
// things it meets are taken to be, in metres: a line passing farther behind a thing than that
// hasn't met it.
const MIRROR_STEPS: u32 = 12u;
const MIRROR_THICK: f32 = 6.0;
// How near the picture's edge an image of the land starts to fade, as a share of the picture;
// and how far a face's turn shakes where the image is read at most, as a share of the picture.
const IMAGE_EDGE: f32 = 0.12;
const IMAGE_SHAKES: f32 = 0.04;
// How much rougher a metre of distance makes the face a glint is thrown off, the ripples under a
// far pixel pointing every way at once; and the brightest a glint shows, in times the light.
const GLINT_WIDENS: f32 = 0.0004;
const GLINT_MOST: f32 = 8.0;

struct WaterOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) lies: vec4<f32>,
    @location(4) bed: vec4<f32>,
    @location(5) fading: vec4<f32>,
    @location(6) depths: vec4<f32>,
}

var<private> deep: f32;
var<private> ashore: f32;
var<private> falling: vec2<f32>;
var<private> spot: vec3<f32>;
var<private> lay: vec3<f32>;
var<private> pixel: vec4<f32>;
var<private> shine: vec3<f32>;
var<private> gives: vec4<f32>;
var<private> fades: vec4<f32>;
var<private> body: vec3<f32>;

fn depth() -> f32 {
    return deep;
}

fn shore() -> f32 {
    return ashore;
}

fn fall() -> vec2<f32> {
    return falling;
}

fn position() -> vec3<f32> {
    return spot;
}

fn eye() -> vec3<f32> {
    return view.world_position;
}

fn light() -> vec3<f32> {
    return shine;
}

fn bends() -> f32 {
    return drawn_sheet.bends;
}

fn water() -> Water {
    let slowest = min(fades.r, min(fades.g, fades.b));
    return Water(SEEN_LENGTHS / (2.0 * max(slowest, 1.0e-6)), body, fades.a);
}

fn silt() -> vec4<f32> {
    return gives;
}

fn bed(bend: vec2<f32>) -> vec3<f32> {
    return behind_at(landing(bend));
}

fn sample(at: vec2<f32>) -> vec4<f32> {
    return vec4<f32>(bed(vec2<f32>(0.0)), 1.0);
}

fn column(bend: vec2<f32>) -> f32 {
    let under = bed_under(landing(bend));
    let seen = distance(under, spot);
    let sunk = max(spot.y - under.y, 0.0);
    let toward = caster_way();
    // The light goes down through the water no flatter than the surface bends it.
    let steep = sqrt(1.0 - (1.0 - toward.y * toward.y) / (WATER_SLOWS * WATER_SLOWS));
    return seen + sunk / steep;
}

fn seen_through(behind: vec3<f32>, path: f32) -> vec3<f32> {
    let silted = gives.a * exp(-deep / SILT_HANGS);
    let fade = exp(-(fades.rgb + silted * SILT_FADING) * max(path, 0.0));
    let within = mix(body, gives.rgb, silted) * shine * mix(fades.a, 1.0, silted);
    return behind * fade + within * (1.0 - fade);
}

fn mirrors(normal: vec3<f32>) -> f32 {
    let toward = normalize(view.world_position - spot);
    let square = clamp(dot(normalize(normal), toward), 0.0, 1.0);
    return MIRRORS_STRAIGHT + (1.0 - MIRRORS_STRAIGHT) * pow(1.0 - square, 5.0);
}

fn mirror(normal: vec3<f32>, rough: f32) -> vec3<f32> {
    let face = normalize(normal);
    let toward = normalize(view.world_position - spot);
    let thrown = climbing(reflect(-toward, face));
    var shown = vec3<f32>(0.0);
    if drawn_sheet.under_sky != 0u {
        // A rough face throws its mirror about four times as wide as it is rough.
        let across = 4.0 * rough * SKY_PICTURE_SIDE / 3.1415927;
        let size = log2(max(across, 1.0));
        shown = textureSampleLevel(sky_picture, sky_picture_sampler, sky_round(thrown), size).rgb
            * view.exposure;
    }
    let held = 1.0 - smoothstep(IMAGE_FADES, IMAGE_LOST, rough);
    if drawn_sheet.mirrors_land != 0u && held > 0.0 {
        let land = land_met(climbing(reflect(-toward, lay)), thrown);
        shown = mix(shown, land.rgb, land.a * held);
    }
    return shown + glint(face, rough, toward);
}

// A mirrored line of sight held to climbing: a mirror never shows what lies under it.
fn climbing(thrown: vec3<f32>) -> vec3<f32> {
    return normalize(vec3<f32>(thrown.x, max(thrown.y, MIRROR_CLIMBS), thrown.z));
}

// The glint of the sky's lights off a face turned to `face`, as rough as `rough`, seen from
// `toward`: each light's own, where no shadow falls.
fn glint(face: vec3<f32>, rough: f32, toward: vec3<f32>) -> vec3<f32> {
    let away = distance(view.world_position, spot);
    let spread = clamp(rough + away * GLINT_WIDENS, 0.02, 1.0);
    let spread_squared = spread * spread;
    let view_z = position_world_to_view(spot).z;
    let facing = max(dot(face, toward), 0.05);
    var thrown = vec3<f32>(0.0);
    for (var index = 0u; index < lights.n_directional_lights; index++) {
        let shining = &lights.directional_lights[index];
        let sunward = (*shining).direction_to_light;
        if sunward.y <= 0.0 {
            continue;
        }
        let half_way = normalize(toward + sunward);
        let aligned = max(dot(face, half_way), 0.0);
        let lobe = aligned * aligned * (spread_squared - 1.0) + 1.0;
        let gathered = spread_squared / (3.1415927 * lobe * lobe);
        let glancing = 1.0 - max(dot(half_way, toward), 0.0);
        let mirrored = MIRRORS_STRAIGHT + (1.0 - MIRRORS_STRAIGHT) * pow(glancing, 5.0);
        var lit = 1.0;
        if ((*shining).flags & DIRECTIONAL_LIGHT_FLAGS_SHADOWS_ENABLED_BIT) != 0u {
            lit = fetch_directional_shadow(index, vec4<f32>(spot, 1.0), face, view_z, pixel.xy);
        }
        let strength = min(gathered * mirrored / (4.0 * facing), GLINT_MOST);
        thrown += (*shining).color.rgb * view.exposure * strength * lit;
    }
    return thrown;
}

// Where on screen the line of sight bent by `bend` lands: the pixel's own place moved by the
// bend, farther the deeper the water and the nearer the surface; the pixel's own place where
// what shows there stands in front of the surface.
fn landing(bend: vec2<f32>) -> vec2<f32> {
    let own = frag_coord_to_uv(pixel.xy);
    let away = max(distance(view.world_position, spot), 1.0);
    let reach = BEND_REACH * min(deep, BEND_DEEPEST) / away * drawn_sheet.bends;
    let bent = clamp(own + bend * reach * vec2<f32>(view.viewport.w / view.viewport.z, 1.0), vec2<f32>(0.0), vec2<f32>(1.0));
    if depth_behind(bent) > pixel.z {
        return own;
    }
    return bent;
}

@if(VIEW_TRANSMISSION_TEXTURE)
fn behind_at(at: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(view_transmission_texture, view_transmission_sampler, at, 0.0).rgb;
}
@else
fn behind_at(at: vec2<f32>) -> vec3<f32> {
    return gives.rgb * shine;
}

// How deep into the picture what lies behind the surface at `at` on screen lies, as the picture's
// depth holds it: nearer is more.
@if(DEPTH_PREPASS)
fn depth_behind(at: vec2<f32>) -> f32 {
    let texel = vec2<i32>(at * view.viewport.zw + view.viewport.xy);
    return textureLoad(depth_prepass_texture, texel, 0);
}
@else
fn depth_behind(at: vec2<f32>) -> f32 {
    return 0.0;
}

// Where in the world what lies behind the surface at `at` on screen lies: straight under the
// surface's own point, as deep as the water stands, for a view that kept no depth of it.
@if(DEPTH_PREPASS)
fn bed_under(at: vec2<f32>) -> vec3<f32> {
    let behind = depth_behind(at);
    if behind <= 0.0 {
        return spot - vec3<f32>(0.0, deep, 0.0);
    }
    return position_ndc_to_world(vec3<f32>(uv_to_ndc(at), behind));
}
@else
fn bed_under(at: vec2<f32>) -> vec3<f32> {
    return spot - vec3<f32>(0.0, deep, 0.0);
}

// The land and the things the mirrored line of sight `level` meets on screen, read where the
// line `shaken` would have met them as far off, and in alpha how surely it met them: nothing
// where it leaves the picture or passes behind what it would meet, and less toward the
// picture's edges, where the next thing along may be out of view.
@if(DEPTH_PREPASS && VIEW_TRANSMISSION_TEXTURE)
fn land_met(level: vec3<f32>, shaken: vec3<f32>) -> vec4<f32> {
    var march = depth_ray_march_new_from_depth(view.viewport.zw);
    depth_ray_march_from_cs(&march, position_world_to_ndc(spot + level * 0.05));
    depth_ray_march_to_ws_dir(&march, level);
    march.linear_steps = MIRROR_STEPS;
    march.bisection_steps = 4u;
    march.use_secant = true;
    march.depth_thickness_linear_z = MIRROR_THICK;
    march.jitter = fract(52.9829189 * fract(dot(pixel.xy, vec2<f32>(0.06711056, 0.00583715))));
    let met = depth_ray_march_march(&march);
    if !met.hit {
        return vec4<f32>(0.0);
    }
    let near_edge = min(min(met.hit_uv.x, 1.0 - met.hit_uv.x), min(met.hit_uv.y, 1.0 - met.hit_uv.y));
    let inside = smoothstep(0.0, IMAGE_EDGE, near_edge);
    let found = bed_under(met.hit_uv);
    // What lies under the surface lies behind the mirror, never in it.
    if found.y < spot.y {
        return vec4<f32>(0.0);
    }
    let thrown_to = ndc_to_uv(position_world_to_ndc(spot + shaken * distance(found, spot)).xy);
    let shook = met.hit_uv + clamp(thrown_to - met.hit_uv, vec2<f32>(-IMAGE_SHAKES), vec2<f32>(IMAGE_SHAKES));
    let read = select(met.hit_uv, shook, bed_under(shook).y >= spot.y);
    return vec4<f32>(behind_at(read), inside);
}
@else
fn land_met(level: vec3<f32>, shaken: vec3<f32>) -> vec4<f32> {
    return vec4<f32>(0.0);
}

@vertex
fn vertex(@builtin(instance_index) instance: u32, @location(0) grid: vec3<f32>) -> WaterOutput {
    let point = sheet_point(grid, instance);
    var out: WaterOutput;
    out.world_position = vec4<f32>(point.world, 1.0);
    out.position = position_world_to_clip(point.world);
    out.world_normal = point.normal;
    out.uv = point.uv;
    out.lies = vec4<f32>(point.depth, point.shore, point.fall);
    out.bed = point.bed;
    out.fading = point.fading;
    out.depths = vec4<f32>(point.depths, point.weight);
    return out;
}

@fragment
fn fragment(in: WaterOutput) -> FragmentOutput {
    // A point belongs to the water most of the pixels round it name, and no water lies where
    // none stands over the ground.
    if in.depths.a < 0.5 || in.lies.x <= 0.0 {
        discard;
    }
    deep = in.lies.x;
    ashore = in.lies.y;
    falling = in.lies.zw;
    spot = in.world_position.xyz;
    lay = normalize(in.world_normal);
    pixel = in.position;
    gives = in.bed;
    fades = in.fading;
    body = in.depths.rgb;
    read_weather();
    var input = pbr_input_new();
    input.flags = MESH_FLAGS_SHADOW_RECEIVER_BIT | MESH_FLAGS_SIGN_DETERMINANT_MODEL_3X3_BIT;
    input.material.flags = STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT;
    input.material.base_color = vec4<f32>(1.0);
    input.material.perceptual_roughness = 1.0;
    input.material.metallic = 0.0;
    input.material.reflectance = vec3<f32>(0.0);
    input.frag_coord = in.position;
    input.world_position = in.world_position;
    input.world_normal = vec3<f32>(0.0, 1.0, 0.0);
    input.N = input.world_normal;
    input.is_orthographic = view.clip_from_view[3].w == 1.0;
    input.V = calculate_view(in.world_position, input.is_orthographic);
    shine = lighting(input).rgb;
    let painted = vfx(in.uv);
    let shown = mix(behind_at(frag_coord_to_uv(pixel.xy)), painted.rgb, clamp(painted.a, 0.0, 1.0));
    var out: FragmentOutput;
    out.color = main_pass_post_lighting_processing(input, vec4<f32>(shown, 1.0));
    return out;
}
