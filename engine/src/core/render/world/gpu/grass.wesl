// A cover's blades as a view draws them, three vertices a blade, from the piece's slab: a
// triangle standing up from the blade's root, its base turned as the blade is and as wide as its
// shape at its size, its tip as tall, lit by the ground's normal under it. Below the grain a
// blade is thinned by its seed and grown to make up for those dropped, and the player's grass
// setting thins them all; a blade thinned away draws nothing.
import bevy_pbr::render::mesh_view_bindings::view;

struct Blade {
    root: vec3<f32>,
    size: f32,
    normal: vec3<f32>,
    yaw: f32,
    tint: f32,
    weight: f32,
    seed: f32,
    unused: f32,
}

struct BladeLook {
    color_from: vec4<f32>,
    color_to: vec4<f32>,
    width: f32,
    height: f32,
    piece: u32,
    density: f32,
    grain_pixels: f32,
    grain_growth: f32,
}

@group(2) @binding(100) var<storage, read> blades: array<Blade>;
@group(2) @binding(101) var<uniform> look: BladeLook;

// A vertex of a blade: where it stands, its blade's root, the ground's normal, its uv across and
// up the blade, how far up it stands, the blade's weight, seed, colour and piece, and whether it
// stands at all.
struct BladePoint {
    world: vec3<f32>,
    root: vec3<f32>,
    normal: vec3<f32>,
    uv: vec2<f32>,
    measure: f32,
    weight: f32,
    seed: f32,
    color: vec4<f32>,
    piece: u32,
    shown: bool,
}

// The `vertex`th vertex of the blades, its blade's a third of it.
fn blade_point(vertex: u32) -> BladePoint {
    let blade = blades[vertex / 3u];
    let corner = vertex % 3u;
    var size = blade.size;
    var kept = look.density;
    let across_pixels = max(look.width, look.height) * size * pixels_per_meter(blade.root);
    if across_pixels < look.grain_pixels {
        let grown = min(look.grain_pixels / max(across_pixels, 1e-6), look.grain_growth);
        kept /= grown * grown;
        size *= grown;
    }
    let across = vec3<f32>(cos(blade.yaw), 0.0, -sin(blade.yaw)) * (look.width * 0.5 * size);
    var point: BladePoint;
    point.root = blade.root;
    point.normal = blade.normal;
    point.weight = blade.weight;
    point.seed = blade.seed;
    point.color = mix(look.color_from, look.color_to, blade.tint);
    point.piece = look.piece;
    point.shown = blade.seed < kept;
    point.world = blade.root - across;
    point.uv = vec2<f32>(0.0, 0.0);
    point.measure = 0.0;
    if corner == 1u {
        point.world = blade.root + across;
        point.uv = vec2<f32>(1.0, 0.0);
    } else if corner == 2u {
        point.world = blade.root + vec3<f32>(0.0, look.height * size, 0.0);
        point.uv = vec2<f32>(0.5, 1.0);
        point.measure = 1.0;
    }
    return point;
}

// How many pixels a metre spans at `at`, through the view's lens.
fn pixels_per_meter(at: vec3<f32>) -> f32 {
    let at_a_meter = view.clip_from_view[1][1] * view.viewport.w * 0.5;
    if view.clip_from_view[3][3] == 1.0 {
        return at_a_meter;
    }
    return at_a_meter / max(distance(at, view.world_position), 1e-3);
}
