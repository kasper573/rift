//! Coarser stand-ins for the meshes of the world, each with how far its shape strays from the
//! mesh's own, so a mesh far off the eye draws with no more triangles than its pixels show.

use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::ecs::system::SystemParam;
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use meshopt::SimplifyOptions;

use crate::core::render::camera::View;
use crate::core::units::Meters;

/// Simplifies the meshes of the world into their stand-ins, each once.
#[derive(SystemParam)]
pub struct Simplifier<'w> {
    levels: ResMut<'w, MeshLevels>,
    meshes: ResMut<'w, Assets<Mesh>>,
}

/// The levels of every mesh simplified so far, by the mesh they stand in for.
#[derive(Resource, Default)]
pub(super) struct MeshLevels(HashMap<AssetId<Mesh>, Arc<Levels>>);

/// The stand-ins for a mesh, finest first, each with about half the triangles of the one before
/// and how far its shape strays from the mesh's, in the mesh's own units.
pub struct Levels(Vec<(Handle<Mesh>, f32)>);

impl Simplifier<'_> {
    /// The levels of `mesh`, simplified the first time they are asked for; none until it loads.
    pub fn levels_of(&mut self, mesh: &Handle<Mesh>) -> Option<Arc<Levels>> {
        if let Some(levels) = self.levels.0.get(&mesh.id()) {
            return Some(levels.clone());
        }
        let simplified = simplified(self.meshes.get(mesh)?);
        let levels = Arc::new(Levels(
            simplified
                .into_iter()
                .map(|(level, strays)| (self.meshes.add(level), strays))
                .collect(),
        ));
        self.levels.0.insert(mesh.id(), levels.clone());
        Some(levels)
    }
}

impl Levels {
    /// The coarsest stand-in that strays from the mesh by less than [`STRAY_PIXELS`] of `view`,
    /// drawn `scale` times its size `distance` off the eye; none where only the mesh itself does.
    pub fn for_view(&self, view: &View, distance: Meters, scale: f32) -> Option<&Handle<Mesh>> {
        let allowed = STRAY_PIXELS / (view.pixels_per_meter(distance) * scale);
        self.0
            .iter()
            .rev()
            .find(|(_, strays)| *strays <= allowed)
            .map(|(level, _)| level)
    }
}

/// Forgets the levels of meshes that are gone, so their stand-ins go with them.
pub(super) fn forget_dropped(
    mut events: MessageReader<AssetEvent<Mesh>>,
    mut levels: ResMut<MeshLevels>,
) {
    let gone = super::dropped(&mut events);
    if !gone.is_empty() {
        levels.0.retain(|mesh, _| !gone.contains(mesh));
    }
}

/// How far a stand-in's shape may stray from its mesh's, in pixels of the view it is seen in.
const STRAY_PIXELS: f32 = 0.5;

/// The stand-ins for `mesh`, halving its triangles each time while the simplifier still can,
/// collapsing across the seams of its texture coordinates and normals as well, which a shape seen
/// at under a pixel's stray doesn't show; none for a mesh that isn't a list of triangles over
/// positions, that morphs, or whose data only the GPU holds.
fn simplified(mesh: &Mesh) -> Vec<(Mesh, f32)> {
    let (Ok(Some(VertexAttributeValues::Float32x3(positions))), Ok(Some(indices)), Ok(false)) = (
        mesh.try_attribute_option(Mesh::ATTRIBUTE_POSITION),
        mesh.try_indices_option(),
        mesh.try_has_morph_targets(),
    ) else {
        return Vec::new();
    };
    if mesh.primitive_topology() != PrimitiveTopology::TriangleList {
        return Vec::new();
    }
    let indices: Vec<u32> = indices.iter().map(|index| index as u32).collect();
    let mut levels = Vec::new();
    let mut triangles = indices.len() / 3;
    loop {
        let target = triangles / 2;
        if target == 0 {
            return levels;
        }
        let mut strays = 0.0;
        let kept = meshopt::simplify_decoder(
            &indices,
            positions,
            target * 3,
            f32::MAX,
            SimplifyOptions::Permissive | SimplifyOptions::ErrorAbsolute,
            Some(&mut strays),
        );
        let kept_triangles = kept.len() / 3;
        if kept_triangles == 0 || kept_triangles * 4 > triangles * 3 {
            return levels;
        }
        levels.push((level(mesh, kept), strays));
        triangles = kept_triangles;
    }
}

/// `mesh` drawn by `indices` alone, holding only the vertices they use.
fn level(mesh: &Mesh, indices: Vec<u32>) -> Mesh {
    let mut level = mesh.clone();
    level.asset_usage = RenderAssetUsages::RENDER_WORLD;
    level.insert_indices(Indices::U32(indices));
    level.duplicate_vertices();
    level
        .merge_duplicate_vertices()
        .expect("a mesh just stripped of its indices");
    level.compress_indices();
    level
}
