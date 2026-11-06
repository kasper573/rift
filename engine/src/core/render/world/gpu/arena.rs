//! The world's prop models as the GPU holds them: every model's meshes in one vertex slab and one
//! index slab, a part for each material the model is drawn in, each at levels from its full mesh
//! down to coarser stand-ins over the same vertices, so what draws a prop names its model and a
//! level and reads where they lie here. A model is prepared off the frame once it is wanted and
//! loaded, sent to the GPU within the frame's upload budget, and let go when its asset goes.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;

use bevy::camera::primitives::Aabb;
use bevy::gltf::{Gltf, GltfMaterial, GltfMesh, GltfNode};
use bevy::prelude::*;
use bevy::render::render_resource::BufferUsages;
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::{Extract, ExtractSchedule, Render, RenderApp, RenderSystems};
use bevy::tasks::futures::check_ready;
use bevy::tasks::{AsyncComputeTaskPool, Task};

use super::slab::{Slab, SlabBlock, SlabSpace};
use super::upload::UploadBudget;
use crate::core::render::world::cover::{self, Piece};
use crate::core::render::world::lod::{self, Surface};

pub struct ModelArenaPlugin;

impl Plugin for ModelArenaPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PropModels>()
            .add_systems(Last, (forget_models, prepare_models, send_models).chain());
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app
                .init_resource::<ModelArena>()
                .add_systems(ExtractSchedule, extract_models)
                .add_systems(Render, write_models.in_set(RenderSystems::PrepareResources));
        }
    }
}

/// A vertex of a model as the slab holds it: where it stands in the model, the way it faces and
/// its texture coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
}

impl ModelVertex {
    pub const BYTES: u64 = 32;
}

/// The bytes an index takes in the index slab.
pub const INDEX_BYTES: u64 = 4;

/// A model ready for the slabs: a part for each material it is drawn in, and the box it fills.
pub struct PreparedModel {
    pub parts: Vec<PreparedPart>,
    pub bounds: Aabb,
}

/// The pieces of a model drawn in one material, as one mesh, with its levels: the full mesh first,
/// then coarser and coarser over the same vertices, each with how far its shape strays from the
/// full one, in the model's own units.
pub struct PreparedPart {
    pub material: Option<Handle<GltfMaterial>>,
    pub vertices: Vec<ModelVertex>,
    pub levels: Vec<PreparedLevel>,
}

pub struct PreparedLevel {
    pub indices: Vec<u32>,
    pub strays: f32,
}

impl PreparedModel {
    /// The model of `pieces`, each carried through its transform, simplified level by level as
    /// [`lod`] simplifies every mesh of the world: worked out off the main thread. None when no
    /// piece holds a vertex.
    pub fn of(pieces: &[Piece]) -> Option<PreparedModel> {
        let (drawn, _) = cover::by_material(pieces);
        let positions = drawn
            .iter()
            .flat_map(|drawn| drawn.vertices.positions.iter().copied());
        let bounds = Aabb::enclosing(positions)?;
        let parts = drawn
            .into_iter()
            .map(|cover::DrawnIn { material, vertices }| {
                let positions: Vec<[f32; 3]> =
                    vertices.positions.iter().map(|at| at.to_array()).collect();
                let normals: Vec<[f32; 3]> =
                    vertices.normals.iter().map(|way| way.to_array()).collect();
                let surface = Surface {
                    uvs: Some(&vertices.uvs),
                    normals: Some(&normals),
                };
                let coarser = lod::simplify(&vertices.indices, &positions, surface);
                let levels = std::iter::once(PreparedLevel {
                    indices: vertices.indices.clone(),
                    strays: 0.0,
                })
                .chain(
                    coarser
                        .into_iter()
                        .map(|(indices, strays)| PreparedLevel { indices, strays }),
                )
                .collect();
                PreparedPart {
                    material,
                    vertices: (0..positions.len())
                        .map(|vertex| ModelVertex {
                            position: positions[vertex],
                            normal: normals[vertex],
                            uv: vertices.uvs[vertex],
                        })
                        .collect(),
                    levels,
                }
            })
            .collect();
        Some(PreparedModel { parts, bounds })
    }

    /// The bytes the model takes on the GPU.
    pub fn bytes(&self) -> u64 {
        self.parts
            .iter()
            .map(|part| {
                part.vertices.len() as u64 * ModelVertex::BYTES
                    + part
                        .levels
                        .iter()
                        .map(|level| level.indices.len() as u64 * INDEX_BYTES)
                        .sum::<u64>()
            })
            .sum()
    }
}

/// Where a model lies in the slabs, part by part, and the box it fills.
#[derive(Clone, Debug)]
pub struct PlacedModel {
    pub parts: Vec<PartRanges>,
    pub bounds: Aabb,
}

/// Where a part lies in the slabs: its first vertex among the vertex slab's and how many it has,
/// which its indices count from, and each of its levels' indices among the index slab's.
#[derive(Clone, Debug, PartialEq)]
pub struct PartRanges {
    pub material: Option<Handle<GltfMaterial>>,
    pub first_vertex: u32,
    pub vertices: u32,
    pub levels: Vec<LevelRange>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LevelRange {
    pub first_index: u32,
    pub indices: u32,
    pub strays: f32,
}

/// What the slabs' buffers take in for a model just placed: bytes, each at its offset.
#[derive(Default)]
pub struct SlabWrites {
    pub vertices: Vec<(u64, Vec<u8>)>,
    pub indices: Vec<(u64, Vec<u8>)>,
}

/// Where the models lie in a vertex slab and an index slab, each model by its `K`: a part's
/// vertices in a block of their own, and its levels' indices one after another in another, every
/// block kept where it is for as long as its model is.
pub struct ModelSlabs<K> {
    vertices: SlabSpace,
    indices: SlabSpace,
    models: HashMap<K, Held>,
}

struct Held {
    placed: PlacedModel,
    blocks: Vec<(SlabBlock, SlabBlock)>,
}

impl<K: Eq + Hash> Default for ModelSlabs<K> {
    fn default() -> ModelSlabs<K> {
        ModelSlabs {
            vertices: SlabSpace::new(ModelVertex::BYTES),
            indices: SlabSpace::new(INDEX_BYTES),
            models: HashMap::new(),
        }
    }
}

impl<K: Eq + Hash> ModelSlabs<K> {
    /// Places `model` as `key`, in place of the model `key` held before: the bytes to write for it.
    pub fn insert(&mut self, key: K, model: &PreparedModel) -> SlabWrites {
        self.remove(&key);
        let mut writes = SlabWrites::default();
        let mut parts = Vec::new();
        let mut blocks = Vec::new();
        for part in &model.parts {
            let vertex_bytes = vertex_bytes(&part.vertices);
            let vertex_block = self.vertices.alloc(vertex_bytes.len() as u64);
            let index_bytes = index_bytes(&part.levels);
            let index_block = self.indices.alloc(index_bytes.len() as u64);
            let (Some(vertex_range), Some(index_range)) = (
                self.vertices.range(vertex_block),
                self.indices.range(index_block),
            ) else {
                unreachable!("a block just given out has its range");
            };
            let mut first_index = (index_range.start / INDEX_BYTES) as u32;
            let levels = part
                .levels
                .iter()
                .map(|level| {
                    let range = LevelRange {
                        first_index,
                        indices: level.indices.len() as u32,
                        strays: level.strays,
                    };
                    first_index += range.indices;
                    range
                })
                .collect();
            parts.push(PartRanges {
                material: part.material.clone(),
                first_vertex: (vertex_range.start / ModelVertex::BYTES) as u32,
                vertices: part.vertices.len() as u32,
                levels,
            });
            writes.vertices.push((vertex_range.start, vertex_bytes));
            writes.indices.push((index_range.start, index_bytes));
            blocks.push((vertex_block, index_block));
        }
        let placed = PlacedModel {
            parts,
            bounds: model.bounds,
        };
        self.models.insert(key, Held { placed, blocks });
        writes
    }

    /// Lets go of the model `key` holds, its room given out again: false if it holds none.
    pub fn remove(&mut self, key: &K) -> bool {
        let Some(held) = self.models.remove(key) else {
            return false;
        };
        for (vertex_block, index_block) in held.blocks {
            self.vertices.free(vertex_block);
            self.indices.free(index_block);
        }
        true
    }

    pub fn get(&self, key: &K) -> Option<&PlacedModel> {
        self.models.get(key).map(|held| &held.placed)
    }

    /// The bytes the vertex slab's buffer must hold, and the index slab's, for every model to fit.
    pub fn capacity(&self) -> (u64, u64) {
        (self.vertices.capacity(), self.indices.capacity())
    }
}

/// The prop models the world wants on the GPU, each by its asset, as far as each has come: waiting
/// for its asset to load, being prepared, waiting for room in a frame's upload budget, or sent.
#[derive(Resource, Default)]
pub struct PropModels {
    models: HashMap<AssetId<Gltf>, ModelState>,
    /// What the render world takes in this frame: models sent, and models let go.
    outgoing: Vec<(AssetId<Gltf>, Option<Arc<PreparedModel>>)>,
}

impl PropModels {
    /// Wants `gltf`'s model on the GPU, from when it has loaded for as long as a handle to it is
    /// held.
    pub fn want(&mut self, gltf: &Handle<Gltf>) {
        self.models
            .entry(gltf.id())
            .or_insert_with(|| ModelState::Loading(gltf.clone()));
    }

    /// Whether `gltf`'s model is on the GPU, or on its way there this frame.
    pub fn sent(&self, gltf: AssetId<Gltf>) -> bool {
        matches!(self.models.get(&gltf), Some(ModelState::Sent))
    }
}

/// The models the GPU holds, as [`ModelSlabs`] places them, and the slabs' buffers.
#[derive(Resource)]
pub struct ModelArena {
    pub slabs: ModelSlabs<AssetId<Gltf>>,
    pub vertices: Slab,
    pub indices: Slab,
    incoming: Vec<(AssetId<Gltf>, Option<Arc<PreparedModel>>)>,
}

impl Default for ModelArena {
    fn default() -> ModelArena {
        ModelArena {
            slabs: ModelSlabs::default(),
            vertices: Slab::new(
                "model vertices",
                BufferUsages::VERTEX | BufferUsages::STORAGE,
            ),
            indices: Slab::new("model indices", BufferUsages::INDEX | BufferUsages::STORAGE),
            incoming: Vec::new(),
        }
    }
}

// Each state before `Sent` holds the asset's handle, so the asset stays loaded until its model is
// sent.
enum ModelState {
    Loading(Handle<Gltf>),
    Preparing(Handle<Gltf>, Task<Option<PreparedModel>>),
    Prepared {
        model: Arc<PreparedModel>,
        _asset: Handle<Gltf>,
    },
    Sent,
}

/// Lets go of the models whose asset is gone, on the GPU as well.
fn forget_models(mut events: MessageReader<AssetEvent<Gltf>>, mut models: ResMut<PropModels>) {
    models.outgoing.clear();
    for event in events.read() {
        if let AssetEvent::Removed { id } | AssetEvent::Unused { id } = *event
            && let Some(state) = models.models.remove(&id)
            && matches!(state, ModelState::Sent)
        {
            models.outgoing.push((id, None));
        }
    }
}

/// Starts preparing every wanted model whose asset has loaded, and takes in those prepared.
fn prepare_models(
    mut models: ResMut<PropModels>,
    assets: Res<AssetServer>,
    gltfs: Res<Assets<Gltf>>,
    nodes: Res<Assets<GltfNode>>,
    gltf_meshes: Res<Assets<GltfMesh>>,
    meshes: Res<Assets<Mesh>>,
) {
    for state in models.models.values_mut() {
        match state {
            ModelState::Loading(gltf) if assets.is_loaded_with_dependencies(gltf.id()) => {
                let Some(pieces) = gltfs
                    .get(gltf.id())
                    .and_then(|loaded| cover::pieces_of(loaded, &nodes, &gltf_meshes, &meshes))
                else {
                    continue;
                };
                let task =
                    AsyncComputeTaskPool::get().spawn(async move { PreparedModel::of(&pieces) });
                *state = ModelState::Preparing(gltf.clone(), task);
            }
            ModelState::Preparing(gltf, task) => {
                if let Some(prepared) = check_ready(task) {
                    *state = match prepared {
                        Some(prepared) => ModelState::Prepared {
                            model: Arc::new(prepared),
                            _asset: gltf.clone(),
                        },
                        None => ModelState::Sent,
                    };
                }
            }
            _ => {}
        }
    }
}

/// Sends the prepared models to the GPU, as many as what is left of the frame's budget holds.
fn send_models(mut models: ResMut<PropModels>, mut budget: UploadBudget) {
    let PropModels { models, outgoing } = &mut *models;
    for (&id, state) in models.iter_mut() {
        let ModelState::Prepared {
            model: prepared, ..
        } = state
        else {
            continue;
        };
        if !budget.spend(prepared.bytes() as usize) {
            break;
        }
        outgoing.push((id, Some(prepared.clone())));
        *state = ModelState::Sent;
    }
}

fn extract_models(models: Extract<Res<PropModels>>, mut arena: ResMut<ModelArena>) {
    arena.incoming.extend(models.outgoing.iter().cloned());
}

/// Writes the models sent this frame into the slabs, and lets go of those let go.
fn write_models(mut arena: ResMut<ModelArena>, device: Res<RenderDevice>, queue: Res<RenderQueue>) {
    let ModelArena {
        slabs,
        vertices,
        indices,
        incoming,
    } = &mut *arena;
    let mut writes = Vec::new();
    for (id, model) in incoming.drain(..) {
        match model {
            Some(model) => writes.push(slabs.insert(id, &model)),
            None => {
                slabs.remove(&id);
            }
        }
    }
    if writes.is_empty() {
        return;
    }
    let (vertex_bytes, index_bytes) = slabs.capacity();
    vertices.fit(&device, &queue, vertex_bytes);
    indices.fit(&device, &queue, index_bytes);
    for written in writes {
        for (offset, bytes) in written.vertices {
            vertices.write(&queue, offset, &bytes);
        }
        for (offset, bytes) in written.indices {
            indices.write(&queue, offset, &bytes);
        }
    }
}

fn vertex_bytes(vertices: &[ModelVertex]) -> Vec<u8> {
    vertices
        .iter()
        .flat_map(|vertex| {
            vertex
                .position
                .into_iter()
                .chain(vertex.normal)
                .chain(vertex.uv)
        })
        .flat_map(f32::to_le_bytes)
        .collect()
}

fn index_bytes(levels: &[PreparedLevel]) -> Vec<u8> {
    levels
        .iter()
        .flat_map(|level| level.indices.iter().copied())
        .flat_map(u32::to_le_bytes)
        .collect()
}
