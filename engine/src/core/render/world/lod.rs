//! Coarser stand-ins for the meshes of the world, each with how far its shape strays from the
//! mesh's own, so a mesh far off the eye draws with no more triangles than its pixels show.

use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::ecs::system::SystemParam;
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::tasks::futures::check_ready;
use bevy::tasks::{AsyncComputeTaskPool, Task};
use meshopt::SimplifyOptions;

use crate::core::render::camera::View;
use crate::core::units::Meters;

/// Simplifies the meshes of the world into their stand-ins, each once, off the frame.
#[derive(SystemParam)]
pub struct Simplifier<'w> {
    levels: ResMut<'w, MeshLevels>,
    meshes: Res<'w, Assets<Mesh>>,
}

/// The levels of every mesh simplified so far, or being simplified, by the mesh they stand in
/// for. It changes as each mesh's levels are made.
#[derive(Resource, Default)]
pub struct MeshLevels(HashMap<AssetId<Mesh>, Simplifying>);

enum Simplifying {
    Making(Task<Vec<(Mesh, f32)>>),
    Made(Arc<Levels>),
}

/// The stand-ins for a mesh, finest first, each with about half the triangles of the one before
/// and how far its shape strays from the mesh's, in the mesh's own units.
pub struct Levels(Vec<(Handle<Mesh>, f32)>);

impl Simplifier<'_> {
    /// Starts simplifying `mesh` the first time it is asked for, once it has loaded.
    pub fn simplify(&mut self, mesh: &Handle<Mesh>) {
        if self.levels.0.contains_key(&mesh.id()) {
            return;
        }
        let Some(full) = self.meshes.get(mesh) else {
            return;
        };
        let full = full.clone();
        let task = AsyncComputeTaskPool::get().spawn(async move { simplified(&full) });
        self.levels
            .bypass_change_detection()
            .0
            .insert(mesh.id(), Simplifying::Making(task));
    }

    pub fn levels(&self) -> &MeshLevels {
        &self.levels
    }
}

impl MeshLevels {
    /// The levels of `mesh`, once they are made.
    pub fn of(&self, mesh: &Handle<Mesh>) -> Option<&Levels> {
        match self.0.get(&mesh.id())? {
            Simplifying::Made(levels) => Some(levels),
            Simplifying::Making(_) => None,
        }
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

/// Takes in the levels simplified since the last frame.
pub(super) fn finish_simplifying(mut levels: ResMut<MeshLevels>, mut meshes: ResMut<Assets<Mesh>>) {
    let mut finished = false;
    for simplifying in levels.bypass_change_detection().0.values_mut() {
        if let Simplifying::Making(task) = simplifying
            && let Some(made) = check_ready(task)
        {
            *simplifying = Simplifying::Made(Arc::new(Levels(
                made.into_iter()
                    .map(|(level, strays)| (meshes.add(level), strays))
                    .collect(),
            )));
            finished = true;
        }
    }
    if finished {
        levels.set_changed();
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
pub(super) const STRAY_PIXELS: f32 = 0.5;

/// The stand-ins for `mesh`, as [`simplify`] makes them; none for a mesh that isn't a list of
/// triangles over positions, that morphs, or whose data only the GPU holds.
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
    simplify(&indices, positions)
        .into_iter()
        .map(|(kept, strays)| (level(mesh, kept), strays))
        .collect()
}

/// The triangles of the mesh `indices` draw over `positions`, coarser and coarser: halved each
/// time while the simplifier still can, collapsing across the seams of the mesh's texture
/// coordinates and normals as well, which a shape seen at under a pixel's stray doesn't show,
/// each with how far its shape strays from the mesh's, in the mesh's own units.
pub(super) fn simplify(indices: &[u32], positions: &[[f32; 3]]) -> Vec<(Vec<u32>, f32)> {
    let mut levels = Vec::new();
    let mut triangles = indices.len() / 3;
    loop {
        let target = triangles / 2;
        if target == 0 {
            return levels;
        }
        let mut strays = 0.0;
        let kept = meshopt::simplify_decoder(
            indices,
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
        levels.push((kept, strays));
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
