//! Coarser stand-ins for the meshes of the world, each with how far its shape strays from the
//! mesh's own, so a mesh far off the eye draws with no more triangles than its pixels show.

use bevy::asset::RenderAssetUsages;
use bevy::ecs::system::SystemParam;
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::tasks::futures::check_ready;
use bevy::tasks::{AsyncComputeTaskPool, Task};
use meshopt::SimplifyOptions;

use crate::core::render::camera::Lens;
use crate::core::units::Meters;

/// Simplifies the meshes of the world into their stand-ins, each once, off the frame.
#[derive(SystemParam)]
pub struct Simplifier<'w> {
    levels: ResMut<'w, MeshLevels>,
    meshes: Res<'w, Assets<Mesh>>,
}

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

/// The levels of every mesh simplified so far, or being simplified, by the mesh they stand in
/// for. It changes as each mesh's levels are made.
#[derive(Resource, Default)]
pub struct MeshLevels(HashMap<AssetId<Mesh>, Simplifying>);

impl MeshLevels {
    /// The levels of `mesh`, once they are made.
    pub fn of(&self, mesh: &Handle<Mesh>) -> Option<&Levels> {
        match self.0.get(&mesh.id())? {
            Simplifying::Made(levels) => Some(levels),
            Simplifying::Making(_) => None,
        }
    }
}

/// The stand-ins for a mesh, finest first, each with about half the triangles of the one before
/// and how far its shape strays from the mesh's, in the mesh's own units.
pub struct Levels(Vec<(Handle<Mesh>, f32)>);

impl Levels {
    /// The coarsest stand-in that strays from the mesh by less than [`STRAY_PIXELS`] through
    /// `lens`, drawn `scale` times its size `distance` off the eye; none where only the mesh
    /// itself does.
    pub fn through(&self, lens: &Lens, distance: Meters, scale: f32) -> Option<&Handle<Mesh>> {
        let allowed = STRAY_PIXELS / (lens.pixels_per_meter(distance) * scale);
        self.0
            .iter()
            .rev()
            .find(|(_, strays)| *strays <= allowed)
            .map(|(level, _)| level)
    }
}

/// How many pixels across a thing among many alike must span for the view to tell it apart from
/// its neighbours: smaller, some of them may be dropped and the rest grown to this size, and the
/// crowd looks the same.
pub const GRAIN_PIXELS: f32 = 2.0;

/// Takes in the levels simplified since the last frame.
pub(super) fn finish_simplifying(mut levels: ResMut<MeshLevels>, mut meshes: ResMut<Assets<Mesh>>) {
    let mut finished = false;
    for simplifying in levels.bypass_change_detection().0.values_mut() {
        if let Simplifying::Making(task) = simplifying
            && let Some(made) = check_ready(task)
        {
            *simplifying = Simplifying::Made(Levels(
                made.into_iter()
                    .map(|(level, strays)| (meshes.add(level), strays))
                    .collect(),
            ));
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

enum Simplifying {
    Making(Task<Vec<(Mesh, f32)>>),
    Made(Levels),
}

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
    let surface = Surface {
        uvs: match mesh.try_attribute_option(Mesh::ATTRIBUTE_UV_0) {
            Ok(Some(VertexAttributeValues::Float32x2(uvs))) => Some(uvs.as_slice()),
            _ => None,
        },
        normals: match mesh.try_attribute_option(Mesh::ATTRIBUTE_NORMAL) {
            Ok(Some(VertexAttributeValues::Float32x3(normals))) => Some(normals.as_slice()),
            _ => None,
        },
    };
    simplify(&indices, positions, surface)
        .into_iter()
        .map(|(kept, strays)| (level(mesh, kept), strays))
        .collect()
}

/// How much a slide of the texture coordinates and a turn of the normals weigh against the
/// shape's own moves, within the range meshoptimizer recommends for attributes spanning 0 to 1.
const UV_WEIGHT: f32 = 0.1;

const NORMAL_WEIGHT: f32 = 0.05;

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
