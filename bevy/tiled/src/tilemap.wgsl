#import bevy_sprite::mesh2d_vertex_output::VertexOutput

// A whole flat layer drawn as one quad, sampled per fragment: the index texture holds the tile id and
// flips for each cell, the frame map remaps a tile id to its currently-animated id, and the atlas is
// the sheet. Geometry is O(1) in map size; only on-screen fragments shade.
@group(2) @binding(0) var index_tex: texture_2d<u32>;
@group(2) @binding(1) var atlas_tex: texture_2d<f32>;
@group(2) @binding(2) var atlas_sampler: sampler;
@group(2) @binding(3) var frame_map: texture_2d<u32>;
@group(2) @binding(4) var<uniform> tm: Tilemap;

struct Tilemap {
    grid: vec4<f32>,   // map_w, map_h, atlas_cols, _
    sheet: vec4<f32>,  // atlas_w, atlas_h, tile_w, tile_h
    params: vec4<f32>, // margin, spacing, _, _
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let map_size = tm.grid.xy;
    let cols = u32(tm.grid.z);
    let atlas_size = tm.sheet.xy;
    let tile_size = tm.sheet.zw;
    let margin = tm.params.x;
    let spacing = tm.params.y;

    let cell_f = in.uv * map_size;
    let cell = clamp(vec2<i32>(floor(cell_f)), vec2(0), vec2<i32>(map_size) - vec2(1));
    let entry = textureLoad(index_tex, cell, 0);
    if entry.w == 0u {
        return vec4(0.0);
    }
    let tile_id = entry.x | (entry.y << 8u);
    let mapped = textureLoad(frame_map, vec2<i32>(i32(tile_id), 0), 0);
    let atlas_id = mapped.x | (mapped.y << 8u);

    var intra = fract(cell_f);
    if (entry.z & 1u) != 0u { intra.x = 1.0 - intra.x; }
    if (entry.z & 2u) != 0u { intra.y = 1.0 - intra.y; }

    let atlas_cell = vec2<f32>(f32(atlas_id % cols), f32(atlas_id / cols));
    let origin = vec2(margin) + atlas_cell * (tile_size + vec2(spacing));
    let atlas_px = origin + intra * tile_size;
    return textureSample(atlas_tex, atlas_sampler, atlas_px / atlas_size);
}
