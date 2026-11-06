//! How far a coarser stand-in for a mesh may stray from its shape, and the stand-ins themselves:
//! the triangles of a mesh, coarser and coarser, each with how far its shape strays, so a mesh far
//! off the eye draws with no more triangles than its pixels show.

use meshopt::SimplifyOptions;

/// How many pixels across a thing among many alike must span for the view to tell it apart from
/// its neighbours: smaller, some of them may be dropped and the rest grown to this size, and the
/// crowd looks the same.
pub const GRAIN_PIXELS: f32 = 2.0;

/// How far a stand-in's shape may stray from its mesh's, in pixels of the view it is seen in.
pub(super) const STRAY_PIXELS: f32 = 0.5;

/// The triangles of the mesh `indices` draw over `positions`, coarser and coarser: halved each
/// time while the simplifier still can, collapsing across the seams of its texture coordinates and
/// normals where how far they slide (`surface`) adds little to how far its shape moves, each with
/// how far its shape strays from the mesh's, in the mesh's own units.
pub(super) fn simplify(
    indices: &[u32],
    positions: &[[f32; 3]],
    surface: Surface,
) -> Vec<(Vec<u32>, f32)> {
    let attributes: Vec<f32> = (0..positions.len())
        .flat_map(|vertex| {
            let [u, v] = surface.uvs.map_or([0.0; 2], |uvs| uvs[vertex]);
            let [x, y, z] = surface.normals.map_or([0.0; 3], |normals| normals[vertex]);
            [u, v, x, y, z]
        })
        .collect();
    let weights = [
        UV_WEIGHT,
        UV_WEIGHT,
        NORMAL_WEIGHT,
        NORMAL_WEIGHT,
        NORMAL_WEIGHT,
    ];
    let unlocked = vec![false; positions.len()];
    let mut levels = Vec::new();
    let mut triangles = indices.len() / 3;
    loop {
        let target = triangles / 2;
        if target == 0 {
            return levels;
        }
        let mut strays = 0.0;
        let kept = meshopt::simplify_with_attributes_and_locks_decoder(
            indices,
            positions,
            &attributes,
            &weights,
            weights.len() * std::mem::size_of::<f32>(),
            &unlocked,
            target * 3,
            f32::MAX,
            SimplifyOptions::Permissive | SimplifyOptions::ErrorAbsolute,
            Some(&mut strays),
        );
        let kept_triangles = kept.len() / 3;
        if kept_triangles == 0 || kept_triangles * 4 > triangles * 3 {
            return levels;
        }
        levels.push((kept, strays));
        triangles = kept_triangles;
    }
}

/// What a mesh's surface carries besides its shape, for [`simplify`] to keep: its texture
/// coordinates and normals, where it has them.
#[derive(Clone, Copy, Default)]
pub(super) struct Surface<'a> {
    pub uvs: Option<&'a [[f32; 2]]>,
    pub normals: Option<&'a [[f32; 3]]>,
}

/// How much a slide of the texture coordinates and a turn of the normals weigh against the
/// shape's own moves, within the range meshoptimizer recommends for attributes spanning 0 to 1.
const UV_WEIGHT: f32 = 0.1;

const NORMAL_WEIGHT: f32 = 0.05;
