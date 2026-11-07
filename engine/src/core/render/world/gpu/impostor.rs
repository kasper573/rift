//! A prop model as the far field draws it: an atlas of its pictures from every way round the
//! upper hemisphere, baked on the GPU once the model is in the slabs, and kept under the profile
//! so the next start reads it instead. A cell of the atlas looks at the middle of the model's
//! bounding sphere from one direction, orthographically, framed to the sphere, and holds what it
//! sees: the base colour with its coverage, the normal and the depth. The directions lie on a
//! hemi-octahedron, so a direction finds its cell by a fold and a cell its direction by the
//! unfold, and neighbouring cells look neighbouring ways.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use bevy::asset::AssetId;
use bevy::ecs::system::SystemParam;
use bevy::gltf::Gltf;
use bevy::math::{Mat4, UVec2, Vec2, Vec3, proj};
use bevy::prelude::*;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{
    sampler, storage_buffer_read_only_sized, texture_2d, uniform_buffer,
};
use bevy::render::render_resource::{
    BindGroupEntries, BindGroupLayout, BindGroupLayoutEntries, Buffer, BufferBinding,
    BufferDescriptor, BufferInitDescriptor, BufferUsages, ColorTargetState, ColorWrites,
    CommandEncoder, CompareFunction, DepthStencilState, Extent3d, FilterMode, IndexFormat, LoadOp,
    MapMode, MultisampleState, Operations, Origin3d, PipelineCompilationOptions,
    PipelineLayoutDescriptor, PrimitiveState, RawFragmentState, RawRenderPipelineDescriptor,
    RawVertexBufferLayout, RawVertexState, RenderPassColorAttachment,
    RenderPassDepthStencilAttachment, RenderPassDescriptor, RenderPipeline, Sampler,
    SamplerBindingType, SamplerDescriptor, ShaderModuleDescriptor, ShaderSource, ShaderStages,
    ShaderType, StoreOp, TexelCopyBufferInfo, TexelCopyBufferLayout, TexelCopyTextureInfo, Texture,
    TextureAspect, TextureDescriptor, TextureDimension, TextureFormat, TextureSampleType,
    TextureUsages, TextureView, TextureViewDescriptor, TextureViewDimension, VertexStepMode,
    encase,
};
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::texture::GpuImage;
use bevy::render::{Extract, ExtractSchedule, Render, RenderApp, RenderStartup, RenderSystems};
use bevy::tasks::futures::check_ready;
use bevy::tasks::{IoTaskPool, Task};
use serde::{Deserialize, Serialize};

use super::arena::{self, ModelArena, ModelVertex, PlacedModel};
use super::residency::Residency;
use super::upload::UploadBudget;
use crate::core::os::Profile;
use crate::core::render::world::RenderProgress;
use crate::core::render::world::lod::STRAY_PIXELS;

pub struct ImpostorPlugin;

impl Plugin for ImpostorPlugin {
    fn build(&self, app: &mut App) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        let baked = Baked::default();
        render_app
            .insert_resource(baked.clone())
            .init_resource::<ImpostorOrders>()
            .add_systems(RenderStartup, start)
            .add_systems(ExtractSchedule, extract_impostors)
            .add_systems(
                Render,
                (
                    prepare_impostors
                        .in_set(ImpostorsPrepared)
                        .after(arena::write_models)
                        .before(RenderSystems::Queue),
                    read_bakes.in_set(RenderSystems::Cleanup),
                ),
            );
        app.insert_resource(ImpostorStore::new(baked))
            .add_systems(Last, keep_impostors.after(arena::send_models));
    }
}

/// The frame's impostors given their layers and bakes, after which the atlas's layers stand for
/// the frame.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct ImpostorsPrepared;

/// How many models the GPU bakes in one frame, each about a millisecond of its time.
const BAKES_PER_FRAME: usize = 4;

/// The folder under the profile the atlases are kept in.
const IMPOSTORS: &str = "impostors";

/// The impostors of every model the world draws, as far as each has come: its file being read,
/// read and waiting for room in the frame's upload budget, waiting to be baked, or on the GPU,
/// baked or read. Held only beside a render world, which takes what it sends.
#[derive(Resource)]
pub struct ImpostorStore {
    models: HashMap<AssetId<Gltf>, ImpostorState>,
    outgoing: Vec<ImpostorOrder>,
    baked: Baked,
}

/// What the render world is told of a model's impostor: its atlas read from its file, to bake it,
/// or to let it go.
#[derive(Clone)]
pub enum ImpostorOrder {
    Read {
        model: AssetId<Gltf>,
        key: String,
        texels: Arc<ImpostorTexels>,
    },
    Bake {
        model: AssetId<Gltf>,
        key: String,
        materials: Arc<[BakeMaterial]>,
    },
    Forget {
        model: AssetId<Gltf>,
    },
}

enum ImpostorState {
    Reading {
        key: String,
        materials: Arc<[BakeMaterial]>,
        task: Task<Option<ImpostorTexels>>,
    },
    Read {
        key: String,
        texels: Arc<ImpostorTexels>,
    },
    ToBake {
        key: String,
        materials: Arc<[BakeMaterial]>,
    },
    Sent {
        key: String,
    },
}

/// The atlases the render world has baked and read back, for the files that keep them.
#[derive(Resource, Clone, Default)]
struct Baked(Arc<Mutex<Vec<(String, ImpostorTexels)>>>);

impl ImpostorStore {
    fn new(baked: Baked) -> ImpostorStore {
        ImpostorStore {
            models: HashMap::new(),
            outgoing: Vec::new(),
            baked,
        }
    }

    /// Wants `model`'s impostor on the GPU, baked from `materials` or read from the file kept
    /// under `key` in `profile`, until the model's asset goes. A model wanted again under another
    /// key is baked or read anew.
    pub fn want(
        &mut self,
        profile: &Profile,
        model: AssetId<Gltf>,
        key: String,
        materials: Arc<[BakeMaterial]>,
    ) {
        let current = self.models.get(&model).is_some_and(|state| match state {
            ImpostorState::Reading { key: held, .. }
            | ImpostorState::Read { key: held, .. }
            | ImpostorState::ToBake { key: held, .. }
            | ImpostorState::Sent { key: held } => *held == key,
        });
        if current {
            return;
        }
        let file = folder_under(profile).join(&key);
        let state = if file.is_file() {
            ImpostorState::Reading {
                key,
                materials,
                task: IoTaskPool::get()
                    .spawn(async move { ImpostorTexels::from_file(&std::fs::read(file).ok()?) }),
            }
        } else {
            ImpostorState::ToBake { key, materials }
        };
        self.models.insert(model, state);
    }

    /// How many models' impostors are still on their way to the GPU.
    pub fn pending(&self) -> usize {
        self.models
            .values()
            .filter(|state| !matches!(state, ImpostorState::Sent { .. }))
            .count()
    }
}

/// Moves every model's impostor along: files read taken in, atlases sent to the GPU within the
/// upload budget, bakes ordered, baked atlases written to their files, and models whose asset
/// went let go.
fn keep_impostors(
    mut store: ResMut<ImpostorStore>,
    profile: Res<Profile>,
    mut events: MessageReader<AssetEvent<Gltf>>,
    mut budget: UploadBudget,
) {
    store.outgoing.clear();
    for event in events.read() {
        if let AssetEvent::Removed { id } | AssetEvent::Unused { id } = *event
            && store.models.remove(&id).is_some()
        {
            store.outgoing.push(ImpostorOrder::Forget { model: id });
        }
    }
    let ImpostorStore {
        models,
        outgoing,
        baked,
    } = &mut *store;
    for (&model, state) in models.iter_mut() {
        match state {
            ImpostorState::Reading {
                key,
                materials,
                task,
            } => {
                if let Some(read) = check_ready(task) {
                    *state = match read {
                        Some(texels) => ImpostorState::Read {
                            key: key.clone(),
                            texels: Arc::new(texels),
                        },
                        None => ImpostorState::ToBake {
                            key: key.clone(),
                            materials: materials.clone(),
                        },
                    };
                }
            }
            ImpostorState::Read { key, texels } => {
                if budget.spend(texels.albedo.len() + texels.normal.len() + texels.depth.len()) {
                    outgoing.push(ImpostorOrder::Read {
                        model,
                        key: key.clone(),
                        texels: texels.clone(),
                    });
                    *state = ImpostorState::Sent { key: key.clone() };
                }
            }
            ImpostorState::ToBake { key, materials } => {
                outgoing.push(ImpostorOrder::Bake {
                    model,
                    key: key.clone(),
                    materials: materials.clone(),
                });
                *state = ImpostorState::Sent { key: key.clone() };
            }
            ImpostorState::Sent { .. } => {}
        }
    }
    let written: Vec<(String, ImpostorTexels)> =
        std::mem::take(&mut *baked.0.lock().expect("the baked atlases lock"));
    let dir = folder_under(&profile);
    for (key, texels) in written {
        let dir = dir.clone();
        IoTaskPool::get()
            .spawn(async move {
                let written = std::fs::create_dir_all(&dir)
                    .and_then(|()| std::fs::write(dir.join(&key), texels.to_file()));
                if let Err(error) = written {
                    warn!("impostor atlas not kept: {error}");
                }
            })
            .detach();
    }
}

/// The folder the atlases are kept in under `profile`.
fn folder_under(profile: &Profile) -> PathBuf {
    profile.0.join(IMPOSTORS)
}

/// What the render world has been told and not yet done: orders waiting for their model to be in
/// the slabs or for a layer, and bakes whose texels are on their way back.
#[derive(Resource, Default)]
struct ImpostorOrders {
    incoming: Vec<ImpostorOrder>,
    pending: Vec<ImpostorOrder>,
    readbacks: Vec<Readback>,
}

fn start(mut commands: Commands, device: Res<RenderDevice>, queue: Res<RenderQueue>) {
    commands.insert_resource(ImpostorAtlas::new(&device));
    commands.insert_resource(BakePass::new(&device, &queue));
}

fn extract_impostors(store: Extract<Res<ImpostorStore>>, mut orders: ResMut<ImpostorOrders>) {
    orders.incoming.extend(store.outgoing.iter().cloned());
}

/// Gives every atlas read its layer, bakes a few of those ordered whose model is in the slabs,
/// and lets go of the layers of models let go. While anything is left to do the frame is not
/// whole, so a veil holds until every impostor stands.
fn prepare_impostors(
    gpu: Gpu,
    atlas: Option<ResMut<ImpostorAtlas>>,
    pass: Option<Res<BakePass>>,
    mut orders: ResMut<ImpostorOrders>,
    progress: Res<RenderProgress>,
) {
    let (Some(mut atlas), Some(pass)) = (atlas, pass) else {
        return;
    };
    let Gpu {
        device,
        queue,
        models,
        images,
    } = gpu;
    atlas.layers.next_frame();
    for model in models.slabs.keys() {
        atlas.layers.want(*model);
    }
    let waiting: Vec<ImpostorOrder> = std::mem::take(&mut orders.pending)
        .into_iter()
        .chain(std::mem::take(&mut orders.incoming))
        .collect();
    let mut later = Vec::new();
    let mut bakes = Vec::new();
    for order in waiting {
        match order {
            ImpostorOrder::Forget { model } => {
                atlas.layers.release(model);
                atlas.round += 1;
            }
            ImpostorOrder::Read {
                model,
                ref key,
                ref texels,
            } => match atlas.layers.admit(model, key.clone()) {
                Some(admitted) => {
                    atlas.write(&queue, admitted.slot, texels);
                    atlas.round += 1;
                }
                None => later.push(order),
            },
            ImpostorOrder::Bake {
                model,
                ref key,
                ref materials,
            } => {
                let placed = models.slabs.get(&model);
                let admitted = placed
                    .filter(|_| bakes.len() < BAKES_PER_FRAME)
                    .and_then(|_| atlas.layers.admit(model, key.clone()));
                match (placed, admitted) {
                    (Some(placed), Some(admitted)) => {
                        atlas.round += 1;
                        bakes.push((model, key.clone(), materials.clone(), placed, admitted.slot));
                    }
                    _ => later.push(order),
                }
            }
        }
    }
    if !bakes.is_empty() {
        let mut encoder = device.create_command_encoder(&default());
        let drawn: Vec<Bake> = bakes
            .iter()
            .map(|(_, _, materials, placed, layer)| Bake {
                model: placed,
                materials,
                layer: *layer,
            })
            .collect();
        pass.encode(&device, &mut encoder, &atlas, &models, &images, &drawn);
        for (model, key, _, _, layer) in &bakes {
            orders.readbacks.push(Readback::of(
                &device,
                &mut encoder,
                &atlas,
                *layer,
                *model,
                key.clone(),
            ));
        }
        queue.submit([encoder.finish()]);
    }
    if !later.is_empty() || !bakes.is_empty() {
        progress.left_out();
    }
    orders.pending = later;
}

/// The device and what the bakes draw from: the models in the slabs and their textures.
#[derive(SystemParam)]
struct Gpu<'w> {
    device: Res<'w, RenderDevice>,
    queue: Res<'w, RenderQueue>,
    models: Res<'w, ModelArena>,
    images: Res<'w, RenderAssets<GpuImage>>,
}

/// Hands every baked atlas that has come back to the main world, for its file.
fn read_bakes(mut orders: ResMut<ImpostorOrders>, baked: Res<Baked>) {
    orders
        .readbacks
        .retain_mut(|readback| match readback.poll() {
            None => true,
            Some(Ok(texels)) => {
                baked
                    .0
                    .lock()
                    .expect("the baked atlases lock")
                    .push((readback.key.clone(), texels));
                false
            }
            Some(Err(())) => false,
        });
}

/// How many cells the atlas has along each side: a direction every `1 / (CELLS - 1)` of the
/// hemi-octahedron's unfolded square, its corners the horizon.
pub const CELLS: u32 = 8;

/// How many pixels a cell is across.
pub const CELL_PIXELS: u32 = 64;

/// How many pixels a model's atlas is across.
pub const ATLAS_PIXELS: u32 = CELLS * CELL_PIXELS;

/// The way a cell of the atlas looks from: toward the model's middle from `direction`, up the
/// model's y unless it looks straight down it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellView {
    pub cell: UVec2,
    pub direction: Vec3,
}

impl CellView {
    /// Every cell of the atlas with the direction it looks from, row by row.
    pub fn all() -> impl Iterator<Item = CellView> {
        (0..CELLS).flat_map(|row| {
            (0..CELLS).map(move |column| {
                let cell = UVec2::new(column, row);
                CellView {
                    cell,
                    direction: direction_of(cell),
                }
            })
        })
    }
}

/// The direction the cell at `cell` looks from: the hemi-octahedron unfolded over the atlas,
/// `(0, 0)` the horizon along -x, the middle cell straight up.
pub fn direction_of(cell: UVec2) -> Vec3 {
    let unit = cell.as_vec2() / (CELLS - 1) as f32;
    unfold(unit)
}

/// Where on the atlas's unfolded square a view from `direction` lies, in cells, for the
/// direction's picture to be read between the four cells around it. A direction below the
/// horizon reads as the horizon's.
pub fn cell_of(direction: Vec3) -> Vec2 {
    fold(direction) * (CELLS - 1) as f32
}

/// The direction the unfolded square's point `unit` (each coordinate in `0..=1`) stands for.
pub fn unfold(unit: Vec2) -> Vec3 {
    let folded = unit * 2.0 - Vec2::ONE;
    let x = (folded.x + folded.y) * 0.5;
    let z = (folded.x - folded.y) * 0.5;
    let y = 1.0 - x.abs() - z.abs();
    Vec3::new(x, y, z).normalize()
}

/// Where on the unfolded square (each coordinate in `0..=1`) `direction` lies.
pub fn fold(direction: Vec3) -> Vec2 {
    let above = Vec3::new(direction.x, direction.y.max(0.0), direction.z);
    let scaled = above / (above.x.abs() + above.y + above.z.abs()).max(1e-6);
    let folded = Vec2::new(scaled.x + scaled.z, scaled.x - scaled.z);
    ((folded + Vec2::ONE) * 0.5).clamp(Vec2::ZERO, Vec2::ONE)
}

/// What an atlas file under the profile begins with: how it was baked, so a file baked otherwise
/// is read as none. The texels follow the header's line: the albedo with its coverage (four bytes
/// a pixel), then the normal folded as a direction is (two), then the depth (one), each
/// `ATLAS_PIXELS` square, row by row.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ImpostorHeader {
    pub cells: u32,
    pub cell_pixels: u32,
    pub build: String,
}

impl ImpostorHeader {
    pub fn current() -> ImpostorHeader {
        ImpostorHeader {
            cells: CELLS,
            cell_pixels: CELL_PIXELS,
            build: crate::core::content::version::build().to_owned(),
        }
    }

    /// The bytes the texels after this header take.
    pub fn texel_bytes(&self) -> usize {
        let pixels = (self.cells * self.cell_pixels).pow(2) as usize;
        pixels * (ALBEDO_BYTES + NORMAL_BYTES + DEPTH_BYTES)
    }
}

pub const ALBEDO_BYTES: usize = 4;
pub const NORMAL_BYTES: usize = 2;
pub const DEPTH_BYTES: usize = 1;

/// The texels of a model's atlas as they are baked, read and kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImpostorTexels {
    pub albedo: Vec<u8>,
    pub normal: Vec<u8>,
    pub depth: Vec<u8>,
}

impl ImpostorTexels {
    /// The file that keeps these texels: the current header's line, then the texels.
    pub fn to_file(&self) -> Vec<u8> {
        let mut file = serde_json::to_vec(&ImpostorHeader::current()).expect("a header serializes");
        file.push(b'\n');
        file.extend_from_slice(&self.albedo);
        file.extend_from_slice(&self.normal);
        file.extend_from_slice(&self.depth);
        file
    }

    /// The texels `file` keeps, if it was baked as the current build bakes.
    pub fn from_file(file: &[u8]) -> Option<ImpostorTexels> {
        let line = file.iter().position(|&byte| byte == b'\n')?;
        let header: ImpostorHeader = serde_json::from_slice(&file[..line]).ok()?;
        if header != ImpostorHeader::current() {
            return None;
        }
        let texels = &file[line + 1..];
        if texels.len() != header.texel_bytes() {
            return None;
        }
        let pixels = (ATLAS_PIXELS * ATLAS_PIXELS) as usize;
        let (albedo, rest) = texels.split_at(pixels * ALBEDO_BYTES);
        let (normal, depth) = rest.split_at(pixels * NORMAL_BYTES);
        Some(ImpostorTexels {
            albedo: albedo.to_vec(),
            normal: normal.to_vec(),
            depth: depth.to_vec(),
        })
    }
}

/// The name a model's atlas is kept under: what the model is made of and how it is baked, so a
/// model changed, dressed otherwise or baked by another build bakes anew.
pub fn atlas_key(model_hash: &str, materials: &[BakeMaterialKey]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(model_hash.as_bytes());
    for material in materials {
        hasher.update(&material.color.map(f32::to_le_bytes).concat());
        hasher.update(material.texture.as_deref().unwrap_or("").as_bytes());
        hasher.update(&material.cutoff.to_le_bytes());
    }
    let header = ImpostorHeader::current();
    hasher.update(&header.cells.to_le_bytes());
    hasher.update(&header.cell_pixels.to_le_bytes());
    hasher.update(header.build.as_bytes());
    hasher.finalize().to_hex().to_string()
}

/// What of a part's material the bake shows, as the key tells it apart: the base colour, the
/// path of its texture, and the coverage below which a pixel is bare.
#[derive(Clone, Debug, PartialEq)]
pub struct BakeMaterialKey {
    pub color: [f32; 4],
    pub texture: Option<String>,
    pub cutoff: f32,
}

/// The most models whose atlases the GPU holds at once; past that the one wanted longest ago
/// gives its layer up, and its props draw their proxies until it is baked or read again.
pub const LAYERS: u32 = 128;

/// The albedo with its coverage, as the atlas and its files hold it.
pub const ALBEDO_FORMAT: TextureFormat = TextureFormat::Rgba8UnormSrgb;
/// The normal folded onto an octahedron.
pub const NORMAL_FORMAT: TextureFormat = TextureFormat::Rg8Unorm;
/// The depth across the model's bounding sphere, 0 nearest the cell's eye.
pub const DEPTH_FORMAT: TextureFormat = TextureFormat::R8Unorm;
const BAKE_DEPTH: TextureFormat = TextureFormat::Depth32Float;

/// The atlases of every model the GPU holds, a layer of each texture a model, and which model
/// holds which layer, by the key its atlas was baked or read under.
#[derive(Resource)]
pub struct ImpostorAtlas {
    pub albedo: Texture,
    pub normal: Texture,
    pub depth: Texture,
    /// The three textures whole, as the impostor stages read them, and how they are sampled.
    pub albedo_view: TextureView,
    pub normal_view: TextureView,
    pub depth_view: TextureView,
    pub sampler: Sampler,
    pub layers: Residency<AssetId<Gltf>, String>,
    /// Counts each time a layer is given or taken, for what tells the kernel which props have
    /// one.
    pub round: u64,
    bake_depth: Texture,
}

impl ImpostorAtlas {
    pub fn new(device: &RenderDevice) -> ImpostorAtlas {
        let texture = |label, format, usage| {
            device.create_texture(&TextureDescriptor {
                label: Some(label),
                size: Extent3d {
                    width: ATLAS_PIXELS,
                    height: ATLAS_PIXELS,
                    depth_or_array_layers: LAYERS,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
        };
        let baked = TextureUsages::RENDER_ATTACHMENT
            | TextureUsages::TEXTURE_BINDING
            | TextureUsages::COPY_SRC
            | TextureUsages::COPY_DST;
        let albedo = texture("impostor albedo", ALBEDO_FORMAT, baked);
        let normal = texture("impostor normals", NORMAL_FORMAT, baked);
        let depth = texture("impostor depths", DEPTH_FORMAT, baked);
        let whole = |texture: &Texture| {
            texture.create_view(&TextureViewDescriptor {
                label: Some("impostor atlas"),
                dimension: Some(TextureViewDimension::D2Array),
                ..default()
            })
        };
        ImpostorAtlas {
            albedo_view: whole(&albedo),
            normal_view: whole(&normal),
            depth_view: whole(&depth),
            albedo,
            normal,
            depth,
            sampler: device.create_sampler(&SamplerDescriptor {
                label: Some("impostor atlas"),
                mag_filter: FilterMode::Linear,
                min_filter: FilterMode::Linear,
                ..default()
            }),
            layers: Residency::new(LAYERS),
            round: 0,
            bake_depth: device.create_texture(&TextureDescriptor {
                label: Some("impostor bake depth"),
                size: Extent3d {
                    width: ATLAS_PIXELS,
                    height: ATLAS_PIXELS,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: BAKE_DEPTH,
                usage: TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            }),
        }
    }

    /// Writes `texels`, read from a file, into `layer`.
    pub fn write(&self, queue: &RenderQueue, layer: u32, texels: &ImpostorTexels) {
        for (texture, bytes, texel_bytes) in [
            (&self.albedo, &texels.albedo, ALBEDO_BYTES),
            (&self.normal, &texels.normal, NORMAL_BYTES),
            (&self.depth, &texels.depth, DEPTH_BYTES),
        ] {
            queue.write_texture(
                layer_copy(texture, layer),
                bytes,
                TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(ATLAS_PIXELS * texel_bytes as u32),
                    rows_per_image: Some(ATLAS_PIXELS),
                },
                layer_size(),
            );
        }
    }

    fn layer_view(texture: &Texture, layer: u32) -> TextureView {
        texture.create_view(&TextureViewDescriptor {
            label: Some("impostor layer"),
            dimension: Some(TextureViewDimension::D2),
            base_array_layer: layer,
            array_layer_count: Some(1),
            ..default()
        })
    }
}

/// What of a part's material the bake draws: the base colour, its texture if it has one, and the
/// coverage below which a pixel is bare.
#[derive(Clone, Debug)]
pub struct BakeMaterial {
    pub color: [f32; 4],
    pub texture: Option<AssetId<Image>>,
    pub cutoff: f32,
}

/// The bake pipeline and what it binds.
#[derive(Resource)]
pub struct BakePass {
    layout: BindGroupLayout,
    pipeline: RenderPipeline,
    blank: (TextureView, Sampler),
}

/// A model to bake into a layer: where it lies in the slabs, what it is drawn in, part by part,
/// and the layer.
pub struct Bake<'a> {
    pub model: &'a PlacedModel,
    pub materials: &'a [BakeMaterial],
    pub layer: u32,
}

/// What a part is drawn in, as the bake shader reads it.
#[derive(ShaderType, Clone, Copy)]
struct BakePart {
    color: Vec4,
    cutoff: f32,
    textured: u32,
}

impl BakePass {
    pub fn new(device: &RenderDevice, queue: &RenderQueue) -> BakePass {
        let layout = device.create_bind_group_layout(
            "impostor bake",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::VERTEX_FRAGMENT,
                (
                    storage_buffer_read_only_sized(false, None),
                    uniform_buffer::<BakePart>(false),
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                ),
            ),
        );
        let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("impostor bake"),
            source: ShaderSource::Wgsl(include_str!("impostor_bake.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("impostor bake"),
            bind_group_layouts: &[Some(&layout)],
            ..default()
        });
        let target = |format| {
            Some(ColorTargetState {
                format,
                blend: None,
                write_mask: ColorWrites::ALL,
            })
        };
        let pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("impostor bake"),
            layout: Some(&pipeline_layout),
            vertex: RawVertexState {
                module: &module,
                entry_point: Some("vertex"),
                compilation_options: PipelineCompilationOptions {
                    constants: &[("CELLS", f64::from(CELLS))],
                    ..default()
                },
                buffers: &[Some(RawVertexBufferLayout {
                    array_stride: ModelVertex::BYTES,
                    step_mode: VertexStepMode::Vertex,
                    attributes: &ModelVertex::ATTRIBUTES,
                })],
            },
            primitive: PrimitiveState {
                cull_mode: None,
                ..default()
            },
            depth_stencil: Some(DepthStencilState {
                format: BAKE_DEPTH,
                depth_write_enabled: Some(true),
                depth_compare: Some(CompareFunction::LessEqual),
                stencil: default(),
                bias: default(),
            }),
            multisample: MultisampleState::default(),
            fragment: Some(RawFragmentState {
                module: &module,
                entry_point: Some("fragment"),
                compilation_options: PipelineCompilationOptions::default(),
                targets: &[
                    target(ALBEDO_FORMAT),
                    target(NORMAL_FORMAT),
                    target(DEPTH_FORMAT),
                ],
            }),
            multiview_mask: None,
            cache: None,
        });
        BakePass {
            layout,
            pipeline,
            blank: blank_image(device, queue),
        }
    }

    /// Draws each of `bakes` into its layer of `atlas` over `encoder`: every part once, an
    /// instance a cell, each placed in its cell by the cell's view.
    pub fn encode(
        &self,
        device: &RenderDevice,
        encoder: &mut CommandEncoder,
        atlas: &ImpostorAtlas,
        models: &ModelArena,
        images: &RenderAssets<GpuImage>,
        bakes: &[Bake],
    ) {
        let (Some(vertices), Some(indices)) = (models.vertices.buffer(), models.indices.buffer())
        else {
            return;
        };
        let alignment = u64::from(device.limits().min_uniform_buffer_offset_alignment).max(1);
        let stride = BakePart::min_size().get().div_ceil(alignment) * alignment;
        let bake_depth = ImpostorAtlas::layer_view(&atlas.bake_depth, 0);
        for bake in bakes {
            let views: Vec<Mat4> = CellView::all()
                .map(|view| clip_from_model(bake.model, view.direction))
                .collect();
            let mut written = encase::StorageBuffer::new(Vec::<u8>::new());
            written
                .write(&views)
                .expect("the cells' views write as a storage buffer");
            let views = device.create_buffer_with_data(&BufferInitDescriptor {
                label: Some("impostor bake views"),
                contents: written.as_ref(),
                usage: BufferUsages::STORAGE,
            });
            let mut parts = Vec::new();
            let mut bound = Vec::new();
            for (part, material) in bake.model.parts.iter().zip(bake.materials) {
                if part.levels.is_empty() {
                    continue;
                }
                let mut written = encase::UniformBuffer::new(Vec::<u8>::new());
                written
                    .write(&BakePart {
                        color: Vec4::from_array(material.color),
                        cutoff: material.cutoff,
                        textured: u32::from(material.texture.is_some()),
                    })
                    .expect("a bake part writes as a uniform");
                let mut bytes = written.into_inner();
                bytes.resize(stride as usize, 0);
                let offset = parts.len() as u64 * stride;
                parts.push(bytes);
                let image = material
                    .texture
                    .and_then(|texture| images.get(texture))
                    .map_or((&self.blank.0, &self.blank.1), |image| {
                        (&image.texture_view, &image.sampler)
                    });
                bound.push((part, offset, image));
            }
            let parts: Vec<u8> = parts.concat();
            let parts = device.create_buffer_with_data(&BufferInitDescriptor {
                label: Some("impostor bake parts"),
                contents: &parts,
                usage: BufferUsages::UNIFORM,
            });
            let groups: Vec<_> = bound
                .iter()
                .map(|(_, offset, (view, sampler))| {
                    device.create_bind_group(
                        "impostor bake",
                        &self.layout,
                        &BindGroupEntries::sequential((
                            views.as_entire_binding(),
                            BufferBinding {
                                buffer: &parts,
                                offset: *offset,
                                size: Some(BakePart::min_size()),
                            },
                            *view,
                            *sampler,
                        )),
                    )
                })
                .collect();
            let targets = [
                ImpostorAtlas::layer_view(&atlas.albedo, bake.layer),
                ImpostorAtlas::layer_view(&atlas.normal, bake.layer),
                ImpostorAtlas::layer_view(&atlas.depth, bake.layer),
            ];
            let attachments = targets.iter().map(|view| {
                Some(RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: Operations {
                        load: LoadOp::Clear(default()),
                        store: StoreOp::Store,
                    },
                })
            });
            let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
                label: Some("impostor bake"),
                color_attachments: &attachments.collect::<Vec<_>>(),
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view: &bake_depth,
                    depth_ops: Some(Operations {
                        load: LoadOp::Clear(1.0),
                        store: StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_vertex_buffer(0, *vertices.slice(..));
            pass.set_index_buffer(*indices.slice(..), IndexFormat::Uint32);
            let allowed = cell_stray(bake.model);
            for ((part, _, _), group) in bound.iter().zip(&groups) {
                pass.set_bind_group(0, Some(&**group), &[]);
                let level = part
                    .levels
                    .iter()
                    .rev()
                    .find(|level| level.strays <= allowed)
                    .unwrap_or(&part.levels[0]);
                pass.draw_indexed(
                    level.first_index..level.first_index + level.indices,
                    part.first_vertex as i32,
                    0..CELLS * CELLS,
                );
            }
        }
    }
}

/// A layer's texels on their way back from the GPU, to be kept in a file.
pub struct Readback {
    pub model: AssetId<Gltf>,
    pub key: String,
    buffer: Buffer,
    mapped: Option<Arc<Mutex<Option<bool>>>>,
}

impl Readback {
    /// Copies `layer`'s texels into a buffer to read once the GPU has written them.
    pub fn of(
        device: &RenderDevice,
        encoder: &mut CommandEncoder,
        atlas: &ImpostorAtlas,
        layer: u32,
        model: AssetId<Gltf>,
        key: String,
    ) -> Readback {
        let buffer = device.create_buffer(&BufferDescriptor {
            label: Some("impostor readback"),
            size: ImpostorHeader::current().texel_bytes() as u64,
            usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut offset = 0;
        for (texture, texel_bytes) in [
            (&atlas.albedo, ALBEDO_BYTES),
            (&atlas.normal, NORMAL_BYTES),
            (&atlas.depth, DEPTH_BYTES),
        ] {
            encoder.copy_texture_to_buffer(
                layer_copy(texture, layer),
                TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: TexelCopyBufferLayout {
                        offset,
                        bytes_per_row: Some(ATLAS_PIXELS * texel_bytes as u32),
                        rows_per_image: Some(ATLAS_PIXELS),
                    },
                },
                layer_size(),
            );
            offset += (ATLAS_PIXELS * ATLAS_PIXELS) as u64 * texel_bytes as u64;
        }
        Readback {
            model,
            key,
            buffer,
            mapped: None,
        }
    }

    /// Asks for the texels the first time, then whether they have come back: the texels once
    /// they have, none while they are on their way, and an error if the GPU lost them.
    pub fn poll(&mut self) -> Option<Result<ImpostorTexels, ()>> {
        let Some(mapped) = &self.mapped else {
            let mapped = Arc::new(Mutex::new(None));
            let answer = mapped.clone();
            self.buffer
                .slice(..)
                .map_async(MapMode::Read, move |result| {
                    *answer.lock().expect("the readback's answer lock") = Some(result.is_ok());
                });
            self.mapped = Some(mapped);
            return None;
        };
        let ok = (*mapped.lock().expect("the readback's answer lock"))?;
        if !ok {
            return Some(Err(()));
        }
        let bytes = self.buffer.slice(..).get_mapped_range().ok()?.to_vec();
        let pixels = (ATLAS_PIXELS * ATLAS_PIXELS) as usize;
        let (albedo, rest) = bytes.split_at(pixels * ALBEDO_BYTES);
        let (normal, depth) = rest.split_at(pixels * NORMAL_BYTES);
        Some(Ok(ImpostorTexels {
            albedo: albedo.to_vec(),
            normal: normal.to_vec(),
            depth: depth.to_vec(),
        }))
    }
}

/// The orthographic view of `model` from `direction`, framed to its bounding sphere: the eye a
/// radius out from the sphere's middle, the near plane there and the far one a diameter in.
/// How far a level of `model` may stray from its full mesh and still land within half a pixel
/// of a cell, which frames the model's bounding sphere across `CELL_PIXELS`.
fn cell_stray(model: &PlacedModel) -> f32 {
    let radius = Vec3::from(model.bounds.half_extents).length().max(1e-3);
    STRAY_PIXELS * 2.0 * radius / CELL_PIXELS as f32
}

pub fn clip_from_model(model: &PlacedModel, direction: Vec3) -> Mat4 {
    let middle = Vec3::from(model.bounds.center);
    let radius = Vec3::from(model.bounds.half_extents).length().max(1e-3);
    let up = if direction.y.abs() > 0.999 {
        Vec3::NEG_Z
    } else {
        Vec3::Y
    };
    let view_from_model = Transform::from_translation(middle + direction * radius)
        .looking_at(middle, up)
        .to_matrix()
        .inverse();
    let clip_from_view = proj::orthographic(-radius, radius, -radius, radius, 0.0, 2.0 * radius);
    clip_from_view * view_from_model
}

fn layer_copy(texture: &Texture, layer: u32) -> TexelCopyTextureInfo<'_> {
    TexelCopyTextureInfo {
        texture,
        mip_level: 0,
        origin: Origin3d {
            x: 0,
            y: 0,
            z: layer,
        },
        aspect: TextureAspect::All,
    }
}

fn layer_size() -> Extent3d {
    Extent3d {
        width: ATLAS_PIXELS,
        height: ATLAS_PIXELS,
        depth_or_array_layers: 1,
    }
}

/// A white pixel for a part with no texture, so every part binds alike.
fn blank_image(device: &RenderDevice, queue: &RenderQueue) -> (TextureView, Sampler) {
    let texture = device.create_texture(&TextureDescriptor {
        label: Some("impostor blank"),
        size: Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format: TextureFormat::Rgba8Unorm,
        usage: TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: Origin3d::ZERO,
            aspect: TextureAspect::All,
        },
        &[255, 255, 255, 255],
        TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4),
            rows_per_image: Some(1),
        },
        Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    (
        texture.create_view(&default()),
        device.create_sampler(&default()),
    )
}
