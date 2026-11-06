//! The world's props as the GPU holds them. Every prop model's meshes lie in one vertex slab and
//! one index slab, a part for each material the model is drawn in, each at levels from its full
//! mesh down to coarser stand-ins over the same vertices. A model is prepared off the frame once
//! it is wanted and loaded, sent to the GPU within the frame's upload budget, and let go when its
//! asset goes. Each scenery's prop instances lie in an arena of its own, chunk by chunk, each
//! chunk's in a block of the arena's slab that it keeps while it stands, sent within the same
//! budget.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;

use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::RenderLayers;
use bevy::ecs::query::QueryItem;
use bevy::gltf::{Gltf, GltfMaterial, GltfMesh, GltfNode};
use bevy::prelude::*;
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::render_resource::BufferUsages;
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::sync_component::SyncComponent;
use bevy::render::{Extract, ExtractSchedule, Render, RenderApp, RenderSystems};
use bevy::tasks::futures::check_ready;
use bevy::tasks::{AsyncComputeTaskPool, Task};

use super::slab::{Slab, SlabBlock, SlabSpace};
use super::upload::UploadBudget;
use crate::core::render::world::lod::{self, Surface};
use crate::core::render::world::model::{self, Piece};

pub struct ModelArenaPlugin;

impl Plugin for ModelArenaPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ExtractComponentPlugin::<PropArena>::default())
            .init_resource::<PropModels>()
            .init_resource::<InstanceChanges>()
            .add_systems(First, clear_instance_changes)
            .add_systems(Last, (forget_models, prepare_models, send_models).chain());
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app
                .init_resource::<ModelArena>()
                .init_resource::<InstanceArenas>()
                .add_systems(ExtractSchedule, (extract_models, extract_instances))
                .add_systems(
                    Render,
                    (write_models, write_instances).in_set(RenderSystems::PrepareResources),
                );
        }
    }
}

/// A scenery whose props are drawn from its arena, on its root.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct PropArena;

/// A scenery's arena as the render world draws it: on the layers the scenery shows on.
#[derive(Component, Clone, Debug)]
pub struct ExtractedPropArena {
    pub layers: RenderLayers,
}

impl SyncComponent<RenderApp> for PropArena {
    type Target = ExtractedPropArena;
}

impl ExtractComponent<RenderApp> for PropArena {
    type QueryData = Option<&'static RenderLayers>;
    type QueryFilter = With<PropArena>;
    type Out = ExtractedPropArena;

    fn extract_component(layers: QueryItem<'_, '_, Self::QueryData>) -> Option<ExtractedPropArena> {
        Some(ExtractedPropArena {
            layers: layers.cloned().unwrap_or_default(),
        })
    }
}

/// A vertex of a model as the slab holds it: where it stands in the model, the way it faces, its
/// texture coordinates and its colour, white where the model gives none.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub color: [f32; 4],
}

impl ModelVertex {
    pub const BYTES: u64 = 48;
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
        let drawn = model::by_material(pieces);
        let positions = drawn
            .iter()
            .flat_map(|drawn| drawn.vertices.positions.iter().copied());
        let bounds = Aabb::enclosing(positions)?;
        let parts = drawn
            .into_iter()
            .map(|model::DrawnIn { material, vertices }| {
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
                            color: vertices
                                .colors
                                .as_ref()
                                .map_or([1.0; 4], |colors| colors[vertex]),
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

/// A prop standing in the world as the arenas hold it: where it stands, its turn about the up axis
/// and its size, the prop it is by its place among the content's props, a seed of its own, and
/// the chunk of its scenery it stands in, by the scenery's own key for it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InstanceRecord {
    pub position: [f32; 3],
    pub yaw: f32,
    pub scale: f32,
    pub prop: u32,
    pub seed: u32,
    pub chunk: u32,
}

impl InstanceRecord {
    pub const BYTES: u64 = 32;
}

/// Where one scenery's prop instances lie in its arena's slab, chunk by chunk, each chunk by a key
/// of the scenery's own: a chunk's instances in a block of their own, kept where it is for as long
/// as the chunk stands, and none for a chunk standing with no props.
pub struct InstanceArena {
    space: SlabSpace,
    chunks: HashMap<u32, Option<(SlabBlock, u32)>>,
}

/// A change to a scenery's arena the render world takes in: a chunk's instances written at an
/// offset, with the bytes the slab must hold for them; a chunk let go; a prop dressed or its dress
/// let go; or the whole arena gone.
#[derive(Clone)]
pub enum InstanceChange {
    Placed {
        chunk: u32,
        offset: u64,
        bytes: Arc<[u8]>,
        instances: u32,
        capacity: u64,
        contents: Arc<ChunkContents>,
    },
    Removed {
        chunk: u32,
    },
    /// A prop dressed: each part of its model in its world material, and whether it casts a
    /// shadow.
    Dressed {
        prop: u32,
        dressed: Arc<DressedProp>,
    },
    /// A prop no chunk stands any more, its dress let go.
    Undressed {
        prop: u32,
    },
    Dropped,
}

/// How a scenery draws a prop: each part of its model in its material, of whatever kind draws it,
/// whether it casts a shadow, and whether it clears away where it hides the camera's subject.
#[derive(Clone, Debug)]
pub struct DressedProp {
    pub materials: Vec<UntypedHandle>,
    pub shadow: bool,
    pub fades: bool,
}

/// What a chunk's instances are, as culling reads them: the box their places fill, the largest
/// scale among them, and each prop among them with its model and how many instances it has.
#[derive(Clone, Debug, PartialEq)]
pub struct ChunkContents {
    pub low: Vec3,
    pub high: Vec3,
    pub largest_scale: f32,
    pub props: Vec<(u32, AssetId<Gltf>, u32)>,
}

impl ChunkContents {
    /// What `records` are, each prop's model by `model_of`.
    pub fn of(
        records: &[InstanceRecord],
        model_of: impl Fn(u32) -> AssetId<Gltf>,
    ) -> ChunkContents {
        let mut props: Vec<(u32, AssetId<Gltf>, u32)> = Vec::new();
        let mut low = Vec3::INFINITY;
        let mut high = Vec3::NEG_INFINITY;
        let mut largest_scale = 0.0f32;
        for record in records {
            let at = Vec3::from_array(record.position);
            low = low.min(at);
            high = high.max(at);
            largest_scale = largest_scale.max(record.scale);
            match props.iter_mut().find(|(prop, _, _)| *prop == record.prop) {
                Some((_, _, instances)) => *instances += 1,
                None => props.push((record.prop, model_of(record.prop), 1)),
            }
        }
        ChunkContents {
            low,
            high,
            largest_scale,
            props,
        }
    }
}

impl Default for InstanceArena {
    fn default() -> InstanceArena {
        InstanceArena {
            space: SlabSpace::new(InstanceRecord::BYTES),
            chunks: HashMap::new(),
        }
    }
}

impl InstanceArena {
    /// Places `records` as `chunk`'s instances, in place of those it held, each prop's model by
    /// `model_of`: the change that writes them, none when there are none to write.
    pub fn insert(
        &mut self,
        chunk: u32,
        records: &[InstanceRecord],
        model_of: impl Fn(u32) -> AssetId<Gltf>,
    ) -> Option<InstanceChange> {
        let removed = self.remove(chunk);
        if records.is_empty() {
            self.chunks.insert(chunk, None);
            return removed;
        }
        let bytes = instance_bytes(records);
        let block = self.space.alloc(bytes.len() as u64);
        let offset = self
            .space
            .range(block)
            .expect("a block just given out has its range")
            .start;
        self.chunks
            .insert(chunk, Some((block, records.len() as u32)));
        Some(InstanceChange::Placed {
            chunk,
            offset,
            bytes: bytes.into(),
            instances: records.len() as u32,
            capacity: self.space.capacity(),
            contents: Arc::new(ChunkContents::of(records, model_of)),
        })
    }

    /// Lets go of `chunk`'s instances: the change that lets them go, none when it held none.
    pub fn remove(&mut self, chunk: u32) -> Option<InstanceChange> {
        let (block, _) = self.chunks.remove(&chunk)??;
        self.space.free(block);
        Some(InstanceChange::Removed { chunk })
    }

    /// Whether `chunk`'s instances stand in the arena, if it has any.
    pub fn holds(&self, chunk: u32) -> bool {
        self.chunks.contains_key(&chunk)
    }

    /// The chunks standing, by their keys.
    pub fn chunks(&self) -> impl Iterator<Item = u32> + '_ {
        self.chunks.keys().copied()
    }
}

/// The changes the sceneries made to their arenas this frame, each by the scenery it belongs to,
/// in order, for the render world to take in.
#[derive(Resource, Default)]
pub struct InstanceChanges(pub Vec<(Entity, InstanceChange)>);

/// Each scenery's arena as the GPU holds it, by the scenery: its slab, each chunk's instances in
/// it, and how many times its chunks have changed.
#[derive(Resource, Default)]
pub struct InstanceArenas(pub HashMap<Entity, GpuInstances>);

pub struct GpuInstances {
    pub slab: Slab,
    /// How far each instance in the slab shows, by its place there: 1 whole, 0 cleared away
    /// where it hides the camera's subject. An instance placed starts whole.
    pub shown: Slab,
    pub chunks: HashMap<u32, GpuChunk>,
    pub dressed: HashMap<u32, Arc<DressedProp>>,
    pub changes: u64,
    pending: Vec<(u64, Arc<[u8]>)>,
    capacity: u64,
}

/// Where a chunk's instances lie in its arena's slab, in bytes, how many there are, and what
/// they are.
pub struct GpuChunk {
    pub offset: u64,
    pub instances: u32,
    pub contents: Arc<ChunkContents>,
}

impl Default for GpuInstances {
    fn default() -> GpuInstances {
        GpuInstances {
            slab: Slab::new(
                "prop instances",
                BufferUsages::STORAGE | BufferUsages::VERTEX,
            ),
            shown: Slab::new("prop instances shown", BufferUsages::STORAGE),
            chunks: HashMap::new(),
            dressed: HashMap::new(),
            changes: 0,
            pending: Vec::new(),
            capacity: 0,
        }
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

    /// The material of each part of `gltf`'s model, once the model is on the GPU or on its way
    /// there this frame.
    pub fn parts(&self, gltf: AssetId<Gltf>) -> Option<&[Option<Handle<GltfMaterial>>]> {
        match self.models.get(&gltf)? {
            ModelState::Sent(parts) => Some(parts),
            _ => None,
        }
    }
}

/// The models the GPU holds, as [`ModelSlabs`] places them, the slabs' buffers, and how many
/// times the models held have changed.
#[derive(Resource)]
pub struct ModelArena {
    pub slabs: ModelSlabs<AssetId<Gltf>>,
    pub vertices: Slab,
    pub indices: Slab,
    pub changes: u64,
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
            changes: 0,
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
    /// On the GPU, with the material of each of its parts.
    Sent(Arc<[Option<Handle<GltfMaterial>>]>),
}

/// Lets go of the models whose asset is gone, on the GPU as well.
fn forget_models(mut events: MessageReader<AssetEvent<Gltf>>, mut models: ResMut<PropModels>) {
    models.outgoing.clear();
    for event in events.read() {
        if let AssetEvent::Removed { id } | AssetEvent::Unused { id } = *event
            && let Some(state) = models.models.remove(&id)
            && matches!(state, ModelState::Sent(_))
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
                    .and_then(|loaded| model::pieces_of(loaded, &nodes, &gltf_meshes, &meshes))
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
                        None => ModelState::Sent(Arc::new([])),
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
        *state = ModelState::Sent(
            prepared
                .parts
                .iter()
                .map(|part| part.material.clone())
                .collect(),
        );
    }
}

fn extract_models(models: Extract<Res<PropModels>>, mut arena: ResMut<ModelArena>) {
    arena.incoming.extend(models.outgoing.iter().cloned());
}

/// Writes the models sent this frame into the slabs, and lets go of those let go.
fn write_models(mut arena: ResMut<ModelArena>, device: Res<RenderDevice>, queue: Res<RenderQueue>) {
    if arena.incoming.is_empty() {
        return;
    }
    arena.changes += 1;
    let ModelArena {
        slabs,
        vertices,
        indices,
        incoming,
        ..
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

fn clear_instance_changes(mut changes: ResMut<InstanceChanges>) {
    changes.0.clear();
}

fn extract_instances(changes: Extract<Res<InstanceChanges>>, mut arenas: ResMut<InstanceArenas>) {
    for (scenery, change) in changes.0.iter().cloned() {
        match change {
            InstanceChange::Placed {
                chunk,
                offset,
                bytes,
                instances,
                capacity,
                contents,
            } => {
                let arena = arenas.0.entry(scenery).or_default();
                arena.chunks.insert(
                    chunk,
                    GpuChunk {
                        offset,
                        instances,
                        contents,
                    },
                );
                arena.changes += 1;
                arena.pending.push((offset, bytes));
                arena.capacity = arena.capacity.max(capacity);
            }
            InstanceChange::Removed { chunk } => {
                if let Some(arena) = arenas.0.get_mut(&scenery) {
                    arena.chunks.remove(&chunk);
                    arena.changes += 1;
                }
            }
            InstanceChange::Dressed { prop, dressed } => {
                let arena = arenas.0.entry(scenery).or_default();
                arena.dressed.insert(prop, dressed);
                arena.changes += 1;
            }
            InstanceChange::Undressed { prop } => {
                if let Some(arena) = arenas.0.get_mut(&scenery) {
                    arena.dressed.remove(&prop);
                    arena.changes += 1;
                }
            }
            InstanceChange::Dropped => {
                arenas.0.remove(&scenery);
            }
        }
    }
}

/// Writes the instances placed this frame into each arena's slab, grown first to hold them.
fn write_instances(
    mut arenas: ResMut<InstanceArenas>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    for arena in arenas.0.values_mut() {
        if arena.pending.is_empty() {
            continue;
        }
        arena.slab.fit(&device, &queue, arena.capacity);
        arena
            .shown
            .fit(&device, &queue, shown_bytes(arena.capacity));
        for (offset, bytes) in arena.pending.drain(..) {
            arena.slab.write(&queue, offset, &bytes);
            let whole = 1.0f32.to_le_bytes();
            let shown: Vec<u8> = whole.repeat(bytes.len() / InstanceRecord::BYTES as usize);
            arena.shown.write(&queue, shown_bytes(offset), &shown);
        }
    }
}

/// Where the share shown of the instance `bytes` into the slab lies among the shares.
fn shown_bytes(bytes: u64) -> u64 {
    bytes / InstanceRecord::BYTES * 4
}

fn instance_bytes(records: &[InstanceRecord]) -> Vec<u8> {
    records
        .iter()
        .flat_map(|record| {
            let floats = record
                .position
                .into_iter()
                .chain([record.yaw, record.scale])
                .flat_map(f32::to_le_bytes);
            let words = [record.prop, record.seed, record.chunk]
                .into_iter()
                .flat_map(u32::to_le_bytes);
            floats.chain(words)
        })
        .collect()
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
                .chain(vertex.color)
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
