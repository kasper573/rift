// A prop drawn from its impostor atlas, as the far field draws it: a square facing the eye at
// the middle of the instance's bounding sphere, a radius each way, on which the model's picture
// from the eye's direction is read between the four cells of the atlas round that direction,
// each cell's picture seen through its own orthographic view. The pixel is dropped below half
// coverage, lit by the baked normal in the world's light, and given the depth the baked depth
// puts it at, so an impostor sorts against the ground and its neighbours as the model would.
// Without the atlas, a box of the prop's bounds stands in its place, coloured by its seed. In the
// prepass and the shadow maps the same shapes write depth alone.

@if(PREPASS_PIPELINE)
import bevy_pbr::render::pbr_prepass_functions::calculate_motion_vector;
@if(!PREPASS_PIPELINE)
import bevy_pbr::render::{
    mesh_types::{MESH_FLAGS_SHADOW_RECEIVER_BIT, MESH_FLAGS_SIGN_DETERMINANT_MODEL_3X3_BIT},
    pbr_functions::{calculate_view, main_pass_post_lighting_processing},
    pbr_types::{pbr_input_new, STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT},
};
@if(!PREPASS_PIPELINE)
import engine::core::render::world::shade::lighting;
import bevy_pbr::render::mesh_view_bindings::view;
import bevy_pbr::render::view_transformations::position_world_to_clip;
import engine::core::render::world::gpu::instance::{
    Placement, clear_impostor, placement, prop_of, seed_of, shown_of, turned, world_of,
};

// A prop as the kernel reads it, for the sphere round its model and the box within it.
struct Prop {
    x: f32,
    y: f32,
    z: f32,
    radius: f32,
    extent: f32,
    first_part: u32,
    parts: u32,
    fades: u32,
    half_x: f32,
    half_y: f32,
    half_z: f32,
    impostor_command: u32,
    proxy_command: u32,
}

@group(2) @binding(103) var<storage, read> props: array<Prop>;
// Each prop's layer of the atlas, one more than it is: 0 for a prop the atlas holds no picture of.
@group(2) @binding(104) var<storage, read> layers: array<u32>;

@group(3) @binding(100) var albedo: texture_2d_array<f32>;
@group(3) @binding(101) var normals: texture_2d_array<f32>;
@group(3) @binding(102) var depths: texture_2d_array<f32>;
@group(3) @binding(103) var atlas_sampler: sampler;

// The atlas's cells along each side, as it was baked.
const CELLS: f32 = 8.0;
// The corners of a square, in the order its index strip draws them.
const CORNERS: array<vec2<f32>, 4> = array<vec2<f32>, 4>(
    vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, 1.0),
);
// The way each face of a box looks, four vertices a face, in the order its index strip draws.
const FACES: array<vec3<f32>, 6> = array<vec3<f32>, 6>(
    vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(-1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0),
    vec3<f32>(0.0, -1.0, 0.0), vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0, 0.0, -1.0),
);

struct ImpostorOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) world_position: vec4<f32>,
    // Where on the square the pixel lies, a radius each way of the middle.
    @location(1) square: vec2<f32>,
    // The eye's direction from the instance's middle, in the model's own frame.
    @location(2) @interpolate(flat) direction: vec3<f32>,
    @location(3) @interpolate(flat) middle: vec3<f32>,
    @location(4) @interpolate(flat) turn: vec4<f32>,
    @location(5) @interpolate(flat) radius_layer: vec2<f32>,
    @location(6) @interpolate(flat) shown: vec3<f32>,
    @location(7) world_normal: vec3<f32>,
    @location(8) @interpolate(flat) color: vec3<f32>,
    @if(PREPASS_PIPELINE && UNCLIPPED_DEPTH_ORTHO_EMULATION)
    @location(9) unclipped_depth: f32,
}

@vertex
fn vertex(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> ImpostorOutput {
    let placed = placement(instance);
    let prop = props[prop_of(instance)];
    var out: ImpostorOutput;
    out.turn = placed.turn;
    out.shown = shown_of(instance);
    out.middle = world_of(placed, vec3<f32>(prop.x, prop.y, prop.z));
    @if(PROXY) {
        let face = vertex / 4u;
        let corner = CORNERS[vertex % 4u];
        let normal = FACES[face];
        let across = select(vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(1.0, 0.0, 0.0), face >= 4u);
        let up = select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(0.0, 0.0, 1.0), face == 2u || face == 3u);
        let half = vec3<f32>(prop.half_x, prop.half_y, prop.half_z);
        let local = vec3<f32>(prop.x, prop.y, prop.z) + half * (normal + across * corner.x + up * corner.y);
        let world = world_of(placed, local);
        out.position = position_world_to_clip(world);
        out.world_position = vec4<f32>(world, 1.0);
        out.world_normal = turned(placed.turn, normal);
        out.color = seed_color(seed_of(instance));
    }
    @if(!PROXY) {
        let radius = prop.radius * placed.scale;
        let eye = eye_from(out.middle);
        let right = normalize(cross(vec3<f32>(0.0, 1.0, 0.0), eye));
        let up = cross(eye, right);
        let corner = CORNERS[vertex % 4u];
        let world = out.middle + (right * corner.x + up * corner.y) * radius;
        out.position = position_world_to_clip(world);
        out.world_position = vec4<f32>(world, 1.0);
        out.square = corner;
        out.direction = unturned(placed.turn, eye);
        out.radius_layer = vec2<f32>(radius, f32(layers[prop_of(instance)]));
        out.world_normal = eye;
    }
    @if(PREPASS_PIPELINE && UNCLIPPED_DEPTH_ORTHO_EMULATION) {
        out.unclipped_depth = out.position.z;
        out.position.z = min(out.position.z, 1.0);
    }
    return out;
}

// What a pixel of the square shows of the instance: its colour and coverage, its normal in the
// world, and where in the world its surface lies.
struct Surface {
    color: vec4<f32>,
    normal: vec3<f32>,
    world: vec3<f32>,
}

// The pixel's surface: for a proxy its face, else what the atlas shows there, dropped below half
// coverage or off the square's picture.
fn surface(in: ImpostorOutput) -> Surface {
    clear_impostor(in.position.xy, in.shown);
    var out: Surface;
    @if(PROXY) {
        out.color = vec4<f32>(in.color, 1.0);
        out.normal = normalize(in.world_normal);
        out.world = in.world_position.xyz;
    }
    @if(!PROXY) {
        out = read_atlas(in);
        if out.color.a < 0.5 {
            discard;
        }
    }
    return out;
}

@if(!PROXY)
fn read_atlas(in: ImpostorOutput) -> Surface {
    let layer = i32(in.radius_layer.y) - 1;
    let radius = in.radius_layer.x;
    let at = fold_hemi(in.direction) * (CELLS - 1.0);
    let low = floor(min(at, vec2<f32>(CELLS - 1.001)));
    let t = at - low;
    let weights = vec4<f32>((1.0 - t.x) * (1.0 - t.y), t.x * (1.0 - t.y), (1.0 - t.x) * t.y, t.x * t.y);
    let right = normalize(cross(vec3<f32>(0.0, 1.0, 0.0), in.direction));
    let up = cross(in.direction, right);
    // The pixel's point on the square, in the model's frame, a radius each way of the middle.
    let point = right * in.square.x + up * in.square.y;
    var color = vec4<f32>(0.0);
    var normal = vec3<f32>(0.0);
    var depth = 0.0;
    var heaviest = 0.0;
    var heaviest_seen = in.direction;
    for (var k = 0u; k < 4u; k++) {
        let cell = low + vec2<f32>(f32(k & 1u), f32(k >> 1u));
        let seen = unfold_hemi(cell / (CELLS - 1.0));
        let cell_right = normalize(cross(cell_up(seen), seen));
        let cell_up_way = cross(seen, cell_right);
        // Where the square's point lands on the cell's picture, which looks along `seen`.
        let picture = vec2<f32>(dot(point, cell_right), dot(point, cell_up_way)) * 0.5 + 0.5;
        if any(picture < vec2<f32>(0.0)) || any(picture > vec2<f32>(1.0)) {
            continue;
        }
        let texel = (cell + picture) / CELLS;
        let read = textureSample(albedo, atlas_sampler, texel, layer);
        let weight = weights[k] * read.a;
        color += vec4<f32>(read.rgb * weight, weight);
        normal += unfold_oct(textureSample(normals, atlas_sampler, texel, layer).xy) * weight;
        depth += textureSample(depths, atlas_sampler, texel, layer).x * weight;
        if weight > heaviest {
            heaviest = weight;
            heaviest_seen = seen;
        }
    }
    let coverage = max(color.a, 1e-4);
    var out: Surface;
    out.color = vec4<f32>(color.rgb / coverage, color.a);
    out.normal = normalize(turned(in.turn, normal / coverage));
    // The baked depth runs from the cell's near plane, a radius out from the middle the way the
    // cell looks from, to a diameter in.
    let surface = point + heaviest_seen * (1.0 - 2.0 * depth / coverage);
    out.world = in.middle + turned(in.turn, surface) * radius;
    return out;
}

@if(!PREPASS_PIPELINE)
struct LitOutput {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
}

@if(!PREPASS_PIPELINE)
@fragment
fn fragment(in: ImpostorOutput) -> LitOutput {
    let found = surface(in);
    let clip = position_world_to_clip(found.world);
    var input = pbr_input_new();
    input.material.flags |= STANDARD_MATERIAL_FLAGS_FOG_ENABLED_BIT;
    input.material.base_color = vec4<f32>(found.color.rgb, 1.0);
    input.material.metallic = 0.0;
    input.material.perceptual_roughness = 0.9;
    input.flags = MESH_FLAGS_SHADOW_RECEIVER_BIT | MESH_FLAGS_SIGN_DETERMINANT_MODEL_3X3_BIT;
    input.frag_coord = vec4<f32>(in.position.xy, clip.z / clip.w, in.position.w);
    input.world_position = vec4<f32>(found.world, 1.0);
    input.world_normal = found.normal;
    input.N = found.normal;
    input.is_orthographic = view.clip_from_view[3].w == 1.0;
    input.V = calculate_view(input.world_position, input.is_orthographic);
    var out: LitOutput;
    out.color = main_pass_post_lighting_processing(input, lighting(input));
    out.depth = clip.z / clip.w;
    return out;
}

// What the prepass writes of the surface: bevy's prepass outputs, and the depth the surface lies
// at, which every prepass of the impostors writes.
@if(PREPASS_PIPELINE)
struct DepthOutput {
    @if(NORMAL_PREPASS)
    @location(0) normal: vec4<f32>,
    @if(MOTION_VECTOR_PREPASS)
    @location(1) motion_vector: vec2<f32>,
    @builtin(frag_depth) depth: f32,
}

@if(PREPASS_PIPELINE)
@fragment
fn fragment(in: ImpostorOutput) -> DepthOutput {
    let found = surface(in);
    let clip = position_world_to_clip(found.world);
    var out: DepthOutput;
    out.depth = clip.z / clip.w;
    @if(UNCLIPPED_DEPTH_ORTHO_EMULATION) {
        out.depth = in.unclipped_depth + (clip.z - in.position.z);
    }
    @if(NORMAL_PREPASS) {
        out.normal = vec4<f32>(found.normal * 0.5 + vec3<f32>(0.5), 1.0);
    }
    @if(MOTION_VECTOR_PREPASS) {
        out.motion_vector = calculate_motion_vector(input_world(found), input_world(found));
    }
    return out;
}

@if(PREPASS_PIPELINE)
fn input_world(found: Surface) -> vec4<f32> {
    return vec4<f32>(found.world, 1.0);
}

// The way from `at` to the eye: straight back through the view where it is orthographic.
fn eye_from(at: vec3<f32>) -> vec3<f32> {
    if view.clip_from_view[3][3] == 1.0 {
        return normalize(view.world_from_view[2].xyz);
    }
    return normalize(view.world_position - at);
}

// `way` turned back by the quaternion `turn`.
fn unturned(turn: vec4<f32>, way: vec3<f32>) -> vec3<f32> {
    return turned(vec4<f32>(-turn.xyz, turn.w), way);
}

// Which way is up for a cell looking along `seen`, as it was baked: the world's up, or back
// along the ground for a cell looking straight down.
fn cell_up(seen: vec3<f32>) -> vec3<f32> {
    return select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(0.0, 0.0, -1.0), abs(seen.y) > 0.999);
}

// Where on the atlas's unfolded square a direction lies, as the atlas was baked: a direction
// below the horizon reads as the horizon's.
fn fold_hemi(direction: vec3<f32>) -> vec2<f32> {
    let above = vec3<f32>(direction.x, max(direction.y, 0.0), direction.z);
    let scaled = above / max(abs(above.x) + above.y + abs(above.z), 1e-6);
    let folded = vec2<f32>(scaled.x + scaled.z, scaled.x - scaled.z);
    return clamp((folded + vec2<f32>(1.0)) * 0.5, vec2<f32>(0.0), vec2<f32>(1.0));
}

// The direction the unfolded square's point `unit` stands for.
fn unfold_hemi(unit: vec2<f32>) -> vec3<f32> {
    let folded = unit * 2.0 - vec2<f32>(1.0);
    let x = (folded.x + folded.y) * 0.5;
    let z = (folded.x - folded.y) * 0.5;
    return normalize(vec3<f32>(x, 1.0 - abs(x) - abs(z), z));
}

// The direction a normal folded over the whole octahedron, as the bake writes it, stands for.
fn unfold_oct(folded: vec2<f32>) -> vec3<f32> {
    let f = folded * 2.0 - vec2<f32>(1.0);
    var n = vec3<f32>(f.x, 1.0 - abs(f.x) - abs(f.y), f.y);
    let t = max(-n.y, 0.0);
    n.x += select(t, -t, n.x >= 0.0);
    n.z += select(t, -t, n.z >= 0.0);
    return normalize(n);
}

// A colour of the seed's own, so neighbouring proxies are told apart.
fn seed_color(seed: u32) -> vec3<f32> {
    let hue = f32(seed % 360u) / 360.0;
    let k = fract(vec3<f32>(hue, hue + 1.0 / 3.0, hue + 2.0 / 3.0)) * 6.0;
    return 0.25 + 0.5 * clamp(abs(k - 3.0) - 1.0, vec3<f32>(0.0), vec3<f32>(1.0));
}
