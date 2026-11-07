//! The props the arenas hold, drawn as the culling kernel left them: in every view's alpha-masked
//! pass, its prepass and its lights' shadow maps. Each part of a prop is drawn in its own material,
//! a world material or a cover's for the copies a cover strews, on that material's own pipeline,
//! built from bevy's material pipeline pieces as a mesh of the model slab's vertices would be, its
//! vertex stage reading the instance its command draws in place of a mesh's transform. A part
//! drawn by a shader of the game's draws through the shader's own instanced stage. A view draws
//! each part's levels in one indirect multi-draw, then the props' impostors from the atlas and
//! the boxes standing in for what the GPU lacks, each in one more. A model's impostor is baked
//! by the same pipelines, each part's shaders drawing the atlas's cells as instances.

use std::any::TypeId;
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::core_pipeline::core_3d::{AlphaMask3d, CORE_3D_DEPTH_FORMAT};
use bevy::core_pipeline::prepass::{
    AlphaMask3dPrepass, OpaqueNoLightmap3dBatchSetKey, OpaqueNoLightmap3dBinKey,
    prepass_target_descriptors,
};
use bevy::ecs::query::ROQueryItem;
use bevy::ecs::system::SystemParamItem;
use bevy::ecs::system::lifetimeless::{Read, SRes};
use bevy::material::MaterialProperties;
use bevy::material::key::{ErasedMaterialKey, ErasedMaterialPipelineKey, ErasedMeshPipelineKey};
use bevy::math::Affine3;
use bevy::mesh::{Mesh, MeshVertexBufferLayoutRef, MeshVertexBufferLayouts, PrimitiveTopology};
use bevy::pbr::{
    LightEntity, LightKeyCache, MATERIAL_BIND_GROUP_INDEX, MaterialFragmentShader,
    MaterialPipeline, MaterialVertexShader, MeshFlags, MeshPipeline, MeshPipelineKey,
    MeshTransforms, MeshUniform, MeshViewBindGroup, PreparedMaterial, PrepassFragmentShader,
    PrepassPipeline, PrepassPipelineSpecializer, PrepassVertexShader, SetMeshViewBindGroup,
    SetMeshViewBindingArrayBindGroup, SetPrepassViewBindGroup, SetPrepassViewEmptyBindGroup,
    Shadow, ShadowBatchSetKey, ShadowBinKey, ViewKeyCache, ViewKeyPrepassCache,
    alpha_mode_pipeline_key, init_prepass_pipeline, prepare_mesh_view_bind_groups,
};
use bevy::platform::collections::HashMap;
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
    BindGroup, BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, Buffer,
    BufferInitDescriptor, BufferUsages, CachedRenderPipelineId, ColorTargetState, ColorWrites,
    CompareFunction, DepthBiasState, DepthStencilState, FragmentState, IndexFormat,
    MultisampleState, PipelineCache, PrimitiveState, PrimitiveTopology as RawTopology,
    RenderPipeline, RenderPipelineDescriptor, SamplerBindingType, ShaderStages, ShaderType,
    SpecializedMeshPipeline, StencilState, StorageBuffer, TextureSampleType, UniformBuffer,
    VertexState, encase,
};
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::sync_world::MainEntity;
use bevy::render::view::{ExtractedView, RetainedViewEntity};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::{Shader, ShaderDefVal};

use super::arena::{ExtractedPropArena, InstanceArenas, InstanceRecord, LevelRange, ModelArena};
use super::clearing::Clearing;
use super::cull::{PropCulls, ViewCull};
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
                        bind_props,
                        draw_bakes
                            .after(prepare_mesh_view_bind_groups)
                            .after(prepare_material_bind_groups)
                            .after(write_material_bind_group_buffers),
                    )
                        .in_set(RenderSystems::PrepareBindGroups),
                    (
                        forget_drawn,
                        (queue_lit, queue_prepass, queue_shadows).after(forget_drawn),
                    )
                        .in_set(RenderSystems::Queue),
                ),
            );
    }
}

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

/// The passes a part's material is drawn in: lit in a view's alpha-masked pass, its depth in a
/// view's prepass or a light's shadow map, and baked into its model's impostor atlas.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum PropPass {
    Lit,
    Depth,
    Bake,
}

/// The bind groups each view's props are drawn with, by scenery and view, and the parts each
/// view draws of each scenery in each pass: on what pipeline, in what material, with which of
/// the view's commands.
#[derive(Resource, Default)]
struct PropBindings {
    instances: HashMap<(Entity, RetainedViewEntity), BindGroup>,
    drawn: HashMap<(Entity, RetainedViewEntity, bool), Vec<PartDraw>>,
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
    built: HashMap<(bool, MeshPipelineKey, bool), CachedRenderPipelineId>,
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
            bindings.instances.insert(
                (*scenery, *view),
                device.create_bind_group(
                    "prop_instances",
                    &layout,
                    &BindGroupEntries::with_indices((
                        (0, mesh.clone()),
                        (100, records.as_entire_binding()),
                        (101, culled.drawn().as_entire_binding()),
                        (102, clearing.clone()),
                        (103, props.clone()),
                        (104, layers.clone()),
                    )),
                ),
            );
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
    let jobs = std::mem::take(&mut orders.bakes);
    let mut encoder = device.create_command_encoder(&default());
    let mut baked = false;
    for job in jobs {
        let Some(parts) = bake_parts(
            &mut pipeline,
            &cache,
            view_key,
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
            job.model,
            job.key.clone(),
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
#[derive(bevy::ecs::system::SystemParam)]
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
    views:
        Query<'w, 's, (&'static ExtractedView, &'static MeshViewBindGroup), Without<LightEntity>>,
    orders: ResMut<'w, ImpostorOrders>,
    progress: Res<'w, RenderProgress>,
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
    materials: &ErasedRenderAssets<PreparedMaterial>,
    allocators: &MaterialBindGroupAllocators,
    job: &BakeJob,
) -> Option<Vec<BakePart<'a>>> {
    let mut parts = Vec::new();
    for (part, material) in job.placed.parts.iter().zip(job.materials.iter()) {
        let Some(level) = impostor::bake_level(&job.placed, job.overhang, part) else {
            continue;
        };
        let prepared = materials.get(material.id())?;
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
#[derive(bevy::ecs::system::SystemParam)]
struct PropQueue<'w, 's> {
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
    /// The parts `view` draws of each scenery in the pass `lit` names, on the pipelines for the
    /// view's key `view_key`, as each is queued: each scenery with its entity in the render world
    /// and the main world, and the pipeline of the first part it draws. A light's view draws only
    /// the parts that cast shadows.
    fn drawn(
        &mut self,
        view: RetainedViewEntity,
        view_key: MeshPipelineKey,
        lit: bool,
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
            let (Some(table), true) = (
                self.culls
                    .0
                    .get(&scenery)
                    .and_then(|cull| cull.table.as_ref()),
                self.culls
                    .0
                    .get(&scenery)
                    .is_some_and(|cull| cull.views.contains_key(&view)),
            ) else {
                if !arena.chunks.is_empty() {
                    self.progress.left_out();
                }
                continue;
            };
            let progress = &self.progress;
            let parts: Vec<PartDraw> = table
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
                        if lit { PropPass::Lit } else { PropPass::Depth },
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
            let mut parts = parts;
            if let Some(atlas) = &self.bindings.atlas {
                let table = table.table();
                for (proxy, commands) in [(false, &table.impostors), (true, &table.proxies)] {
                    if commands.is_empty() {
                        continue;
                    }
                    let Some(id) = pipeline.specialize_impostors(&self.cache, view_key, lit, proxy)
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
            self.bindings.drawn.insert((scenery, view, lit), parts);
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
    for view in &views {
        let (Some(phase), Some(&key)) = (
            phases.get_mut(&view.retained_view_entity),
            keys.get(&view.retained_view_entity),
        ) else {
            continue;
        };
        let drawn = queue.drawn(view.retained_view_entity, key, true, false);
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
    for view in &views {
        let (Some(phase), Some(&key)) = (
            phases.get_mut(&view.retained_view_entity),
            keys.get(&view.retained_view_entity),
        ) else {
            continue;
        };
        let drawn = queue.drawn(view.retained_view_entity, key, false, false);
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
    for view in &views {
        let (Some(phase), Some(&key)) = (
            phases.get_mut(&view.retained_view_entity),
            keys.get(&view.retained_view_entity),
        ) else {
            continue;
        };
        let drawn = queue.drawn(view.retained_view_entity, key, false, true);
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

    /// The pipeline drawing the impostors, or the proxies where `proxy`, in the pass `lit` names,
    /// in a view of `view`'s key, queued the first time it is asked for.
    fn specialize_impostors(
        &mut self,
        cache: &PipelineCache,
        view: MeshPipelineKey,
        lit: bool,
        proxy: bool,
    ) -> Option<CachedRenderPipelineId> {
        let key = (lit, view, proxy);
        if let Some(&id) = self.impostors.built.get(&key) {
            return Some(id);
        }
        let descriptor = if lit {
            self.impostors_lit(view, proxy)?
        } else {
            self.impostors_depth(view, proxy)
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
        let (Some(parts), Some(instances), Some(culled), Some(vertices), Some(indices)) = (
            bindings.drawn.get(&(scenery, view, LIT)),
            bindings.instances.get(&(scenery, view)),
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
                culled.commands(),
                ViewCull::command_offset(part.first_command),
                part.commands,
            );
        }
        RenderCommandResult::Success
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
