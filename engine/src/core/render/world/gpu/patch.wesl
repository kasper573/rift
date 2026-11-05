// A square of flat ground the GPU draw path lays: its corner on the ground, its side, and the
// height it lies at, in metres.
struct TerrainPatch {
    corner: vec2<f32>,
    side: f32,
    height: f32,
}

@group(2) @binding(100) var<uniform> terrain_patch: TerrainPatch;

// Where the point `at` of the grid, from 0 to 1 across the patch each way, lies in the world.
fn placed(at: vec3<f32>) -> vec4<f32> {
    return vec4<f32>(
        terrain_patch.corner.x + at.x * terrain_patch.side,
        terrain_patch.height,
        terrain_patch.corner.y + at.z * terrain_patch.side,
        1.0,
    );
}
