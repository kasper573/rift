//! What a view shows of the sceneries' arenas, instance by instance, for whoever asks: while a
//! camera carries [`Picking`], every instance its views draw writes its id where it is the surface
//! the view shows, after the world is drawn. The id under the pointer is read back without waiting
//! on the GPU and handed to the main world a frame or two later ([`picked`]), and the instances of
//! each outlined set are outlined in the set's colour over what the view shows.

use std::sync::{Arc, Mutex};

use bevy::core_pipeline::core_3d::main_transparent_pass_3d;
use bevy::core_pipeline::{Core3d, Core3dSystems, FullscreenShader};
use bevy::ecs::query::QueryItem;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::render_resource::binding_types::{
    storage_buffer_read_only, storage_buffer_read_only_sized, texture_2d, texture_depth_2d,
    uniform_buffer,
};
use bevy::render::render_resource::{
    BindGroupEntries, BindGroupLayout, BindGroupLayoutDescriptor, BindGroupLayoutEntries,
    BlendState, Buffer, BufferDescriptor, BufferInitDescriptor, BufferUsages,
    CachedRenderPipelineId, ColorTargetState, ColorWrites, CommandEncoder, CompareFunction,
    DepthStencilState, Extent3d, FragmentState, IndexFormat, LoadOp, MapMode, MultisampleState,
    Operations, Origin3d, PipelineCache, PipelineCompilationOptions, PipelineLayoutDescriptor,
    PrimitiveState, RawFragmentState, RawRenderPipelineDescriptor, RawVertexBufferLayout,
    RawVertexState, RenderPassColorAttachment, RenderPassDepthStencilAttachment,
    RenderPassDescriptor, RenderPipeline, RenderPipelineDescriptor, ShaderModuleDescriptor,
    ShaderSource, ShaderStages, ShaderType, StorageBuffer, StoreOp, TexelCopyBufferInfo,
    TexelCopyBufferLayout, TexelCopyTextureInfo, TextureAspect, TextureDescriptor,
    TextureDimension, TextureFormat, TextureSampleType, TextureUsages, TextureView, UniformBuffer,
    VertexAttribute, VertexFormat, VertexStepMode, encase,
};
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::sync_component::SyncComponent;
use bevy::render::texture::{CachedTexture, TextureCache};
use bevy::render::view::{ExtractedView, RetainedViewEntity, ViewDepthStencilTexture, ViewTarget};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};

use super::arena::{InstanceArenas, ModelArena, ModelVertex};
use super::cull::{PropCulls, ViewCull};

pub struct PickPlugin;

impl Plugin for PickPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "outline.wesl");
        let picks = Picks::default();
        app.insert_resource(picks.clone())
            .add_plugins(ExtractComponentPlugin::<Picking>::default())
            .add_systems(PostUpdate, let_depth_be_read);
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .insert_resource(picks)
            .init_resource::<PendingPicks>()
            .add_systems(RenderStartup, start)
            .add_systems(
                Render,
                (
                    prepare_targets.in_set(RenderSystems::PrepareResources),
                    queue_outlines.in_set(RenderSystems::Queue),
                    read_picks.in_set(RenderSystems::Cleanup),
                ),
            )
            .add_systems(
                Core3d,
                (draw_picks, draw_outlines)
                    .chain()
                    .after(main_transparent_pass_3d)
                    .after(super::super::haze::Hazing)
                    .in_set(Core3dSystems::MainPass),
            );
    }
}

/// A camera whose views something wants singled out: the pixel of its picture the pointer is over,
/// whose instance [`picked`] tells, and the instances outlined, in two sets, the second over the
/// first. Its views write their ids only while it points or outlines something.
#[derive(Component, Clone, Debug, Default)]
pub struct Picking {
    pub pointer: Option<UVec2>,
    pub outlines: [Outline; 2],
}

/// Instances outlined in `color`, each a scenery's root and its place among its arena's instances.
#[derive(Clone, Debug, Default)]
pub struct Outline {
    pub color: LinearRgba,
    pub instances: Arc<[(Entity, u32)]>,
}

/// What showed at a pixel of a camera's picture: the instance of a scenery's arena, if any.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pick {
    pub at: UVec2,
    pub hit: Option<PickHit>,
}

/// An instance a view showed: its scenery's root and its place among the arena's instances.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PickHit {
    pub scenery: Entity,
    pub instance: u32,
}

/// What the latest pixel read back of `camera`'s picture showed.
pub fn picked(world: &World, camera: Entity) -> Option<Pick> {
    world
        .get_resource::<Picks>()?
        .0
        .lock()
        .expect("the picks lock")
        .get(&camera)
        .copied()
}

/// How the GPU writes each view's ids: a pass drawing the instances a view's commands draw into an
/// id picture and a depth of its own, keeping only the surface the view's depth shows.
pub struct PickPass {
    layout: BindGroupLayout,
    pipeline: RenderPipeline,
}

/// What a pick pass writes and reads: the ids and its own depth, which it clears, and the depth the
/// view was drawn with.
pub struct PickTargets<'a> {
    pub ids: &'a TextureView,
    pub depth: &'a TextureView,
    pub view_depth: &'a TextureView,
}

/// A scenery a pick pass draws: its slot among those the view draws, the view's projection, its
/// arena's instances, the models' vertices and indices, and the view's culled draws of it: the
/// instances drawn and `commands` commands in `commands_buffer`.
pub struct PickDraw<'a> {
    pub slot: u8,
    pub clip_from_world: Mat4,
    pub instances: &'a Buffer,
    pub vertices: &'a Buffer,
    pub indices: &'a Buffer,
    pub drawn: &'a Buffer,
    pub commands_buffer: &'a Buffer,
    pub commands: u32,
}

/// The format of the ids a pick pass writes.
pub const PICK_IDS: TextureFormat = TextureFormat::R32Uint;

/// The format of a pick pass's own depth.
pub const PICK_DEPTH: TextureFormat = TextureFormat::Depth32Float;

impl PickPass {
    pub fn new(device: &RenderDevice) -> PickPass {
        let layout = device.create_bind_group_layout(
            "pick",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::VERTEX_FRAGMENT,
                (
                    uniform_buffer::<PickViewUniform>(false),
                    storage_buffer_read_only_sized(false, None),
                    storage_buffer_read_only_sized(false, None),
                    texture_depth_2d(),
                ),
            ),
        );
        let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("pick"),
            source: ShaderSource::Wgsl(include_str!("pick.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("pick"),
            bind_group_layouts: &[Some(&layout)],
            ..default()
        });
        let pipeline = device.create_render_pipeline(&RawRenderPipelineDescriptor {
            label: Some("pick"),
            layout: Some(&pipeline_layout),
            vertex: RawVertexState {
                module: &module,
                entry_point: Some("vertex"),
                compilation_options: PipelineCompilationOptions::default(),
                buffers: &[Some(RawVertexBufferLayout {
                    array_stride: ModelVertex::BYTES,
                    step_mode: VertexStepMode::Vertex,
                    attributes: &[VertexAttribute {
                        format: VertexFormat::Float32x3,
                        offset: 0,
                        shader_location: 0,
                    }],
                })],
            },
            primitive: PrimitiveState::default(),
            depth_stencil: Some(DepthStencilState {
                format: PICK_DEPTH,
                depth_write_enabled: Some(true),
                depth_compare: Some(CompareFunction::GreaterEqual),
                stencil: default(),
                bias: default(),
            }),
            multisample: MultisampleState::default(),
            fragment: Some(RawFragmentState {
                module: &module,
                entry_point: Some("fragment"),
                compilation_options: PipelineCompilationOptions::default(),
                targets: &[Some(ColorTargetState {
                    format: PICK_IDS,
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        PickPass { layout, pipeline }
    }

    /// Writes the ids of what `draws` draw into `targets`, nearest first, nothing where the view's
    /// depth shows something else.
    pub fn encode(
        &self,
        device: &RenderDevice,
        encoder: &mut CommandEncoder,
        targets: &PickTargets,
        draws: &[PickDraw],
    ) {
        let bound: Vec<_> = draws
            .iter()
            .map(|draw| {
                let mut view = encase::UniformBuffer::new(Vec::<u8>::new());
                view.write(&PickViewUniform {
                    clip_from_world: draw.clip_from_world,
                    slot: u32::from(draw.slot),
                })
                .expect("a pick view writes as a uniform");
                let uniform = device.create_buffer_with_data(&BufferInitDescriptor {
                    label: Some("pick view"),
                    contents: view.as_ref(),
                    usage: BufferUsages::UNIFORM,
                });
                device.create_bind_group(
                    "pick",
                    &self.layout,
                    &BindGroupEntries::sequential((
                        uniform.as_entire_binding(),
                        draw.instances.as_entire_binding(),
                        draw.drawn.as_entire_binding(),
                        targets.view_depth,
                    )),
                )
            })
            .collect();
        let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
            label: Some("pick"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: targets.ids,
                depth_slice: None,
                resolve_target: None,
                ops: Operations {
                    load: LoadOp::Clear(default()),
                    store: StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                view: targets.depth,
                depth_ops: Some(Operations {
                    load: LoadOp::Clear(0.0),
                    store: StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        for (draw, bound) in draws.iter().zip(&bound) {
            pass.set_bind_group(0, Some(&**bound), &[]);
            pass.set_vertex_buffer(0, Some((**draw.vertices).slice(..)));
            pass.set_index_buffer((**draw.indices).slice(..), IndexFormat::Uint32);
            pass.multi_draw_indexed_indirect(
                draw.commands_buffer,
                ViewCull::command_offset(0),
                draw.commands,
            );
        }
    }
}

/// The scenery slot and the place among its arena's instances an id written by a pick pass names,
/// none for nothing.
pub fn decode(id: u32) -> Option<(u8, u32)> {
    let instance = id & INSTANCE_BITS;
    (instance > 0).then(|| ((id >> 24) as u8, instance - 1))
}

/// The id a pick pass writes for the instance at `instance` of the scenery in `slot`.
pub fn encode(slot: u8, instance: u32) -> u32 {
    u32::from(slot) << 24 | (instance + 1) & INSTANCE_BITS
}

const INSTANCE_BITS: u32 = (1 << 24) - 1;

/// How many pixels round an outlined instance its outline reaches.
const OUTLINE_PIXELS: u32 = 2;

/// The bytes a readback of one id takes, as a copy of a texture's row must.
const READBACK_BYTES: u64 = 256;

#[derive(ShaderType, Clone, Copy)]
struct PickViewUniform {
    clip_from_world: Mat4,
    slot: u32,
}

#[derive(ShaderType, Clone, Copy, Default)]
struct OutlineUniform {
    colors: [Vec4; 2],
    counts: UVec2,
    width: u32,
}

/// A camera's [`Picking`] as the render world reads it, with the camera it belongs to.
#[derive(Component, Clone, Debug)]
pub struct ExtractedPicking {
    camera: Entity,
    pointer: Option<UVec2>,
    outlines: [Outline; 2],
}

impl SyncComponent<RenderApp> for Picking {
    type Target = ExtractedPicking;
}

impl ExtractComponent<RenderApp> for Picking {
    type QueryData = (Entity, &'static Picking);
    type QueryFilter = ();
    type Out = ExtractedPicking;

    fn extract_component(
        (camera, picking): QueryItem<'_, '_, Self::QueryData>,
    ) -> Option<Self::Out> {
        let wanted = picking.pointer.is_some()
            || picking
                .outlines
                .iter()
                .any(|outline| !outline.instances.is_empty());
        wanted.then(|| ExtractedPicking {
            camera,
            pointer: picking.pointer,
            outlines: picking.outlines.clone(),
        })
    }
}

/// What the main world has been told each camera's pointer is over, by the camera.
#[derive(Resource, Clone, Default)]
struct Picks(Arc<Mutex<HashMap<Entity, Pick>>>);

/// The pick pass and how outlines are drawn.
#[derive(Resource)]
struct PickPipelines {
    pass: PickPass,
    outline_layout: BindGroupLayoutDescriptor,
    outline_shader: Handle<Shader>,
}

/// A view's ids and the pick pass's own depth for them.
#[derive(Component)]
struct PickTextures {
    ids: CachedTexture,
    depth: CachedTexture,
    size: UVec2,
}

#[derive(Component, Clone, Copy)]
struct OutlinePipelineId(CachedRenderPipelineId);

/// The ids under pointers on their way back from the GPU.
#[derive(Resource, Default)]
struct PendingPicks(Vec<PendingPick>);

/// An id under a camera's pointer on its way back from the GPU, with the sceneries by the slots
/// the pass drew them in.
struct PendingPick {
    camera: Entity,
    at: UVec2,
    sceneries: Vec<Entity>,
    buffer: Buffer,
    mapped: Option<Arc<Mutex<Option<bool>>>>,
}

/// The most ids on their way back at once: past them, the oldest are let go.
const MOST_PENDING: usize = 8;

/// A picking camera as it is made or changed, or as it takes up picking.
type PickingCameraChanged = (With<Picking>, Or<(Changed<Camera3d>, Added<Picking>)>);

/// Every picking camera's depth can be read once the world is drawn, as the pick pass reads it.
fn let_depth_be_read(mut cameras: Query<&mut Camera3d, PickingCameraChanged>) {
    for mut camera in &mut cameras {
        let usages = TextureUsages::from(camera.depth_texture_usages);
        if !usages.contains(TextureUsages::TEXTURE_BINDING) {
            camera.depth_texture_usages = (usages | TextureUsages::TEXTURE_BINDING).into();
        }
    }
}

fn start(mut commands: Commands, device: Res<RenderDevice>, assets: Res<AssetServer>) {
    commands.insert_resource(PickPipelines {
        pass: PickPass::new(&device),
        outline_layout: BindGroupLayoutDescriptor::new(
            "outline",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    texture_2d(TextureSampleType::Uint),
                    storage_buffer_read_only::<Vec<u32>>(false),
                    uniform_buffer::<OutlineUniform>(false),
                ),
            ),
        ),
        outline_shader: bevy::asset::load_embedded_asset!(assets.as_ref(), "outline.wesl"),
    });
}

/// Gives every picking view its ids and the pick pass's depth, the size of its picture.
fn prepare_targets(
    mut commands: Commands,
    device: Res<RenderDevice>,
    mut cache: ResMut<TextureCache>,
    views: Query<(Entity, &ExtractedCamera), With<ExtractedPicking>>,
) {
    for (entity, camera) in &views {
        let Some(size) = camera.physical_target_size else {
            continue;
        };
        let mut texture = |label: &'static str, format: TextureFormat, usage: TextureUsages| {
            cache.get(
                &device,
                TextureDescriptor {
                    label: Some(label),
                    size: Extent3d {
                        width: size.x,
                        height: size.y,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                },
            )
        };
        let ids = texture(
            "pick ids",
            PICK_IDS,
            TextureUsages::RENDER_ATTACHMENT
                | TextureUsages::TEXTURE_BINDING
                | TextureUsages::COPY_SRC,
        );
        let depth = texture("pick depth", PICK_DEPTH, TextureUsages::RENDER_ATTACHMENT);
        commands
            .entity(entity)
            .insert(PickTextures { ids, depth, size });
    }
}

fn queue_outlines(
    mut commands: Commands,
    cache: Res<PipelineCache>,
    pipelines: Option<Res<PickPipelines>>,
    fullscreen: Res<FullscreenShader>,
    views: Query<(Entity, &ExtractedView, &ExtractedPicking)>,
    mut queued: Local<HashMap<TextureFormat, CachedRenderPipelineId>>,
) {
    let Some(pipelines) = pipelines else {
        return;
    };
    for (entity, view, picking) in &views {
        if picking
            .outlines
            .iter()
            .all(|outline| outline.instances.is_empty())
        {
            continue;
        }
        let id = *queued.entry(view.target_format).or_insert_with(|| {
            cache.queue_render_pipeline(RenderPipelineDescriptor {
                label: Some("outline".into()),
                layout: vec![pipelines.outline_layout.clone()],
                vertex: fullscreen.to_vertex_state(),
                fragment: Some(FragmentState {
                    shader: pipelines.outline_shader.clone(),
                    targets: vec![Some(ColorTargetState {
                        format: view.target_format,
                        blend: Some(BlendState::ALPHA_BLENDING),
                        write_mask: ColorWrites::ALL,
                    })],
                    ..default()
                }),
                ..default()
            })
        });
        commands.entity(entity).insert(OutlinePipelineId(id));
    }
}

/// The sceneries `view` draws, each by its slot.
fn slots(culls: &PropCulls, view: RetainedViewEntity) -> Vec<Entity> {
    let mut drawn: Vec<Entity> = culls
        .0
        .iter()
        .filter(|(_, cull)| cull.table.is_some() && cull.views.contains_key(&view))
        .map(|(scenery, _)| *scenery)
        .collect();
    drawn.sort();
    drawn.truncate(usize::from(u8::MAX) + 1);
    drawn
}

/// Writes the ids of what a picking view shows, and sends the one under its pointer back.
fn draw_picks(
    view: ViewQuery<(
        &ExtractedView,
        &ExtractedPicking,
        &PickTextures,
        &ViewDepthStencilTexture,
    )>,
    pipelines: Option<Res<PickPipelines>>,
    culls: Res<PropCulls>,
    arenas: Res<InstanceArenas>,
    models: Res<ModelArena>,
    mut pending: ResMut<PendingPicks>,
    mut ctx: RenderContext,
) {
    let (seen, picking, textures, view_depth) = view.into_inner();
    let (Some(pipelines), Some(vertices), Some(indices)) =
        (pipelines, models.vertices.buffer(), models.indices.buffer())
    else {
        return;
    };
    let Some(view_depth) = view_depth
        .attachment
        .depth_stencil_views()
        .depth_only_view()
    else {
        return;
    };
    let clip_from_world = seen
        .clip_from_world
        .unwrap_or_else(|| seen.clip_from_view * seen.world_from_view.to_matrix().inverse());
    let sceneries = slots(&culls, seen.retained_view_entity);
    let draws: Vec<PickDraw> = sceneries
        .iter()
        .enumerate()
        .filter_map(|(slot, scenery)| {
            let cull = culls.0.get(scenery)?;
            let culled = cull.views.get(&seen.retained_view_entity)?;
            Some(PickDraw {
                slot: slot as u8,
                clip_from_world,
                instances: arenas.0.get(scenery)?.slab.buffer()?,
                vertices,
                indices,
                drawn: culled.drawn(),
                commands_buffer: culled.commands(),
                commands: cull.table.as_ref()?.table().impostors.start,
            })
        })
        .collect();
    let device = ctx.render_device().clone();
    let encoder = ctx.command_encoder();
    pipelines.pass.encode(
        &device,
        encoder,
        &PickTargets {
            ids: &textures.ids.default_view,
            depth: &textures.depth.default_view,
            view_depth,
        },
        &draws,
    );
    let Some(at) = picking
        .pointer
        .filter(|at| at.x < textures.size.x && at.y < textures.size.y)
    else {
        return;
    };
    let buffer = device.create_buffer(&BufferDescriptor {
        label: Some("pick readback"),
        size: READBACK_BYTES,
        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        TexelCopyTextureInfo {
            texture: &textures.ids.texture,
            mip_level: 0,
            origin: Origin3d {
                x: at.x,
                y: at.y,
                z: 0,
            },
            aspect: TextureAspect::All,
        },
        TexelCopyBufferInfo {
            buffer: &buffer,
            layout: TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: None,
                rows_per_image: None,
            },
        },
        Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    pending.0.push(PendingPick {
        camera: picking.camera,
        at,
        sceneries,
        buffer,
        mapped: None,
    });
    let over = pending.0.len().saturating_sub(MOST_PENDING);
    pending.0.drain(..over);
}

/// Outlines each set of a picking view's instances over what it shows.
fn draw_outlines(
    view: ViewQuery<(
        &ExtractedView,
        &ExtractedCamera,
        &ViewTarget,
        &ExtractedPicking,
        &PickTextures,
        &OutlinePipelineId,
    )>,
    pipelines: Option<Res<PickPipelines>>,
    cache: Res<PipelineCache>,
    culls: Res<PropCulls>,
    queue: Res<RenderQueue>,
    mut ctx: RenderContext,
) {
    let (seen, camera, target, picking, textures, pipeline_id) = view.into_inner();
    let (Some(pipelines), Some(pipeline)) = (pipelines, cache.get_render_pipeline(pipeline_id.0))
    else {
        return;
    };
    let sceneries = slots(&culls, seen.retained_view_entity);
    let slot_of: HashMap<Entity, u8> = sceneries
        .iter()
        .enumerate()
        .map(|(slot, scenery)| (*scenery, slot as u8))
        .collect();
    let sets = picking.outlines.clone().map(|outline| {
        let mut ids: Vec<u32> = outline
            .instances
            .iter()
            .filter_map(|(scenery, instance)| Some(encode(*slot_of.get(scenery)?, *instance)))
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    });
    if sets.iter().all(Vec::is_empty) {
        return;
    }
    let device = ctx.render_device().clone();
    let mut uniform = UniformBuffer::from(OutlineUniform {
        colors: picking
            .outlines
            .clone()
            .map(|outline| outline.color.to_vec4()),
        counts: UVec2::new(sets[0].len() as u32, sets[1].len() as u32),
        width: OUTLINE_PIXELS,
    });
    uniform.write_buffer(&device, &queue);
    let mut outlined = StorageBuffer::from(
        sets.iter()
            .flatten()
            .copied()
            .chain([0])
            .collect::<Vec<u32>>(),
    );
    outlined.write_buffer(&device, &queue);
    let (Some(outlined), Some(uniform)) = (outlined.binding(), uniform.binding()) else {
        return;
    };
    let bind_group = device.create_bind_group(
        "outline",
        &cache.get_bind_group_layout(&pipelines.outline_layout),
        &BindGroupEntries::sequential((&textures.ids.default_view, outlined, uniform)),
    );
    let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some("outline"),
        color_attachments: &[Some(target.get_color_attachment())],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    if let Some(viewport) = camera.viewport.as_ref() {
        pass.set_camera_viewport(viewport);
    }
    pass.set_render_pipeline(pipeline);
    pass.set_bind_group(0, &bind_group, &[]);
    pass.draw(0..3, 0..1);
}

/// Asks the GPU for each id under a pointer once the frame that wrote it is sent, and tells the
/// main world what each showed once it has come back.
fn read_picks(mut pending: ResMut<PendingPicks>, picks: Res<Picks>) {
    pending.0.retain_mut(|pick| {
        let Some(mapped) = &pick.mapped else {
            let mapped = Arc::new(Mutex::new(None));
            let answer = mapped.clone();
            pick.buffer
                .slice(..)
                .map_async(MapMode::Read, move |result| {
                    *answer.lock().expect("the pick's answer lock") = Some(result.is_ok());
                });
            pick.mapped = Some(mapped);
            return true;
        };
        let Some(ok) = *mapped.lock().expect("the pick's answer lock") else {
            return true;
        };
        let hit = ok
            .then(|| {
                let bytes = pick.buffer.slice(..4).get_mapped_range().ok()?.to_vec();
                Some(u32::from_le_bytes(bytes.try_into().ok()?))
            })
            .flatten()
            .and_then(decode)
            .and_then(|(slot, instance)| {
                Some(PickHit {
                    scenery: *pick.sceneries.get(usize::from(slot))?,
                    instance,
                })
            });
        picks
            .0
            .lock()
            .expect("the picks lock")
            .insert(pick.camera, Pick { at: pick.at, hit });
        false
    });
}
