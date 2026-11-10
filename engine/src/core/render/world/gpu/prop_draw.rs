//! The props the arenas hold, drawn as the culling kernel left them: in every view's alpha-masked
//! pass, its prepass and its lights' shadow maps. Each part of a prop is drawn in its own material,
//! a world material or a cover's for the copies a cover strews, on that material's own pipeline,
//! built from bevy's material pipeline pieces as a mesh of the model slab's vertices would be, its
//! vertex stage reading the instance its command draws in place of a mesh's transform. A part
//! drawn by a shader of the game's draws through the shader's own instanced stage. A view draws
//! each part's levels in one indirect multi-draw, then the props' impostors from the atlas and
//! the boxes standing in for what the GPU lacks, each in one more. A model's impostor is baked
//! by the same pipelines, each part's shaders drawing the atlas's cells as instances. And for a
//! view that singles instances out ([`SingledProps`]), the same pipelines write each instance's
//! id over what the view shows of it, and the whole shape of each instance it outlines.

use std::any::TypeId;
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::core_pipeline::core_3d::{AlphaMask3d, CORE_3D_DEPTH_FORMAT};
use bevy::core_pipeline::prepass::{
    AlphaMask3dPrepass, OpaqueNoLightmap3dBatchSetKey, OpaqueNoLightmap3dBinKey,
    prepass_target_descriptors,
};
use bevy::ecs::query::ROQueryItem;
use bevy::ecs::system::lifetimeless::{Read, SRes};
use bevy::ecs::system::{SystemParam, SystemParamItem};
use bevy::material::MaterialProperties;
use bevy::material::key::{ErasedMaterialKey, ErasedMaterialPipelineKey, ErasedMeshPipelineKey};
use bevy::math::Affine3;
use bevy::mesh::{Mesh, MeshVertexBufferLayoutRef, MeshVertexBufferLayouts, PrimitiveTopology};
use bevy::pbr::{
    LightEntity, LightKeyCache, MATERIAL_BIND_GROUP_INDEX, MaterialFragmentShader,
    MaterialPipeline, MaterialVertexShader, MeshFlags, MeshPipeline, MeshPipelineKey,
    MeshTransforms, MeshUniform, MeshViewBindGroup, PreparedMaterial, PrepassFragmentShader,
    PrepassPipeline, PrepassPipelineSpecializer, PrepassVertexShader, PrepassViewBindGroup,
    SetMeshViewBindGroup, SetMeshViewBindingArrayBindGroup, SetPrepassViewBindGroup,
    SetPrepassViewEmptyBindGroup, Shadow, ShadowBatchSetKey, ShadowBinKey, ViewKeyCache,
    ViewKeyPrepassCache, alpha_mode_pipeline_key, init_prepass_pipeline,
    prepare_mesh_view_bind_groups,
};
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use bevy::render::erased_render_asset::ErasedRenderAssets;
use bevy::render::material_bind_groups::{
    MaterialBindGroupAllocators, prepare_material_bind_groups, write_material_bind_group_buffers,
};
use bevy::render::mesh::allocator::MeshSlabs;
use bevy::render::render_phase::{
    AddRenderCommand, BinnedPhaseItem, BinnedRenderPhaseType, DrawFunctions, InputUniformIndex,
    PhaseItem, RenderCommand, RenderCommandResult, TrackedRenderPass, ViewBinnedRenderPhases,
};
use bevy::render::render_resource::binding_types::{
    sampler, storage_buffer_read_only_sized, texture_2d_array, uniform_buffer,
};
use bevy::render::render_resource::{
    BindGroup, BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, BlendComponent,
    BlendFactor, BlendOperation, BlendState, Buffer, BufferInitDescriptor, BufferUsages,
    CachedRenderPipelineId, ColorTargetState, ColorWrites, CompareFunction, DepthBiasState,
    DepthStencilState, FragmentState, IndexFormat, MultisampleState, PipelineCache, PrimitiveState,
    PrimitiveTopology as RawTopology, RenderPipeline, RenderPipelineDescriptor, SamplerBindingType,
    ShaderStages, ShaderType, SpecializedMeshPipeline, StencilState, StorageBuffer, TextureFormat,
    TextureSampleType, UniformBuffer, VertexState, encase,
};
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::sync_world::MainEntity;
use bevy::render::view::{ExtractedView, RetainedViewEntity, ViewUniformOffset};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::{Shader, ShaderDefVal};

use super::arena::{ExtractedPropArena, InstanceArenas, InstanceRecord, LevelRange, ModelArena};
use super::clearing::Clearing;
use super::cull::{self, PropCulls, ViewCull};
use super::impostor::{
    self, ALBEDO_FORMAT, BAKE_DEPTH, BakeJob, CELLS, DEPTH_FORMAT, ImpostorAtlas, ImpostorOrders,
    NORMAL_FORMAT, Readback,
};
use crate::core::render::world::RenderProgress;

pub struct PropDrawPlugin;

impl Plugin for PropDrawPlugin {
    fn build(&self, app: &mut App) {
        bevy::shader::load_shader_library!(app, "instance.wesl");
        bevy::shader::load_shader_library!(app, "impostor_bake.wesl");
        bevy::asset::embedded_asset!(app, "instanced.wesl");
        bevy::asset::embedded_asset!(app, "impostor.wesl");
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<PropBindings>()
            .add_render_command::<AlphaMask3d, DrawLitProps>()
            .add_render_command::<AlphaMask3dPrepass, DrawPropDepth>()
            .add_render_command::<Shadow, DrawPropDepth>()
            .add_systems(RenderStartup, start.after(init_prepass_pipeline))
            .add_systems(
                Render,
                (
                    write_clearing.in_set(RenderSystems::PrepareResources),
                    (
                        bind_props.after(cull::prepare_occlusion),
                        draw_bakes
                            .after(prepare_mesh_view_bind_groups)
                            .after(prepare_material_bind_groups)
                            .after(write_material_bind_group_buffers),
                    )
                        .in_set(RenderSystems::PrepareBindGroups),
                    (
                        forget_drawn.before(QueueProps),
                        (queue_lit, queue_prepass, queue_shadows).in_set(QueueProps),
                    )
                        .in_set(RenderSystems::Queue),
                ),
            );
    }
}

/// The queues saying what each view draws of the props this frame, once what the views drew the
/// frame before is forgotten.
#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct QueueProps;

/// Lights a view's props in its alpha-masked pass, with its lights, shadows, ambient occlusion
/// and fog.
type DrawLitProps = (
    SetMeshViewBindGroup<0>,
    SetMeshViewBindingArrayBindGroup<1>,
    DrawProps<true>,
);

/// Writes the props' depth, and their normal and motion where the view keeps them: for the
/// prepass of a camera and for a light's shadow map.
type DrawPropDepth = (
    SetPrepassViewBindGroup<0>,
    SetPrepassViewEmptyBindGroup<1>,
    DrawProps<false>,
);

/// Where the props clear round the camera's subject on the picture, as the instanced stages read
/// it.
#[derive(ShaderType, Clone, Copy, Debug, Default)]
struct ClearingUniform {
    focus: Vec2,
    focus_radius: f32,
}

/// The bind group of a scenery's instances and the instances a view draws of it, where a mesh's
/// would be, and of a part's material after it.
const INSTANCE_GROUP: usize = 2;

/// The format of the ids a view's instances write.
pub(super) const IDS_FORMAT: TextureFormat = TextureFormat::R32Uint;

/// The format of the marks a view's outlined instances write: a channel for each set of them.
pub(super) const MARKS_FORMAT: TextureFormat = TextureFormat::Rg8Unorm;

/// How many samples of a pixel the marks are written on, for a shape's edge to cover the share
/// of each pixel it does, and its outline to move with it by less than a pixel at a time.
pub(super) const MARK_SAMPLES: u32 = 4;

/// The passes a part's material is drawn in: lit in a view's alpha-masked pass, its depth in a
/// view's prepass or a light's shadow map, baked into its model's impostor atlas, and for a view
/// that singles instances out, its instances' ids over what the view shows and the marks of
/// those it outlines.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum PropPass {
    Lit,
    Depth,
    Bake,
    Ids,
    Marks,
}

/// The bind groups each view's props are drawn with, by scenery and view, and the parts each
/// view draws of each scenery in each pass: on what pipeline, in what material, with which of
/// the view's commands.
#[derive(Resource, Default)]
struct PropBindings {
    /// Each view's instances of each scenery: for the depth passes, and for the lit ones.
    instances: HashMap<(Entity, RetainedViewEntity, bool), BindGroup>,
    drawn: HashMap<(Entity, RetainedViewEntity, PropPass), Vec<PartDraw>>,
    /// The impostor atlas, bound where a part's material would be.
    atlas: Option<BindGroup>,
}

/// A part a view draws: its pipeline, its material's bind group, and its levels' commands.
struct PartDraw {
    pipeline: CachedRenderPipelineId,
    material: BindGroup,
    first_command: u32,
    commands: u32,
}

/// How the props are drawn: bevy's material pipelines, as a material draws a mesh, with the
/// instances where a mesh's bindings would be, the model slab's vertices laid out as a mesh of
/// positions, normals, uvs and colours, and the pipeline of every part's material in every pass
/// and view it has been drawn in.
#[derive(Resource)]
struct PropPipeline {
    material: MaterialPipeline,
    prepass: PrepassPipeline,
    instances: BindGroupLayoutDescriptor,
    /// The instances with their scenery's slot and its outlined instances' marks after them, as
    /// the ids and the marks are written with.
    singled: BindGroupLayoutDescriptor,
    layout: MeshVertexBufferLayoutRef,
    /// The instanced stages of the world's own material.
    shader: Handle<Shader>,
    /// Where the props clear round the camera's subject, as the frame's clearing says.
    clearing: UniformBuffer<ClearingUniform>,
    /// Bevy's mesh data for every instance alike: placed where it stands, receiving shadows.
    mesh: StorageBuffer<Vec<MeshUniform>>,
    built: HashMap<
        (PropPass, MeshPipelineKey, TypeId, ErasedMaterialKey),
        Option<CachedRenderPipelineId>,
    >,
    impostors: ImpostorPipeline,
    bake: BakeBindings,
}

/// What a bake binds where a scenery's instances would be: one instance standing at the origin,
/// grown once and shown whole, drawn once a cell, and the cells after it.
struct BakeBindings {
    layout: BindGroupLayoutDescriptor,
    instance: Buffer,
    drawn: Buffer,
}

/// How the impostors and the proxies are drawn: the mesh pipeline's lit descriptor and the
/// prepass's depth one, each drawing `impostor.wesl` with the instances where a mesh's bindings
/// would be and the atlas after, one pipeline a pass, view key and shape.
struct ImpostorPipeline {
    shader: Handle<Shader>,
    atlas: BindGroupLayoutDescriptor,
    prepass_moving: BindGroupLayoutDescriptor,
    prepass_still: BindGroupLayoutDescriptor,
    empty: BindGroupLayoutDescriptor,
    depth_clip_control: bool,
    built: HashMap<(PropPass, MeshPipelineKey, bool), CachedRenderPipelineId>,
}

fn start(
    mut commands: Commands,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mesh: Res<MeshPipeline>,
    prepass: Res<PrepassPipeline>,
    mut layouts: ResMut<MeshVertexBufferLayouts>,
    assets: Res<AssetServer>,
) {
    let model = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]])
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32; 3]])
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32; 2]])
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.0f32; 4]]);
    let instances = BindGroupLayoutDescriptor::new(
        "prop_instances",
        &BindGroupLayoutEntries::with_indices(
            ShaderStages::VERTEX_FRAGMENT,
            (
                (0, storage_buffer_read_only_sized(false, None)),
                (100, storage_buffer_read_only_sized(false, None)),
                (101, storage_buffer_read_only_sized(false, None)),
                (102, uniform_buffer::<ClearingUniform>(false)),
                (103, storage_buffer_read_only_sized(false, None)),
                (104, storage_buffer_read_only_sized(false, None)),
            ),
        ),
    );
    let singled = BindGroupLayoutDescriptor::new(
        "prop_singled_instances",
        &BindGroupLayoutEntries::with_indices(
            ShaderStages::VERTEX_FRAGMENT,
            (
                (0, storage_buffer_read_only_sized(false, None)),
                (100, storage_buffer_read_only_sized(false, None)),
                (101, storage_buffer_read_only_sized(false, None)),
                (102, uniform_buffer::<ClearingUniform>(false)),
                (103, storage_buffer_read_only_sized(false, None)),
                (104, storage_buffer_read_only_sized(false, None)),
                (106, uniform_buffer::<u32>(false)),
                (107, storage_buffer_read_only_sized(false, None)),
            ),
        ),
    );
    let atlas = BindGroupLayoutDescriptor::new(
        "impostor_atlas",
        &BindGroupLayoutEntries::with_indices(
            ShaderStages::FRAGMENT,
            (
                (
                    100,
                    texture_2d_array(TextureSampleType::Float { filterable: true }),
                ),
                (
                    101,
                    texture_2d_array(TextureSampleType::Float { filterable: true }),
                ),
                (
                    102,
                    texture_2d_array(TextureSampleType::Float { filterable: true }),
                ),
                (103, sampler(SamplerBindingType::Filtering)),
            ),
        ),
    );
    let bake_instances = BindGroupLayoutDescriptor::new(
        "prop_bake_instances",
        &BindGroupLayoutEntries::with_indices(
            ShaderStages::VERTEX_FRAGMENT,
            (
                (0, storage_buffer_read_only_sized(false, None)),
                (100, storage_buffer_read_only_sized(false, None)),
                (101, storage_buffer_read_only_sized(false, None)),
                (102, uniform_buffer::<ClearingUniform>(false)),
                (105, storage_buffer_read_only_sized(false, None)),
            ),
        ),
    );
    let origin = InstanceRecord {
        position: [0.0; 3],
        yaw: 0.0,
        scale: 1.0,
        prop: 0,
        seed: 0,
        chunk: 0,
    };
    let whole = [0u32, 1.0f32.to_bits(), 1.0f32.to_bits(), 65535 << 1];
    let bake = BakeBindings {
        layout: bake_instances,
        instance: device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("prop_bake_instance"),
            contents: &origin.to_bytes(),
            usage: BufferUsages::STORAGE,
        }),
        drawn: device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("prop_bake_drawn"),
            contents: &whole
                .iter()
                .flat_map(|word| word.to_le_bytes())
                .collect::<Vec<u8>>()
                .repeat((CELLS * CELLS) as usize),
            usage: BufferUsages::STORAGE,
        }),
    };
    let still = MeshTransforms {
        world_from_local: Affine3::IDENTITY,
        previous_world_from_local: Affine3::IDENTITY,
        flags: (MeshFlags::SHADOW_RECEIVER | MeshFlags::SIGN_DETERMINANT_MODEL_3X3).bits(),
    };
    let mut every = StorageBuffer::from(vec![MeshUniform::new(
        &still,
        0,
        default(),
        None,
        None,
        None,
        None,
        None,
    )]);
    every.write_buffer(&device, &queue);
    commands.insert_resource(PropPipeline {
        material: MaterialPipeline {
            mesh_pipeline: mesh.clone(),
        },
        prepass: prepass.clone(),
        instances,
        singled,
        layout: model.get_mesh_vertex_buffer_layout(&mut layouts),
        shader: bevy::asset::load_embedded_asset!(assets.as_ref(), "instanced.wesl"),
        clearing: UniformBuffer::default(),
        mesh: every,
        built: HashMap::default(),
        bake,
        impostors: ImpostorPipeline {
            shader: bevy::asset::load_embedded_asset!(assets.as_ref(), "impostor.wesl"),
            atlas,
            prepass_moving: prepass.view_layout_motion_vectors.clone(),
            prepass_still: prepass.view_layout_no_motion_vectors.clone(),
            empty: prepass.empty_layout.clone(),
            depth_clip_control: prepass.depth_clip_control_supported,
            built: HashMap::default(),
        },
    });
}

/// Binds what each view draws of each scenery: the scenery's instances, and those its commands
/// draw.
fn bind_props(
    device: Res<RenderDevice>,
    cache: Res<PipelineCache>,
    pipeline: Option<Res<PropPipeline>>,
    atlas: Option<Res<ImpostorAtlas>>,
    culls: Res<PropCulls>,
    arenas: Res<InstanceArenas>,
    mut bindings: ResMut<PropBindings>,
) {
    let Some(pipeline) = pipeline else {
        return;
    };
    bindings.instances.clear();
    let (Some(mesh), Some(clearing), layout) = (
        pipeline.mesh.binding(),
        pipeline.clearing.binding(),
        cache.get_bind_group_layout(&pipeline.instances),
    ) else {
        return;
    };
    if bindings.atlas.is_none()
        && let Some(atlas) = atlas
    {
        bindings.atlas = Some(device.create_bind_group(
            "impostor_atlas",
            &cache.get_bind_group_layout(&pipeline.impostors.atlas),
            &BindGroupEntries::with_indices((
                (100, &atlas.albedo_view),
                (101, &atlas.normal_view),
                (102, &atlas.depth_view),
                (103, &atlas.sampler),
            )),
        ));
    }
    for (scenery, cull) in &culls.0 {
        let (Some(records), Some((props, layers))) = (
            arenas.0.get(scenery).and_then(|arena| arena.slab.buffer()),
            cull.table.as_ref().and_then(|table| table.prop_bindings()),
        ) else {
            continue;
        };
        for (view, culled) in &cull.views {
            for (lit, drawn) in [(false, culled.drawn()), (true, culled.lit_drawn())] {
                bindings.instances.insert(
                    (*scenery, *view, lit),
                    device.create_bind_group(
                        "prop_instances",
                        &layout,
                        &BindGroupEntries::with_indices((
                            (0, mesh.clone()),
                            (100, records.as_entire_binding()),
                            (101, drawn.as_entire_binding()),
                            (102, clearing.clone()),
                            (103, props.clone()),
                            (104, layers.clone()),
                        )),
                    ),
                );
            }
        }
    }
}

/// Bakes every impostor waiting whose parts' pipelines are built and materials bound: each model
/// drawn into its layer of the atlas by its parts' own shaders, the view bound as the lit pass
/// binds it and the atlas's cells where the instances would be, and its texels sent for.
fn draw_bakes(bakes: BakeQueue) {
    let BakeQueue {
        device,
        queue,
        cache,
        pipeline,
        atlas,
        models,
        materials,
        allocators,
        keys,
        prepass_keys,
        light_keys,
        views,
        mut orders,
        progress,
    } = bakes;
    if orders.bakes.is_empty() {
        return;
    }
    let (Some(mut pipeline), Some(atlas)) = (pipeline, atlas) else {
        progress.left_out();
        return;
    };
    let (Some(vertices), Some(indices), Some(mesh), Some(clearing)) = (
        models.vertices.buffer(),
        models.indices.buffer(),
        pipeline.mesh.buffer().cloned(),
        pipeline.clearing.buffer().cloned(),
    ) else {
        progress.left_out();
        return;
    };
    let Some((view_key, view_group)) = views
        .iter()
        .find_map(|(view, group)| Some((*keys.get(&view.retained_view_entity)?, group)))
    else {
        progress.left_out();
        return;
    };
    let warmth = Warmth {
        lit: keys.values().copied().collect(),
        depth: prepass_keys
            .values()
            .chain(light_keys.values())
            .copied()
            .collect(),
    };
    warmth.warm_impostors(&mut pipeline, &cache);
    let jobs = std::mem::take(&mut orders.bakes);
    let mut encoder = device.create_command_encoder(&default());
    let mut baked = false;
    for job in jobs {
        let Some(parts) = bake_parts(
            &mut pipeline,
            &cache,
            view_key,
            &warmth,
            &materials,
            &allocators,
            &job,
        ) else {
            orders.bakes.push(job);
            continue;
        };
        let cells = impostor::bake_cells(&job.placed, job.overhang);
        let mut written = encase::StorageBuffer::new(Vec::<u8>::new());
        written
            .write(&cells)
            .expect("the cells write as a storage buffer");
        let cells = device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("prop_bake_cells"),
            contents: written.as_ref(),
            usage: BufferUsages::STORAGE,
        });
        let instances = device.create_bind_group(
            "prop_bake_instances",
            &cache.get_bind_group_layout(&pipeline.bake.layout),
            &BindGroupEntries::with_indices((
                (0, mesh.as_entire_binding()),
                (100, pipeline.bake.instance.as_entire_binding()),
                (101, pipeline.bake.drawn.as_entire_binding()),
                (102, clearing.as_entire_binding()),
                (105, cells.as_entire_binding()),
            )),
        );
        let targets = atlas.bake_targets(job.layer);
        let mut pass = targets.begin(&mut encoder);
        pass.set_vertex_buffer(0, *vertices.slice(..));
        pass.set_index_buffer(*indices.slice(..), IndexFormat::Uint32);
        pass.set_bind_group(0, Some(&*view_group.main), &view_group.main_offsets);
        pass.set_bind_group(1, Some(&*view_group.binding_array), &[]);
        pass.set_bind_group(INSTANCE_GROUP as u32, Some(&*instances), &[]);
        for part in &parts {
            pass.set_pipeline(part.pipeline);
            pass.set_bind_group(MATERIAL_BIND_GROUP_INDEX as u32, Some(&*part.material), &[]);
            pass.draw_indexed(
                part.level.first_index..part.level.first_index + part.level.indices,
                part.first_vertex as i32,
                0..CELLS * CELLS,
            );
        }
        drop(pass);
        orders.readbacks.push(Readback::of(
            &device,
            &mut encoder,
            &atlas,
            job.layer,
            job.key,
        ));
        baked = true;
    }
    if baked {
        queue.submit([encoder.finish()]);
    }
    if !orders.bakes.is_empty() {
        progress.left_out();
    }
}

/// What the bakes draw with.
#[derive(SystemParam)]
struct BakeQueue<'w, 's> {
    device: Res<'w, RenderDevice>,
    queue: Res<'w, RenderQueue>,
    cache: Res<'w, PipelineCache>,
    pipeline: Option<ResMut<'w, PropPipeline>>,
    atlas: Option<Res<'w, ImpostorAtlas>>,
    models: Res<'w, ModelArena>,
    materials: Res<'w, ErasedRenderAssets<PreparedMaterial>>,
    allocators: Res<'w, MaterialBindGroupAllocators>,
    keys: Res<'w, ViewKeyCache>,
    prepass_keys: Res<'w, ViewKeyPrepassCache>,
    light_keys: Res<'w, LightKeyCache>,
    views:
        Query<'w, 's, (&'static ExtractedView, &'static MeshViewBindGroup), Without<LightEntity>>,
    orders: ResMut<'w, ImpostorOrders>,
    progress: Res<'w, RenderProgress>,
}

/// The keys of every view there is, lit, and of every depth view, the cameras' prepasses and the
/// lights', which every pipeline a model may draw with is queued for as the model is baked: a
/// model is baked as it is readied, under the cover, so no pipeline compiles at its first draw.
struct Warmth {
    lit: Vec<MeshPipelineKey>,
    depth: Vec<MeshPipelineKey>,
}

impl Warmth {
    fn warm(
        &self,
        pipeline: &mut PropPipeline,
        cache: &PipelineCache,
        kind: TypeId,
        properties: &Arc<MaterialProperties>,
    ) {
        for &view in &self.lit {
            pipeline.specialize(cache, view, PropPass::Lit, kind, properties);
        }
        for &view in &self.depth {
            pipeline.specialize(cache, view, PropPass::Depth, kind, properties);
        }
    }

    fn warm_impostors(&self, pipeline: &mut PropPipeline, cache: &PipelineCache) {
        for proxy in [false, true] {
            for &view in &self.lit {
                pipeline.specialize_impostors(cache, view, PropPass::Lit, proxy);
            }
            for &view in &self.depth {
                pipeline.specialize_impostors(cache, view, PropPass::Depth, proxy);
            }
        }
    }
}

/// A part as a bake draws it: on its material's bake pipeline, in its material, the level the
/// bake draws of it.
struct BakePart<'a> {
    pipeline: &'a RenderPipeline,
    material: BindGroup,
    level: LevelRange,
    first_vertex: u32,
}

/// Every part of `job`'s model the bake draws, once each one's pipeline is built and its material
/// bound; a part no pass can draw is left out, as the lit pass leaves it out.
fn bake_parts<'a>(
    pipeline: &mut PropPipeline,
    cache: &'a PipelineCache,
    view: MeshPipelineKey,
    warmth: &Warmth,
    materials: &ErasedRenderAssets<PreparedMaterial>,
    allocators: &MaterialBindGroupAllocators,
    job: &BakeJob,
) -> Option<Vec<BakePart<'a>>> {
    let mut parts = Vec::new();
    for (part, material) in job.placed.parts.iter().zip(job.materials.iter()) {
        let prepared = materials.get(material.id())?;
        warmth.warm(pipeline, cache, material.type_id(), &prepared.properties);
        let Some(level) = impostor::bake_level(&job.placed, job.overhang, part) else {
            continue;
        };
        let bound = allocators
            .get(&material.type_id())?
            .get(prepared.binding.group)?
            .bind_group()?
            .clone();
        let Some(id) = pipeline.specialize(
            cache,
            view,
            PropPass::Bake,
            material.type_id(),
            &prepared.properties,
        ) else {
            continue;
        };
        parts.push(BakePart {
            pipeline: cache.get_render_pipeline(id)?,
            material: bound,
            level: *level,
            first_vertex: part.first_vertex,
        });
    }
    Some(parts)
}

/// Forgets what the views drew last frame, for the queues to say what they draw this frame.
fn forget_drawn(mut bindings: ResMut<PropBindings>) {
    bindings.drawn.clear();
}

/// Writes where the props clear round the camera's subject this frame.
fn write_clearing(
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    clearing: Res<Clearing>,
    pipeline: Option<ResMut<PropPipeline>>,
) {
    let Some(mut pipeline) = pipeline else {
        return;
    };
    let aimed = clearing
        .0
        .map_or(ClearingUniform::default(), |aim| ClearingUniform {
            focus: aim.focus,
            focus_radius: aim.focus_radius,
        });
    pipeline.clearing.set(aimed);
    pipeline.clearing.write_buffer(&device, &queue);
}

/// What the props draw with that the queues read.
#[derive(SystemParam)]
pub(super) struct PropQueue<'w, 's> {
    cache: Res<'w, PipelineCache>,
    pipeline: Option<ResMut<'w, PropPipeline>>,
    culls: Res<'w, PropCulls>,
    arenas: Res<'w, InstanceArenas>,
    materials: Res<'w, ErasedRenderAssets<PreparedMaterial>>,
    allocators: Res<'w, MaterialBindGroupAllocators>,
    bindings: ResMut<'w, PropBindings>,
    progress: Res<'w, RenderProgress>,
    sceneries: Query<'w, 's, (Entity, &'static MainEntity), With<ExtractedPropArena>>,
}

impl PropQueue<'_, '_> {
    /// Has `view` write the ids of the instances it shows this frame, for [`SingledProps`] to
    /// draw.
    pub(super) fn ids(&mut self, view: RetainedViewEntity) {
        self.drawn(view, singled_view(1), PropPass::Ids, false);
    }

    /// Has `view` mark the instances it outlines this frame, for [`SingledProps`] to draw.
    pub(super) fn marks(&mut self, view: RetainedViewEntity) {
        self.drawn(view, singled_view(MARK_SAMPLES), PropPass::Marks, false);
    }

    /// The parts `view` draws of each scenery in `pass`, on the pipelines for the view's key
    /// `view_key`, as each is queued: each scenery with its entity in the render world and the
    /// main world, and the pipeline of the first part it draws. A light's view draws only the
    /// parts that cast shadows, and no box standing in for a model is singled out.
    fn drawn(
        &mut self,
        view: RetainedViewEntity,
        view_key: MeshPipelineKey,
        pass: PropPass,
        light: bool,
    ) -> Vec<(Entity, MainEntity, CachedRenderPipelineId)> {
        let Some(pipeline) = self.pipeline.as_mut() else {
            return Vec::new();
        };
        let mut queued = Vec::new();
        for (entity, &main) in &self.sceneries {
            let scenery = main.id();
            let Some(arena) = self.arenas.0.get(&scenery) else {
                continue;
            };
            let Some(cull) = self.culls.0.get(&scenery) else {
                if !arena.chunks.is_empty() {
                    self.progress.left_out();
                }
                continue;
            };
            if !cull.views.contains_key(&view) {
                continue;
            }
            let Some(table) = cull.table.as_ref() else {
                if !arena.chunks.is_empty() {
                    self.progress.left_out();
                }
                continue;
            };
            let progress = &self.progress;
            let mut parts: Vec<PartDraw> = table
                .table()
                .buckets
                .iter()
                .filter_map(|bucket| {
                    let Some(dressed) = arena.dressed.get(&bucket.prop) else {
                        progress.left_out();
                        return None;
                    };
                    if light && !dressed.shadow {
                        return None;
                    }
                    let material = dressed.materials.get(bucket.part as usize)?;
                    let Some(prepared) = self.materials.get(material.id()) else {
                        progress.left_out();
                        return None;
                    };
                    if light && !prepared.properties.shadows_enabled {
                        return None;
                    }
                    let Some(bound) = self
                        .allocators
                        .get(&material.type_id())
                        .and_then(|allocator| allocator.get(prepared.binding.group))
                        .and_then(|group| group.bind_group())
                        .cloned()
                    else {
                        progress.left_out();
                        return None;
                    };
                    let id = pipeline.specialize(
                        &self.cache,
                        view_key,
                        pass,
                        material.type_id(),
                        &prepared.properties,
                    )?;
                    Some(PartDraw {
                        pipeline: id,
                        material: bound,
                        first_command: bucket.commands.start,
                        commands: bucket.commands.len() as u32,
                    })
                })
                .collect();
            if let Some(atlas) = &self.bindings.atlas {
                let table = table.table();
                let singled = matches!(pass, PropPass::Ids | PropPass::Marks);
                for (proxy, commands) in [(false, &table.impostors), (true, &table.proxies)] {
                    if commands.is_empty() || (proxy && singled) {
                        continue;
                    }
                    let Some(id) =
                        pipeline.specialize_impostors(&self.cache, view_key, pass, proxy)
                    else {
                        progress.left_out();
                        continue;
                    };
                    parts.push(PartDraw {
                        pipeline: id,
                        material: atlas.clone(),
                        first_command: commands.start,
                        commands: commands.len() as u32,
                    });
                }
            } else {
                progress.left_out();
            }
            let Some(first) = parts.first().map(|part| part.pipeline) else {
                continue;
            };
            self.bindings.drawn.insert((scenery, view, pass), parts);
            queued.push((entity, main, first));
        }
        queued
    }
}

fn queue_lit(
    mut queue: PropQueue,
    draws: Res<DrawFunctions<AlphaMask3d>>,
    mut phases: ResMut<ViewBinnedRenderPhases<AlphaMask3d>>,
    keys: Res<ViewKeyCache>,
    views: Query<&ExtractedView>,
    mut queued: Local<Queued>,
) {
    let draw_function = draws.read().id::<DrawLitProps>();
    queued.keep_only(views.iter().map(|view| view.retained_view_entity));
    for view in &views {
        let (Some(phase), Some(&key)) = (
            phases.get_mut(&view.retained_view_entity),
            keys.get(&view.retained_view_entity),
        ) else {
            queued.forget(view.retained_view_entity);
            continue;
        };
        let drawn = queue.drawn(view.retained_view_entity, key, PropPass::Lit, false);
        queued.queue(phase, view.retained_view_entity, &drawn, |pipeline| {
            (
                OpaqueNoLightmap3dBatchSetKey {
                    pipeline,
                    draw_function,
                    material_bind_group_index: None,
                    slabs: MeshSlabs::default(),
                },
                OpaqueNoLightmap3dBinKey {
                    asset_id: AssetId::<Mesh>::default().untyped(),
                },
            )
        });
    }
}

fn queue_prepass(
    mut queue: PropQueue,
    draws: Res<DrawFunctions<AlphaMask3dPrepass>>,
    mut phases: ResMut<ViewBinnedRenderPhases<AlphaMask3dPrepass>>,
    keys: Res<ViewKeyPrepassCache>,
    views: Query<&ExtractedView, Without<LightEntity>>,
    mut queued: Local<Queued>,
) {
    let draw_function = draws.read().id::<DrawPropDepth>();
    queued.keep_only(views.iter().map(|view| view.retained_view_entity));
    for view in &views {
        let (Some(phase), Some(&key)) = (
            phases.get_mut(&view.retained_view_entity),
            keys.get(&view.retained_view_entity),
        ) else {
            queued.forget(view.retained_view_entity);
            continue;
        };
        let drawn = queue.drawn(view.retained_view_entity, key, PropPass::Depth, false);
        queued.queue(phase, view.retained_view_entity, &drawn, |pipeline| {
            (
                OpaqueNoLightmap3dBatchSetKey {
                    pipeline,
                    draw_function,
                    material_bind_group_index: None,
                    slabs: MeshSlabs::default(),
                },
                OpaqueNoLightmap3dBinKey {
                    asset_id: AssetId::<Mesh>::default().untyped(),
                },
            )
        });
    }
}

fn queue_shadows(
    mut queue: PropQueue,
    draws: Res<DrawFunctions<Shadow>>,
    mut phases: ResMut<ViewBinnedRenderPhases<Shadow>>,
    keys: Res<LightKeyCache>,
    views: Query<&ExtractedView, With<LightEntity>>,
    mut queued: Local<Queued>,
) {
    let draw_function = draws.read().id::<DrawPropDepth>();
    queued.keep_only(views.iter().map(|view| view.retained_view_entity));
    for view in &views {
        let (Some(phase), Some(&key)) = (
            phases.get_mut(&view.retained_view_entity),
            keys.get(&view.retained_view_entity),
        ) else {
            queued.forget(view.retained_view_entity);
            continue;
        };
        let drawn = queue.drawn(view.retained_view_entity, key, PropPass::Depth, true);
        queued.queue(phase, view.retained_view_entity, &drawn, |pipeline| {
            (
                ShadowBatchSetKey {
                    pipeline,
                    draw_function,
                    material_bind_group_index: None,
                    slabs: MeshSlabs::default(),
                },
                ShadowBinKey {
                    asset_id: AssetId::<Mesh>::default().untyped(),
                },
            )
        });
    }
}

/// The sceneries each view of a phase was queued with, by the pipeline each was queued on.
#[derive(Default)]
struct Queued(HashMap<RetainedViewEntity, HashMap<MainEntity, CachedRenderPipelineId>>);

impl Queued {
    /// Forgets every view but `live`. Bevy drops the phase of a view that isn't drawn in a frame,
    /// such as the far cascade of a sun's shadows while the shadows take one, and starts it
    /// empty when the view is drawn again, so what was queued into the dropped phase is queued
    /// anew.
    fn keep_only(&mut self, live: impl IntoIterator<Item = RetainedViewEntity>) {
        let live: HashSet<RetainedViewEntity> = live.into_iter().collect();
        self.0.retain(|view, _| live.contains(view));
    }

    /// Forgets what was queued into `view`, whose phase can't be queued into this frame.
    fn forget(&mut self, view: RetainedViewEntity) {
        self.0.remove(&view);
    }

    /// Queues every one of `sceneries` into `phase` on its pipeline, taking out first what was
    /// queued on another pipeline or is gone.
    fn queue<P: BinnedPhaseItem>(
        &mut self,
        phase: &mut bevy::render::render_phase::BinnedRenderPhase<P>,
        view: RetainedViewEntity,
        sceneries: &[(Entity, MainEntity, CachedRenderPipelineId)],
        keys: impl Fn(CachedRenderPipelineId) -> (P::BatchSetKey, P::BinKey),
    ) {
        let queued = self.0.entry(view).or_default();
        queued.retain(|main, was| {
            let kept = sceneries
                .iter()
                .any(|(_, scenery, pipeline)| scenery == main && pipeline == was);
            if !kept {
                phase.remove(*main);
            }
            kept
        });
        for &(entity, main, pipeline) in sceneries {
            if queued.insert(main, pipeline) == Some(pipeline) {
                continue;
            }
            let (batch_set, bin) = keys(pipeline);
            phase.add(
                batch_set,
                bin,
                (entity, main),
                InputUniformIndex::default(),
                BinnedRenderPhaseType::NonMesh,
            );
        }
    }
}

impl PropPipeline {
    /// The pipeline drawing a part in a material of the kind `kind` that `properties` describes,
    /// in `pass`, in a view of `view`'s key, queued the first time it is asked for.
    fn specialize(
        &mut self,
        cache: &PipelineCache,
        view: MeshPipelineKey,
        pass: PropPass,
        kind: TypeId,
        properties: &Arc<MaterialProperties>,
    ) -> Option<CachedRenderPipelineId> {
        let key = (pass, view, kind, properties.material_key.clone());
        if let Some(&id) = self.built.get(&key) {
            return id;
        }
        let descriptor = match pass {
            PropPass::Lit => self.lit(view, kind, properties, false),
            PropPass::Depth => self.depth(view, kind, properties),
            PropPass::Bake => self.bake(view, kind, properties),
            PropPass::Ids => self.ids(view, kind, properties),
            PropPass::Marks => self.marks(view, kind, properties),
        };
        let id = descriptor.map(|descriptor| cache.queue_render_pipeline(descriptor));
        self.built.insert(key, id);
        id
    }

    /// The pipeline bevy would draw a mesh of the model slab's layout in the material in, lit;
    /// or, where `bake`, its shaders baking an impostor, which draws on one sample and drops its
    /// bare pixels itself.
    fn lit(
        &self,
        view: MeshPipelineKey,
        kind: TypeId,
        properties: &MaterialProperties,
        bake: bool,
    ) -> Option<RenderPipelineDescriptor> {
        let msaa = if bake {
            Msaa::Off
        } else {
            Msaa::from_samples(view.msaa_samples())
        };
        let mesh_key =
            view | MeshPipelineKey::from_primitive_topology_and_strip_index(
                PrimitiveTopology::TriangleList,
                None,
            ) | properties
                .mesh_pipeline_key_bits
                .downcast::<MeshPipelineKey>()
                | alpha_mode_pipeline_key(properties.alpha_mode, &msaa);
        let mut descriptor = self
            .material
            .mesh_pipeline
            .specialize(mesh_key, &self.layout)
            .ok()?;
        let untouched = descriptor.vertex.shader.clone();
        let mut defs = vec![ShaderDefVal::UInt(
            "MATERIAL_BIND_GROUP".into(),
            MATERIAL_BIND_GROUP_INDEX as u32,
        )];
        if properties.bindless {
            defs.push("BINDLESS".into());
        }
        if bake {
            defs.push("IMPOSTOR_BAKE".into());
        }
        if let Some(shader) = properties.get_shader(MaterialVertexShader) {
            descriptor.vertex.shader = shader;
        }
        if let Some(shader) = properties.get_shader(MaterialFragmentShader) {
            descriptor.fragment.as_mut()?.shader = shader;
        }
        descriptor.layout.insert(
            MATERIAL_BIND_GROUP_INDEX,
            properties.material_layout.clone()?,
        );
        descriptor.vertex.shader_defs.extend(defs.iter().cloned());
        descriptor.fragment.as_mut()?.shader_defs.extend(defs);
        if let Some(specialize) = properties.user_specialize {
            specialize(
                &self.material,
                &mut descriptor,
                &self.layout,
                material_key(mesh_key, kind, properties),
            )
            .ok()?;
        }
        descriptor.label = Some("prop_lit".into());
        if !bake && view.contains(MeshPipelineKey::DEPTH_PREPASS) {
            shade_only_what_shows(&mut descriptor);
        }
        Some(self.instanced(descriptor, &untouched))
    }

    /// The lit pipeline's shaders baking an impostor: drawing the atlas's cells where the
    /// instances would be, into a layer's albedo, normal and depth, sorted by the bake's own
    /// depth.
    fn bake(
        &self,
        view: MeshPipelineKey,
        kind: TypeId,
        properties: &MaterialProperties,
    ) -> Option<RenderPipelineDescriptor> {
        let mut descriptor = self.lit(view, kind, properties, true)?;
        descriptor.label = Some("prop_bake".into());
        descriptor.layout[INSTANCE_GROUP] = self.bake.layout.clone();
        // Bevy's material shaders take a pixel's normal from the view's normal prepass where the
        // view has one; the bake borrows a camera's view, whose prepass holds whatever that camera
        // last drew there, never the model's own faces.
        let from_prepass = ShaderDefVal::from("LOAD_PREPASS_NORMALS");
        descriptor
            .vertex
            .shader_defs
            .retain(|def| *def != from_prepass);
        descriptor
            .fragment
            .as_mut()?
            .shader_defs
            .retain(|def| *def != from_prepass);
        let target = |format| {
            Some(ColorTargetState {
                format,
                blend: None,
                write_mask: ColorWrites::ALL,
            })
        };
        descriptor.fragment.as_mut()?.targets = vec![
            target(ALBEDO_FORMAT),
            target(NORMAL_FORMAT),
            target(DEPTH_FORMAT),
        ];
        descriptor.depth_stencil = Some(DepthStencilState {
            format: BAKE_DEPTH,
            depth_write_enabled: Some(true),
            depth_compare: Some(CompareFunction::LessEqual),
            stencil: StencilState::default(),
            bias: DepthBiasState::default(),
        });
        descriptor.multisample = MultisampleState::default();
        Some(descriptor)
    }

    /// The pipeline bevy's prepass would write a mesh of the model slab's layout in the material
    /// with: to a camera's prepass or a light's shadow map, as `view`'s key says.
    fn depth(
        &self,
        view: MeshPipelineKey,
        kind: TypeId,
        properties: &Arc<MaterialProperties>,
    ) -> Option<RenderPipelineDescriptor> {
        let mut mesh_key =
            view | MeshPipelineKey::from_primitive_topology_and_strip_index(
                PrimitiveTopology::TriangleList,
                None,
            ) | properties
                .mesh_pipeline_key_bits
                .downcast::<MeshPipelineKey>()
                | MeshPipelineKey::MAY_DISCARD;
        if properties.get_shader(PrepassVertexShader).is_some()
            || properties.get_shader(PrepassFragmentShader).is_some()
        {
            mesh_key |= MeshPipelineKey::PREPASS_READS_MATERIAL;
        }
        let specializer = PrepassPipelineSpecializer {
            pipeline: self.prepass.clone(),
            properties: properties.clone(),
        };
        let mut descriptor = specializer
            .specialize(material_key(mesh_key, kind, properties), &self.layout)
            .ok()?;
        descriptor.label = Some("prop_depth".into());
        Some(self.instanced(descriptor, &self.prepass.default_prepass_shader))
    }

    /// The depth pipeline's stages writing each instance's id wherever its depth is the very one
    /// the view holds there: the stages that stood the instance in the view stand it here, so
    /// however a shader bends, sways or cuts its model, its id covers what the view shows of it
    /// and nothing else.
    fn ids(
        &self,
        view: MeshPipelineKey,
        kind: TypeId,
        properties: &Arc<MaterialProperties>,
    ) -> Option<RenderPipelineDescriptor> {
        let mut descriptor = self.singled(view, kind, properties, "PICK", id_target())?;
        descriptor.label = Some("prop_ids".into());
        shade_only_what_shows(&mut descriptor);
        Some(descriptor)
    }

    /// The depth pipeline's stages marking the whole shape of each instance a view outlines,
    /// as its shaders bend, sway and cut it, whatever stands before it: each set of instances in
    /// a channel of its own, where the shapes of a set add up.
    fn marks(
        &self,
        view: MeshPipelineKey,
        kind: TypeId,
        properties: &Arc<MaterialProperties>,
    ) -> Option<RenderPipelineDescriptor> {
        let mut descriptor = self.singled(view, kind, properties, "MARK", mark_target())?;
        descriptor.label = Some("prop_marks".into());
        descriptor.depth_stencil = None;
        Some(descriptor)
    }

    /// The depth pipeline of a material with its stages built for `singling` and writing
    /// `target`, the scenery's slot and its marks bound after the instances.
    fn singled(
        &self,
        view: MeshPipelineKey,
        kind: TypeId,
        properties: &Arc<MaterialProperties>,
        singling: &'static str,
        target: ColorTargetState,
    ) -> Option<RenderPipelineDescriptor> {
        let mut descriptor = self.depth(view, kind, properties)?;
        descriptor.layout[INSTANCE_GROUP] = self.singled.clone();
        let stages = descriptor.vertex.shader.clone();
        let defs = descriptor.vertex.shader_defs.clone();
        let fragment = descriptor.fragment.get_or_insert_with(|| FragmentState {
            shader: stages,
            shader_defs: defs,
            ..default()
        });
        fragment.targets = vec![Some(target)];
        fragment.shader_defs.push(singling.into());
        descriptor.vertex.shader_defs.push(singling.into());
        Some(descriptor)
    }

    /// The pipeline drawing the impostors, or the proxies where `proxy`, in `pass`, in a view of
    /// `view`'s key, queued the first time it is asked for. No impostor is baked by one.
    fn specialize_impostors(
        &mut self,
        cache: &PipelineCache,
        view: MeshPipelineKey,
        pass: PropPass,
        proxy: bool,
    ) -> Option<CachedRenderPipelineId> {
        let key = (pass, view, proxy);
        if let Some(&id) = self.impostors.built.get(&key) {
            return Some(id);
        }
        let descriptor = match pass {
            PropPass::Lit => self.impostors_lit(view, proxy)?,
            PropPass::Depth => self.impostors_depth(view, proxy),
            PropPass::Ids => self.impostors_ids(view, proxy),
            PropPass::Marks => self.impostors_marks(view, proxy),
            PropPass::Bake => return None,
        };
        let id = cache.queue_render_pipeline(descriptor);
        self.impostors.built.insert(key, id);
        Some(id)
    }

    /// The mesh pipeline's own lit descriptor, drawing the impostor stages with the instances
    /// where a mesh's bindings would be and the atlas after.
    fn impostors_lit(
        &self,
        view: MeshPipelineKey,
        proxy: bool,
    ) -> Option<RenderPipelineDescriptor> {
        let mesh_key =
            view | MeshPipelineKey::from_primitive_topology_and_strip_index(
                PrimitiveTopology::TriangleList,
                None,
            ) | MeshPipelineKey::MAY_DISCARD;
        let mut descriptor = self
            .material
            .mesh_pipeline
            .specialize(mesh_key, &self.layout)
            .ok()?;
        let mut defs = shape_defs(proxy);
        defs.push("STANDARD_MATERIAL_DIFFUSE_TRANSMISSION".into());
        descriptor.label = Some(if proxy { "proxy_lit" } else { "impostor_lit" }.into());
        descriptor.layout.truncate(INSTANCE_GROUP);
        descriptor.layout.push(self.instances.clone());
        descriptor.layout.push(self.impostors.atlas.clone());
        descriptor.vertex.shader = self.impostors.shader.clone();
        descriptor.vertex.buffers = Vec::new();
        descriptor.vertex.shader_defs.extend(defs.iter().cloned());
        let fragment = descriptor.fragment.as_mut()?;
        fragment.shader = self.impostors.shader.clone();
        fragment.shader_defs.extend(defs);
        descriptor.primitive.cull_mode = None;
        Some(descriptor)
    }

    /// As bevy's prepass draws an opaque mesh, for the impostor stages: depth always, the normal
    /// and motion the view keeps, depth unclipped toward a directional light.
    fn impostors_depth(&self, view: MeshPipelineKey, proxy: bool) -> RenderPipelineDescriptor {
        let normal = view.contains(MeshPipelineKey::NORMAL_PREPASS);
        let moving = view.contains(MeshPipelineKey::MOTION_VECTOR_PREPASS);
        let unclipped = view.contains(MeshPipelineKey::UNCLIPPED_DEPTH_ORTHO);
        let emulated = unclipped && !self.impostors.depth_clip_control;
        let mut defs = shape_defs(proxy);
        defs.push("PREPASS_PIPELINE".into());
        if normal {
            defs.push("NORMAL_PREPASS".into());
        }
        if moving {
            defs.push("MOTION_VECTOR_PREPASS".into());
        }
        if emulated {
            defs.push("UNCLIPPED_DEPTH_ORTHO_EMULATION".into());
        }
        let mut targets = prepass_target_descriptors(normal, moving, false);
        if targets.iter().all(Option::is_none) {
            targets.clear();
        }
        defs.push("PREPASS_FRAGMENT".into());
        RenderPipelineDescriptor {
            label: Some(
                if proxy {
                    "proxy_depth"
                } else {
                    "impostor_depth"
                }
                .into(),
            ),
            layout: vec![
                if moving {
                    self.impostors.prepass_moving.clone()
                } else {
                    self.impostors.prepass_still.clone()
                },
                self.impostors.empty.clone(),
                self.instances.clone(),
                self.impostors.atlas.clone(),
            ],
            vertex: VertexState {
                shader: self.impostors.shader.clone(),
                shader_defs: defs.clone(),
                buffers: Vec::new(),
                ..default()
            },
            fragment: Some(FragmentState {
                shader: self.impostors.shader.clone(),
                shader_defs: defs,
                targets,
                ..default()
            }),
            primitive: PrimitiveState {
                topology: RawTopology::TriangleList,
                cull_mode: None,
                unclipped_depth: unclipped && self.impostors.depth_clip_control,
                ..default()
            },
            depth_stencil: Some(DepthStencilState {
                format: CORE_3D_DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(CompareFunction::GreaterEqual),
                stencil: StencilState::default(),
                bias: DepthBiasState::default(),
            }),
            multisample: MultisampleState {
                count: view.msaa_samples(),
                ..default()
            },
            ..default()
        }
    }

    /// The impostors' depth stages writing each instance's id where the depth its picture lies
    /// at is the one the view holds there, or nearer: the view holds nothing nearer than what it
    /// shows, so that is where it shows the picture.
    fn impostors_ids(&self, view: MeshPipelineKey, proxy: bool) -> RenderPipelineDescriptor {
        let mut descriptor = self.impostors_singled(view, proxy, "PICK", id_target());
        descriptor.label = Some("impostor_ids".into());
        if let Some(depth) = &mut descriptor.depth_stencil {
            depth.depth_write_enabled = Some(false);
        }
        descriptor
    }

    /// The impostors' depth stages marking the whole picture of each instance a view outlines.
    fn impostors_marks(&self, view: MeshPipelineKey, proxy: bool) -> RenderPipelineDescriptor {
        let mut descriptor = self.impostors_singled(view, proxy, "MARK", mark_target());
        descriptor.label = Some("impostor_marks".into());
        descriptor.depth_stencil = None;
        descriptor
    }

    /// The impostors' depth pipeline with its stages built for `singling` and writing `target`,
    /// the scenery's slot and its marks bound after the instances.
    fn impostors_singled(
        &self,
        view: MeshPipelineKey,
        proxy: bool,
        singling: &'static str,
        target: ColorTargetState,
    ) -> RenderPipelineDescriptor {
        let mut descriptor = self.impostors_depth(view, proxy);
        descriptor.layout[INSTANCE_GROUP] = self.singled.clone();
        descriptor.vertex.shader_defs.push(singling.into());
        if let Some(fragment) = &mut descriptor.fragment {
            fragment.shader_defs.push(singling.into());
            fragment.targets = vec![Some(target)];
        }
        descriptor
    }

    /// `descriptor` drawing the instances its commands list: from the arenas' instances where a
    /// mesh's bindings would be, through the shader of the game's that draws the material, in
    /// its instanced stages, else through the instanced stages of the world's own material in
    /// place of bevy's `untouched` vertex stage and the material's own fragment stage.
    fn instanced(
        &self,
        mut descriptor: RenderPipelineDescriptor,
        untouched: &Handle<Shader>,
    ) -> RenderPipelineDescriptor {
        if descriptor.vertex.shader == *untouched {
            descriptor.vertex.shader = self.shader.clone();
            if let Some(fragment) = descriptor.fragment.as_mut() {
                fragment.shader = self.shader.clone();
            }
        }
        descriptor.layout[INSTANCE_GROUP] = self.instances.clone();
        descriptor
    }
}

/// Has a pipeline of a view whose depth the same geometry was drawn into shade only the pixels
/// that show: each pixel's depth is left there, so a pixel of this pass shows only where its
/// depth is that very depth, and writes none. Testing for less, the GPU would run the whole
/// lighting for every face under a pixel that passes on its way to the nearest, as a shader that
/// drops pixels of its own keeps the depth test from running before it while it writes depth.
fn shade_only_what_shows(descriptor: &mut RenderPipelineDescriptor) {
    if let Some(depth) = &mut descriptor.depth_stencil {
        depth.depth_compare = Some(CompareFunction::Equal);
        depth.depth_write_enabled = Some(false);
    }
}

/// The key the pipelines singling instances out are built for, whatever the view: depth alone,
/// on `samples` samples a pixel.
fn singled_view(samples: u32) -> MeshPipelineKey {
    MeshPipelineKey::DEPTH_PREPASS | MeshPipelineKey::from_msaa_samples(samples)
}

fn id_target() -> ColorTargetState {
    ColorTargetState {
        format: IDS_FORMAT,
        blend: None,
        write_mask: ColorWrites::ALL,
    }
}

/// Every shape marked in a set adds to the set's channel, which holds no more than whole.
fn mark_target() -> ColorTargetState {
    let adding = BlendComponent {
        src_factor: BlendFactor::One,
        dst_factor: BlendFactor::One,
        operation: BlendOperation::Add,
    };
    ColorTargetState {
        format: MARKS_FORMAT,
        blend: Some(BlendState {
            color: adding,
            alpha: adding,
        }),
        write_mask: ColorWrites::ALL,
    }
}

fn material_key(
    mesh_key: MeshPipelineKey,
    kind: TypeId,
    properties: &MaterialProperties,
) -> ErasedMaterialPipelineKey {
    ErasedMaterialPipelineKey {
        mesh_key: ErasedMeshPipelineKey::new(mesh_key),
        material_key: properties.material_key.clone(),
        type_id: kind,
    }
}

/// Draws the parts a view draws of a scenery, each on its own pipeline in its material, every
/// level of a part in one indirect multi-draw over the view's commands.
struct DrawProps<const LIT: bool>;

impl<P: PhaseItem, const LIT: bool> RenderCommand<P> for DrawProps<LIT> {
    type Param = (
        SRes<PropBindings>,
        SRes<PropCulls>,
        SRes<ModelArena>,
        SRes<PipelineCache>,
        SRes<RenderProgress>,
    );
    type ViewQuery = Read<ExtractedView>;
    type ItemQuery = Read<MainEntity>;

    fn render<'w>(
        _: &P,
        view: ROQueryItem<'w, '_, Self::ViewQuery>,
        scenery: Option<ROQueryItem<'w, '_, Self::ItemQuery>>,
        (bindings, culls, models, cache, progress): SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let (bindings, culls, models, cache, progress) = (
            bindings.into_inner(),
            culls.into_inner(),
            models.into_inner(),
            cache.into_inner(),
            progress.into_inner(),
        );
        let Some(scenery) = scenery.map(MainEntity::id) else {
            return RenderCommandResult::Skip;
        };
        let view = view.retained_view_entity;
        let drawn = if LIT { PropPass::Lit } else { PropPass::Depth };
        let (Some(parts), Some(instances), Some(culled), Some(vertices), Some(indices)) = (
            bindings.drawn.get(&(scenery, view, drawn)),
            bindings.instances.get(&(scenery, view, LIT)),
            culls.0.get(&scenery).and_then(|cull| cull.views.get(&view)),
            models.vertices.buffer(),
            models.indices.buffer(),
        ) else {
            progress.left_out();
            return RenderCommandResult::Skip;
        };
        pass.set_vertex_buffer(0, vertices.slice(..));
        pass.set_index_buffer(indices.slice(..), IndexFormat::Uint32);
        pass.set_bind_group(INSTANCE_GROUP, instances, &[]);
        for part in parts {
            let Some(drawn) = cache.get_render_pipeline(part.pipeline) else {
                progress.awaiting(cache.get_render_pipeline_state(part.pipeline));
                continue;
            };
            pass.set_render_pipeline(drawn);
            pass.set_bind_group(MATERIAL_BIND_GROUP_INDEX, &part.material, &[]);
            pass.multi_draw_indexed_indirect(
                if LIT {
                    culled.lit_commands()
                } else {
                    culled.commands()
                },
                ViewCull::command_offset(part.first_command),
                part.commands,
            );
        }
        RenderCommandResult::Success
    }
}

/// What a view that singles instances out draws them with: the ids of those it shows, for what
/// it points at, and the marks of those it outlines.
#[derive(SystemParam)]
pub(super) struct SingledProps<'w> {
    bindings: Res<'w, PropBindings>,
    pipeline: Option<Res<'w, PropPipeline>>,
    culls: Res<'w, PropCulls>,
    arenas: Res<'w, InstanceArenas>,
    models: Res<'w, ModelArena>,
    cache: Res<'w, PipelineCache>,
    view: Res<'w, PrepassViewBindGroup>,
    progress: Res<'w, RenderProgress>,
}

impl SingledProps<'_> {
    /// Draws into `pass` the id of every instance the view `view` shows, of each of `sceneries`
    /// under its place among them as its slot: the parts [`PropQueue::ids`] readied, over the
    /// depth the view was drawn with, its uniforms at `offset`.
    pub(super) fn draw_ids(
        &self,
        device: &RenderDevice,
        pass: &mut wgpu::RenderPass,
        view: RetainedViewEntity,
        offset: &ViewUniformOffset,
        sceneries: &[Entity],
    ) {
        let none = [0u32];
        for (slot, scenery) in sceneries.iter().enumerate() {
            self.draw(
                device,
                pass,
                PropPass::Ids,
                view,
                offset,
                (*scenery, slot as u32, &none),
            );
        }
    }

    /// Draws into `pass` the whole shape of every instance the view `view` outlines of each
    /// scenery: the parts [`PropQueue::marks`] readied, of the instances `marked` holds in a set,
    /// two bits an instance, the first set's lower.
    pub(super) fn draw_marks(
        &self,
        device: &RenderDevice,
        pass: &mut wgpu::RenderPass,
        view: RetainedViewEntity,
        offset: &ViewUniformOffset,
        marked: &[(Entity, Vec<u32>)],
    ) {
        for (scenery, marks) in marked {
            self.draw(
                device,
                pass,
                PropPass::Marks,
                view,
                offset,
                (*scenery, 0, marks),
            );
        }
    }

    /// Draws a scenery's parts in `singling`, with its slot and its instances' marks bound: the
    /// view's listings the prepass's depth leaves for the ids, which only what shows writes, and
    /// all of them for the marks, which what is hidden writes too.
    fn draw(
        &self,
        device: &RenderDevice,
        pass: &mut wgpu::RenderPass,
        singling: PropPass,
        view: RetainedViewEntity,
        offset: &ViewUniformOffset,
        (scenery, slot, marks): (Entity, u32, &[u32]),
    ) {
        let (Some(pipeline), Some(vertices), Some(indices), Some(seen)) = (
            self.pipeline.as_deref(),
            self.models.vertices.buffer(),
            self.models.indices.buffer(),
            self.view.no_motion_vectors.as_ref(),
        ) else {
            return;
        };
        let (Some(mesh), Some(clearing), Some(parts)) = (
            pipeline.mesh.binding(),
            pipeline.clearing.binding(),
            self.bindings.drawn.get(&(scenery, view, singling)),
        ) else {
            return;
        };
        let (Some(records), Some(cull)) = (
            self.arenas
                .0
                .get(&scenery)
                .and_then(|arena| arena.slab.buffer()),
            self.culls.0.get(&scenery),
        ) else {
            return;
        };
        let (Some(culled), Some((props, layers))) = (
            cull.views.get(&view),
            cull.table.as_ref().and_then(|table| table.prop_bindings()),
        ) else {
            return;
        };
        let (drawn, commands) = if singling == PropPass::Ids {
            (culled.lit_drawn(), culled.lit_commands())
        } else {
            (culled.drawn(), culled.commands())
        };
        let slot = device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("prop_singled_slot"),
            contents: &slot.to_le_bytes(),
            usage: BufferUsages::UNIFORM,
        });
        let marks = device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("prop_singled_marks"),
            contents: &marks
                .iter()
                .flat_map(|word| word.to_le_bytes())
                .collect::<Vec<u8>>(),
            usage: BufferUsages::STORAGE,
        });
        let instances = device.create_bind_group(
            "prop_singled_instances",
            &self.cache.get_bind_group_layout(&pipeline.singled),
            &BindGroupEntries::with_indices((
                (0, mesh),
                (100, records.as_entire_binding()),
                (101, drawn.as_entire_binding()),
                (102, clearing),
                (103, props),
                (104, layers),
                (106, slot.as_entire_binding()),
                (107, marks.as_entire_binding()),
            )),
        );
        pass.set_vertex_buffer(0, *vertices.slice(..));
        pass.set_index_buffer(*indices.slice(..), IndexFormat::Uint32);
        pass.set_bind_group(0, Some(&**seen), &[offset.offset]);
        pass.set_bind_group(1, Some(&*self.view.empty_bind_group), &[]);
        pass.set_bind_group(INSTANCE_GROUP as u32, Some(&*instances), &[]);
        for part in parts {
            let Some(pipeline) = self.cache.get_render_pipeline(part.pipeline) else {
                self.progress
                    .awaiting(self.cache.get_render_pipeline_state(part.pipeline));
                continue;
            };
            pass.set_pipeline(pipeline);
            pass.set_bind_group(MATERIAL_BIND_GROUP_INDEX as u32, Some(&*part.material), &[]);
            pass.multi_draw_indexed_indirect(
                commands,
                ViewCull::command_offset(part.first_command),
                part.commands,
            );
        }
    }
}

/// The shader definitions telling the impostor stages which shape they draw.
fn shape_defs(proxy: bool) -> Vec<ShaderDefVal> {
    if proxy {
        vec!["PROXY".into()]
    } else {
        Vec::new()
    }
}
