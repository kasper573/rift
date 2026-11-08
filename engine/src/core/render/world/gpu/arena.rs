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
use bevy::render::render_resource::{BufferUsages, VertexAttribute, VertexFormat};
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::sync_component::SyncComponent;
use bevy::render::{Extract, ExtractSchedule, MainWorld, Render, RenderApp, RenderSystems};
use bevy::tasks::futures::check_ready;
use bevy::tasks::{AsyncComputeTaskPool, Task};

use super::impostor::AtlasKey;
use super::slab::{Slab, SlabBlock, SlabSpace};
use super::upload::UploadBudget;
use crate::core::render::world::lod::{self, Surface};
use crate::core::render::world::model::{self, Piece};
use crate::core::units::Meters;

pub struct ModelArenaPlugin;

impl Plugin for ModelArenaPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ExtractComponentPlugin::<PropArena>::default())
            .init_resource::<PropModels>()
            .add_systems(Last, (forget_models, prepare_models, send_models).chain());
        if app.get_sub_app(RenderApp).is_some() {
            app.init_resource::<InstanceChanges>();
            app.sub_app_mut(RenderApp)
                .init_resource::<ModelArena>()
                .init_resource::<InstanceArenas>()
                .add_systems(ExtractSchedule, (extract_models, extract_instances))
                .add_systems(
                    Render,
                    (write_models, write_instances)
                        .after(RenderSystems::PrepareViews)
                        .before(RenderSystems::Queue),
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

    /// The vertex as a pipeline reads it from the slab: position, normal, uv, colour.
    pub const ATTRIBUTES: [VertexAttribute; 4] = [
        VertexAttribute {
            format: VertexFormat::Float32x3,
            offset: 0,
            shader_location: 0,
        },
        VertexAttribute {
            format: VertexFormat::Float32x3,
            offset: 12,
            shader_location: 1,
        },
        VertexAttribute {
            format: VertexFormat::Float32x2,
            offset: 24,
            shader_location: 2,
        },
        VertexAttribute {
            format: VertexFormat::Float32x4,
            offset: 32,
            shader_location: 3,
        },
    ];
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
        let mut indices = SlabSpace::new(INDEX_BYTES);
        let shapes = indices.alloc(SHAPES.len() as u64 * INDEX_BYTES);
        debug_assert_eq!(indices.range(shapes).map(|range| range.start), Some(0));
        ModelSlabs {
            vertices: SlabSpace::new(ModelVertex::BYTES),
            indices,
            models: HashMap::new(),
        }
    }
}

/// The indices of the shapes the impostor stages pull their vertices by, at the start of the
/// index slab: a quad of two triangles over four corners, then a box of six faces over four
/// corners each.
pub const SHAPES: [u32; 42] = [
    0, 1, 2, 2, 1, 3, //
    0, 1, 2, 2, 1, 3, 4, 5, 6, 6, 5, 7, 8, 9, 10, 10, 9, 11, //
    12, 13, 14, 14, 13, 15, 16, 17, 18, 18, 17, 19, 20, 21, 22, 22, 21, 23,
];

/// Where the shapes' indices lie among [`SHAPES`]: the quad's, then the box's.
pub const QUAD_INDICES: std::ops::Range<u32> = 0..6;
pub const BOX_INDICES: std::ops::Range<u32> = 6..42;

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

    /// Every model held, by its key.
    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.models.keys()
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

    /// The record as the slab holds it.
    pub fn to_bytes(&self) -> Vec<u8> {
        let floats = self
            .position
            .into_iter()
            .chain([self.yaw, self.scale])
            .flat_map(f32::to_le_bytes);
        let words = [self.prop, self.seed, self.chunk]
            .into_iter()
            .flat_map(u32::to_le_bytes);
        floats.chain(words).collect()
    }
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
    /// How far its shaders draw past its model at most, in the model's metres.
    pub overhang: Meters,
    /// How translucent its materials are at most.
    pub translucency: f32,
    /// The atlas it is drawn from afar as a picture of, none for a cover's copy.
    pub impostor: Option<AtlasKey>,
}

/// What a chunk's instances are, as culling reads them: the box their places fill, the largest
/// scale among them, and each prop among them with its model and how many instances it has.
#[derive(Clone, Debug, PartialEq)]
pub struct ChunkContents {
    pub low: Vec3,
    pub high: Vec3,
    pub largest_scale: f32,
    pub props: Vec<(u32, AssetId<Gltf>, u32)>,
    /// Where the chunk stands: near, over the chunk of ground `index` among the area's, or far,
    /// its instances each carrying the index of the near chunk they lie in.
    pub place: ChunkPlace,
}

/// Where a chunk of the arena stands: near, over one chunk of the ground, drawn by whatever its
/// instances' sizes ask, or far, drawn only where the near chunk an instance lies in doesn't
/// stand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChunkPlace {
    Near { index: u32 },
    Far,
}

impl ChunkContents {
    /// What `records` are, each prop's model by `model_of`.
    pub fn of(
        records: &[InstanceRecord],
        model_of: impl Fn(u32) -> AssetId<Gltf>,
        place: ChunkPlace,
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
            place,
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
    /// Places `records` as `chunk`'s instances, standing at `place`, in place of those it held,
    /// each prop's model by `model_of`: the change that writes them, none when there are none to
    /// write.
    pub fn insert(
        &mut self,
        chunk: u32,
        records: &[InstanceRecord],
        model_of: impl Fn(u32) -> AssetId<Gltf>,
        place: ChunkPlace,
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
            contents: Arc::new(ChunkContents::of(records, model_of, place)),
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

    /// Where `chunk`'s first instance lies among the arena's, if it holds any.
    pub fn first_of(&self, chunk: u32) -> Option<u32> {
        let (block, _) = self.chunks.get(&chunk)?.as_ref()?;
        let range = self.space.range(*block)?;
        Some((range.start / InstanceRecord::BYTES) as u32)
    }

    /// The chunk the instance at `instance` among the arena's belongs to, and its place among the
    /// chunk's instances.
    pub fn chunk_of(&self, instance: u32) -> Option<(u32, u32)> {
        self.chunks.iter().find_map(|(&chunk, held)| {
            let (block, count) = held.as_ref()?;
            let first = (self.space.range(*block)?.start / InstanceRecord::BYTES) as u32;
            (first..first + count)
                .contains(&instance)
                .then(|| (chunk, instance - first))
        })
    }
}

/// The changes the sceneries made to their arenas since the render world last took them, each by
/// the scenery it belongs to, in order. Held only where a render world takes them.
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

    /// How big `gltf`'s model is, once it is prepared.
    pub fn size(&self, gltf: AssetId<Gltf>) -> Option<ModelSize> {
        let bounds = match self.models.get(&gltf)? {
            ModelState::Prepared { model, .. } => model.bounds,
            ModelState::Sent(_, bounds) => *bounds,
            _ => return None,
        };
        Some(ModelSize {
            extent: Meters(2.0 * bounds.half_extents.max_element()),
            radius: Meters(Vec3::from(bounds.half_extents).length()),
        })
    }

    /// The material of each part of `gltf`'s model, once the model is on the GPU or on its way
    /// there this frame.
    pub fn parts(&self, gltf: AssetId<Gltf>) -> Option<&[Option<Handle<GltfMaterial>>]> {
        match self.models.get(&gltf)? {
            ModelState::Sent(parts, _) => Some(parts),
            _ => None,
        }
    }
}

/// How big a prop model is: how wide across its widest side, and the radius of the sphere its
/// impostor is framed to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ModelSize {
    pub extent: Meters,
    pub radius: Meters,
}

impl ModelSize {
    /// The size grown `by` on every side, as a prop's shaders draw past its model.
    pub fn grown(self, by: Meters) -> ModelSize {
        ModelSize {
            extent: Meters(self.extent.0 + 2.0 * by.0),
            radius: Meters(self.radius.0 + by.0),
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
    /// On the GPU, with the material of each of its parts and its bounds.
    Sent(Arc<[Option<Handle<GltfMaterial>>]>, Aabb),
}

/// Lets go of the models whose asset is gone, on the GPU as well.
fn forget_models(mut events: MessageReader<AssetEvent<Gltf>>, mut models: ResMut<PropModels>) {
    models.outgoing.clear();
    for event in events.read() {
        if let AssetEvent::Removed { id } | AssetEvent::Unused { id } = *event
            && let Some(state) = models.models.remove(&id)
            && matches!(state, ModelState::Sent(..))
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
                        None => ModelState::Sent(Arc::new([]), Aabb::default()),
                    };
                }
            }
            _ => {}
        }
    }
}

/// Sends the prepared models to the GPU, as many as what is left of the frame's budget holds.
pub(super) fn send_models(mut models: ResMut<PropModels>, mut budget: UploadBudget) {
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
            prepared.bounds,
        );
    }
}

fn extract_models(models: Extract<Res<PropModels>>, mut arena: ResMut<ModelArena>) {
    arena.incoming.extend(models.outgoing.iter().cloned());
}

/// Writes the models sent this frame into the slabs, and lets go of those let go.
pub(super) fn write_models(
    mut arena: ResMut<ModelArena>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
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
    let first = indices.buffer().is_none();
    indices.fit(&device, &queue, index_bytes);
    if first {
        let shapes: Vec<u8> = SHAPES
            .iter()
            .flat_map(|index| index.to_le_bytes())
            .collect();
        indices.write(&queue, 0, &shapes);
    }
    for written in writes {
        for (offset, bytes) in written.vertices {
            vertices.write(&queue, offset, &bytes);
        }
        for (offset, bytes) in written.indices {
            indices.write(&queue, offset, &bytes);
        }
    }
}

/// Takes the changes the sceneries made since the last frame was extracted, whenever they were
/// made.
fn extract_instances(mut main: ResMut<MainWorld>, mut arenas: ResMut<InstanceArenas>) {
    let changes = std::mem::take(
        &mut main
            .resource_mut::<InstanceChanges>()
            .bypass_change_detection()
            .0,
    );
    for (scenery, change) in changes {
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
pub(super) fn write_instances(
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
    records.iter().flat_map(InstanceRecord::to_bytes).collect()
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
