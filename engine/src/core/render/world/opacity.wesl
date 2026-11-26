import bevy_pbr::render::mesh_bindings::mesh;

import engine::core::render::world::clearing::dither;

// How many of a mesh tag's low bits tell how much of the mesh has faded; the bits over them are
// another's to read.
const FADE_BITS: u32 = 8u;

// Drops the pixel when its mesh has faded that far: the mesh's tag tells how many of 255 pixels it
// drops, none for a mesh that shows whole. The dither that picks them is the clearing's, which the
// picture's settling over the frames blends into a plain translucency.
fn thin(pixel: vec2<f32>, instance: u32) {
    let dropped = mesh[instance].tag & ((1u << FADE_BITS) - 1u);
    if dropped != 0u && dither(pixel) * 255.0 < f32(dropped) {
        discard;
    }
}
