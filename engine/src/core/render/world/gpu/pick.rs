//! What a view shows of the sceneries' arenas, instance by instance, for whoever asks: while a
//! camera carries [`Picking`], the instance its view shows under the pointer writes its id there,
//! after the world is drawn, through the stages that drew it there. The id is read back without
//! waiting on the GPU and handed to the main world a frame or two later ([`picked`]). And the
//! instances of each outlined set are outlined in the set's colour: a line round the whole shape
//! of them, as their shaders stand it, laid over the finished picture whatever stands before them.

use std::sync::{Arc, Mutex};

use bevy::anti_alias::smaa::smaa;
use bevy::core_pipeline::core_3d::main_transparent_pass_3d;
use bevy::core_pipeline::tonemapping::tonemapping;
use bevy::core_pipeline::{Core3d, Core3dSystems, FullscreenShader};
use bevy::ecs::query::QueryItem;
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::diagnostic::RecordDiagnostics;
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::render_resource::binding_types::{sampler, texture_2d, uniform_buffer};
use bevy::render::render_resource::{
    BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, BlendState, Buffer,
    BufferDescriptor, BufferUsages, CachedRenderPipelineId, ColorTargetState, ColorWrites,
    Extent3d, FilterMode, FragmentState, LoadOp, MapMode, Operations, Origin3d, PipelineCache,
    RenderPassColorAttachment, RenderPassDepthStencilAttachment, RenderPassDescriptor,
    RenderPipelineDescriptor, Sampler, SamplerBindingType, SamplerDescriptor, ShaderStages,
    ShaderType, StoreOp, TexelCopyBufferInfo, TexelCopyBufferLayout, TexelCopyTextureInfo,
    TextureAspect, TextureDescriptor, TextureDimension, TextureFormat, TextureSampleType,
    TextureUsages, UniformBuffer,
};
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::sync_component::SyncComponent;
use bevy::render::texture::{CachedTexture, TextureCache};
use bevy::render::view::{
    ExtractedView, RetainedViewEntity, ViewDepthStencilTexture, ViewTarget, ViewUniformOffset,
};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};

use super::cull::PropCulls;
use super::prop_draw::{
    IDS_FORMAT, MARK_SAMPLES, MARKS_FORMAT, PropQueue, QueueProps, SingledProps,
};

pub struct PickPlugin;

impl Plugin for PickPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "outline.wesl");
        let picks = Picks::default();
        app.insert_resource(picks.clone())
            .add_plugins(ExtractComponentPlugin::<Picking>::default());
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
                    (queue_singled.in_set(QueueProps), queue_outlines).in_set(RenderSystems::Queue),
                    read_picks.in_set(RenderSystems::Cleanup),
                ),
            )
            .add_systems(
                Core3d,
                (
                    draw_picks
                        .after(main_transparent_pass_3d)
                        .after(super::super::haze::Hazing)
                        .in_set(Core3dSystems::MainPass),
                    draw_outlines
                        .after(tonemapping)
                        .before(smaa)
                        .in_set(Core3dSystems::PostProcess),
                ),
            );
    }
}

/// A camera whose views something wants singled out: the pixel of its picture the pointer is over,
/// whose instance [`picked`] tells, and the instances outlined, in two sets, the second over the
/// first.
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

/// The scenery slot and the place among its arena's instances an id a view wrote names, none for
/// nothing.
fn decode(id: u32) -> Option<(u8, u32)> {
    let instance = id & INSTANCE_BITS;
    (instance > 0).then(|| ((id >> 24) as u8, instance - 1))
}

const INSTANCE_BITS: u32 = (1 << 24) - 1;

const PICK: &str = "pick";
const OUTLINE: &str = "outline";

/// How many pixels out from an outlined instance's shape its outline reaches.
const OUTLINE_PIXELS: f32 = 2.0;

/// The bytes a readback of one id takes, as a copy of a texture's row must.
const READBACK_BYTES: u64 = 256;

#[derive(ShaderType, Clone, Copy, Default)]
struct OutlineUniform {
    colors: [Vec4; 2],
    width: f32,
}

/// A camera's [`Picking`] as the render world reads it, with the camera it belongs to.
#[derive(Component, Clone, Debug)]
pub struct ExtractedPicking {
    camera: Entity,
    pointer: Option<UVec2>,
    outlines: [Outline; 2],
}

impl ExtractedPicking {
    /// Whether it outlines anything.
    fn outlining(&self) -> bool {
        self.outlines
            .iter()
            .any(|outline| !outline.instances.is_empty())
    }
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

/// How outlines are drawn.
#[derive(Resource)]
struct PickPipelines {
    outline_layout: BindGroupLayoutDescriptor,
    outline_shader: Handle<Shader>,
    /// Reads the marks between their pixels, to tell at a few reads where none lies near.
    between: Sampler,
}

/// A view's ids, and the marks of the instances it outlines: as they are written, on several
/// samples a pixel, and with each pixel's samples settled into the share of it marked.
#[derive(Component)]
struct PickTextures {
    ids: CachedTexture,
    sampled_marks: CachedTexture,
    marks: CachedTexture,
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

fn start(mut commands: Commands, device: Res<RenderDevice>, assets: Res<AssetServer>) {
    commands.insert_resource(PickPipelines {
        outline_layout: BindGroupLayoutDescriptor::new(
            "outline",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                    uniform_buffer::<OutlineUniform>(false),
                ),
            ),
        ),
        outline_shader: bevy::asset::load_embedded_asset!(assets.as_ref(), "outline.wesl"),
        between: device.create_sampler(&SamplerDescriptor {
            label: Some("outline marks"),
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            ..default()
        }),
    });
}

/// Gives every picking view its ids and its marks, the size of its picture.
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
        let mut texture = |label: &'static str, format: TextureFormat, samples: u32, usage| {
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
                    sample_count: samples,
                    dimension: TextureDimension::D2,
                    format,
                    usage: TextureUsages::RENDER_ATTACHMENT | usage,
                    view_formats: &[],
                },
            )
        };
        let ids = texture("pick ids", IDS_FORMAT, 1, TextureUsages::COPY_SRC);
        let sampled_marks = texture(
            "outline marks sampled",
            MARKS_FORMAT,
            MARK_SAMPLES,
            TextureUsages::empty(),
        );
        let marks = texture(
            "outline marks",
            MARKS_FORMAT,
            1,
            TextureUsages::TEXTURE_BINDING,
        );
        commands.entity(entity).insert(PickTextures {
            ids,
            sampled_marks,
            marks,
            size,
        });
    }
}

/// Has every picking view's props ready what they write its ids and its marks with.
fn queue_singled(mut queue: PropQueue, views: Query<(&ExtractedView, &ExtractedPicking)>) {
    for (view, picking) in &views {
        if picking.pointer.is_some() {
            queue.ids(view.retained_view_entity);
        }
        if picking.outlining() {
            queue.marks(view.retained_view_entity);
        }
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
        if !picking.outlining() {
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

/// Writes the id of what a picking view shows under its pointer, over the depth it was drawn
/// with, and sends it back.
fn draw_picks(
    view: ViewQuery<(
        &ExtractedView,
        &ExtractedCamera,
        &ExtractedPicking,
        &PickTextures,
        &ViewDepthStencilTexture,
        &ViewUniformOffset,
        &Msaa,
    )>,
    props: SingledProps,
    culls: Res<PropCulls>,
    mut pending: ResMut<PendingPicks>,
    mut ctx: RenderContext,
) {
    let (seen, camera, picking, textures, view_depth, offset, msaa) = view.into_inner();
    let Some(at) = picking
        .pointer
        .filter(|at| at.x < textures.size.x && at.y < textures.size.y)
    else {
        return;
    };
    // The ids take one sample a pixel, so they are drawn only over a depth that takes one too.
    if *msaa != Msaa::Off {
        return;
    }
    let sceneries = slots(&culls, seen.retained_view_entity);
    let device = ctx.render_device().clone();
    let recorder = ctx.diagnostic_recorder();
    let recorder = recorder.as_deref();
    let encoder = ctx.command_encoder();
    let span = recorder.map(|recorder| recorder.time_span(encoder, PICK));
    let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
        label: Some(PICK),
        color_attachments: &[Some(RenderPassColorAttachment {
            view: &textures.ids.default_view,
            depth_slice: None,
            resolve_target: None,
            ops: Operations {
                load: LoadOp::Clear(default()),
                store: StoreOp::Store,
            },
        })],
        depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
            view: view_depth
                .attachment
                .depth_stencil_views()
                .attachment_view(),
            depth_ops: None,
            stencil_ops: None,
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    stand_in(&mut pass, camera);
    pass.set_scissor_rect(at.x, at.y, 1, 1);
    props.draw_ids(
        &device,
        &mut pass,
        seen.retained_view_entity,
        offset,
        &sceneries,
    );
    drop(pass);
    if let Some(span) = span {
        span.end(encoder);
    }
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

/// Marks the whole shape of every instance a picking view outlines, and lays each set's outline
/// round its shapes over the view's finished picture.
fn draw_outlines(
    view: ViewQuery<(
        &ExtractedView,
        &ExtractedCamera,
        &ViewTarget,
        &ExtractedPicking,
        &PickTextures,
        &OutlinePipelineId,
        &ViewUniformOffset,
    )>,
    pipelines: Option<Res<PickPipelines>>,
    cache: Res<PipelineCache>,
    props: SingledProps,
    queue: Res<RenderQueue>,
    mut ctx: RenderContext,
) {
    let (seen, camera, target, picking, textures, pipeline_id, offset) = view.into_inner();
    let (Some(pipelines), Some(pipeline)) = (pipelines, cache.get_render_pipeline(pipeline_id.0))
    else {
        return;
    };
    let marked = marks(&picking.outlines);
    if marked.is_empty() {
        return;
    }
    let device = ctx.render_device().clone();
    let mut uniform = UniformBuffer::from(OutlineUniform {
        colors: picking
            .outlines
            .clone()
            .map(|outline| outline.color.to_vec4()),
        width: OUTLINE_PIXELS,
    });
    uniform.write_buffer(&device, &queue);
    let Some(uniform) = uniform.binding() else {
        return;
    };
    let bind_group = device.create_bind_group(
        "outline",
        &cache.get_bind_group_layout(&pipelines.outline_layout),
        &BindGroupEntries::sequential((&textures.marks.default_view, &pipelines.between, uniform)),
    );
    let recorder = ctx.diagnostic_recorder();
    let recorder = recorder.as_deref();
    let encoder = ctx.command_encoder();
    let span = recorder.map(|recorder| recorder.time_span(encoder, OUTLINE));
    let mut pass = encoder.begin_render_pass(&RenderPassDescriptor {
        label: Some("outline marks"),
        color_attachments: &[Some(RenderPassColorAttachment {
            view: &textures.sampled_marks.default_view,
            depth_slice: None,
            resolve_target: Some(&textures.marks.default_view),
            ops: Operations {
                load: LoadOp::Clear(default()),
                store: StoreOp::Discard,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    stand_in(&mut pass, camera);
    props.draw_marks(
        &device,
        &mut pass,
        seen.retained_view_entity,
        offset,
        &marked,
    );
    drop(pass);
    let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some(OUTLINE),
        color_attachments: &[Some(target.get_unsampled_color_attachment())],
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
    drop(pass);
    if let Some(span) = span {
        span.end(ctx.command_encoder());
    }
}

/// The marks of the instances `outlines` hold of each scenery: two bits an instance, the first
/// set's lower, in as many words as reach the last instance held.
fn marks(outlines: &[Outline; 2]) -> Vec<(Entity, Vec<u32>)> {
    let mut marked: HashMap<Entity, Vec<u32>> = HashMap::default();
    for (set, outline) in outlines.iter().enumerate() {
        for &(scenery, instance) in outline.instances.iter() {
            let marks = marked.entry(scenery).or_default();
            let word = (instance / 16) as usize;
            if marks.len() <= word {
                marks.resize(word + 1, 0);
            }
            marks[word] |= 1 << (instance % 16 * 2 + set as u32);
        }
    }
    marked.into_iter().collect()
}

/// Has `pass` draw where `camera` draws on its target.
fn stand_in(pass: &mut wgpu::RenderPass, camera: &ExtractedCamera) {
    if let Some(viewport) = camera.viewport.as_ref() {
        pass.set_viewport(
            viewport.physical_position.x as f32,
            viewport.physical_position.y as f32,
            viewport.physical_size.x as f32,
            viewport.physical_size.y as f32,
            viewport.depth.start,
            viewport.depth.end,
        );
    }
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
