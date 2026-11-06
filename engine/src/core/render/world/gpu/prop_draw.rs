//! The props the arenas hold, drawn as the culling kernel left them: in every view's alpha-masked
//! pass, its prepass and its lights' shadow maps. Each part of a prop is drawn in its own world
//! material on that material's own pipeline, built from bevy's material pipeline pieces as a mesh
//! of the model slab's vertices would be, its vertex stage reading the instance its command
//! draws in place of a mesh's transform. A part drawn by a shader of the game's draws through the
//! shader's own instanced stage. A view draws each part's levels in one indirect multi-draw.

use std::any::TypeId;
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::core_pipeline::core_3d::AlphaMask3d;
use bevy::core_pipeline::prepass::{
    AlphaMask3dPrepass, OpaqueNoLightmap3dBatchSetKey, OpaqueNoLightmap3dBinKey,
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
    MeshTransforms, MeshUniform, PreparedMaterial, PrepassFragmentShader, PrepassPipeline,
    PrepassPipelineSpecializer, PrepassVertexShader, SetMeshViewBindGroup,
    SetMeshViewBindingArrayBindGroup, SetPrepassViewBindGroup, SetPrepassViewEmptyBindGroup,
    Shadow, ShadowBatchSetKey, ShadowBinKey, ViewKeyCache, ViewKeyPrepassCache,
    alpha_mode_pipeline_key, init_prepass_pipeline,
};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::erased_render_asset::ErasedRenderAssets;
use bevy::render::material_bind_groups::MaterialBindGroupAllocators;
use bevy::render::mesh::allocator::MeshSlabs;
use bevy::render::render_phase::{
    AddRenderCommand, BinnedPhaseItem, BinnedRenderPhaseType, DrawFunctions, InputUniformIndex,
    PhaseItem, RenderCommand, RenderCommandResult, TrackedRenderPass, ViewBinnedRenderPhases,
};
use bevy::render::render_resource::binding_types::{
    storage_buffer_read_only_sized, uniform_buffer,
};
use bevy::render::render_resource::{
    BindGroup, BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries,
    CachedRenderPipelineId, IndexFormat, PipelineCache, RenderPipelineDescriptor, ShaderStages,
    ShaderType, SpecializedMeshPipeline, StorageBuffer, UniformBuffer,
};
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::sync_world::MainEntity;
use bevy::render::view::{ExtractedView, RetainedViewEntity};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::{Shader, ShaderDefVal};

use super::arena::{ExtractedPropArena, InstanceArenas, ModelArena};
use super::clearing::Clearing;
use super::cull::{self, PropCulls, ViewCull};
use crate::core::render::world::material::WorldMaterial;

pub struct PropDrawPlugin;

impl Plugin for PropDrawPlugin {
    fn build(&self, app: &mut App) {
        bevy::shader::load_shader_library!(app, "instance.wesl");
        bevy::asset::embedded_asset!(app, "instanced.wesl");
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
                    bind_props
                        .in_set(RenderSystems::PrepareBindGroups)
                        .after(cull::prepare_culls),
                    (queue_lit, queue_prepass, queue_shadows).in_set(RenderSystems::Queue),
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

/// The bind groups each view's props are drawn with, by scenery and view, and the parts each
/// view draws of each scenery in each pass: on what pipeline, in what material, with which of
/// the view's commands.
#[derive(Resource, Default)]
struct PropBindings {
    instances: HashMap<(Entity, RetainedViewEntity), BindGroup>,
    drawn: HashMap<(Entity, RetainedViewEntity, bool), Vec<PartDraw>>,
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
    built: HashMap<(bool, MeshPipelineKey, ErasedMaterialKey), Option<CachedRenderPipelineId>>,
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
            ),
        ),
    );
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
    });
}

/// Binds what each view draws of each scenery: the scenery's instances, and those its commands
/// draw.
fn bind_props(
    device: Res<RenderDevice>,
    cache: Res<PipelineCache>,
    pipeline: Option<Res<PropPipeline>>,
    culls: Res<PropCulls>,
    arenas: Res<InstanceArenas>,
    mut bindings: ResMut<PropBindings>,
) {
    let Some(pipeline) = pipeline else {
        return;
    };
    bindings.instances.clear();
    bindings.drawn.clear();
    let (Some(mesh), Some(clearing), layout) = (
        pipeline.mesh.binding(),
        pipeline.clearing.binding(),
        cache.get_bind_group_layout(&pipeline.instances),
    ) else {
        return;
    };
    for (scenery, cull) in &culls.0 {
        let Some(records) = arenas.0.get(scenery).and_then(|arena| arena.slab.buffer()) else {
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
                    )),
                ),
            );
        }
    }
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
        let allocator = self.allocators.get(&TypeId::of::<WorldMaterial>());
        let mut queued = Vec::new();
        for (entity, &main) in &self.sceneries {
            let scenery = main.id();
            let (Some(cull), Some(arena)) =
                (self.culls.0.get(&scenery), self.arenas.0.get(&scenery))
            else {
                continue;
            };
            let (Some(table), true) = (&cull.table, cull.views.contains_key(&view)) else {
                continue;
            };
            let parts: Vec<PartDraw> = table
                .table()
                .buckets
                .iter()
                .filter_map(|bucket| {
                    let dressed = arena.dressed.get(&bucket.prop)?;
                    if light && !dressed.shadow {
                        return None;
                    }
                    let material = dressed.materials.get(bucket.part as usize)?;
                    let prepared = self.materials.get(material.id())?;
                    if light && !prepared.properties.shadows_enabled {
                        return None;
                    }
                    let bound = allocator?
                        .get(prepared.binding.group)?
                        .bind_group()?
                        .clone();
                    let id =
                        pipeline.specialize(&self.cache, view_key, lit, &prepared.properties)?;
                    Some(PartDraw {
                        pipeline: id,
                        material: bound,
                        first_command: bucket.commands.start,
                        commands: bucket.commands.len() as u32,
                    })
                })
                .collect();
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
    /// The pipeline drawing a part of the material `properties` describes in the pass `lit`
    /// names, in a view of `view`'s key, queued the first time it is asked for.
    fn specialize(
        &mut self,
        cache: &PipelineCache,
        view: MeshPipelineKey,
        lit: bool,
        properties: &Arc<MaterialProperties>,
    ) -> Option<CachedRenderPipelineId> {
        let key = (lit, view, properties.material_key.clone());
        if let Some(&id) = self.built.get(&key) {
            return id;
        }
        let descriptor = if lit {
            self.lit(view, properties)
        } else {
            self.depth(view, properties)
        };
        let id = descriptor.map(|descriptor| cache.queue_render_pipeline(descriptor));
        self.built.insert(key, id);
        id
    }

    /// The pipeline bevy would draw a mesh of the model slab's layout in the material in, lit.
    fn lit(
        &self,
        view: MeshPipelineKey,
        properties: &MaterialProperties,
    ) -> Option<RenderPipelineDescriptor> {
        let mesh_key =
            view | MeshPipelineKey::from_primitive_topology_and_strip_index(
                PrimitiveTopology::TriangleList,
                None,
            ) | properties
                .mesh_pipeline_key_bits
                .downcast::<MeshPipelineKey>()
                | alpha_mode_pipeline_key(
                    properties.alpha_mode,
                    &Msaa::from_samples(view.msaa_samples()),
                );
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
                material_key(mesh_key, properties),
            )
            .ok()?;
        }
        descriptor.label = Some("prop_lit".into());
        Some(self.instanced(descriptor, &untouched))
    }

    /// The pipeline bevy's prepass would write a mesh of the model slab's layout in the material
    /// with: to a camera's prepass or a light's shadow map, as `view`'s key says.
    fn depth(
        &self,
        view: MeshPipelineKey,
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
            .specialize(material_key(mesh_key, properties), &self.layout)
            .ok()?;
        descriptor.label = Some("prop_depth".into());
        Some(self.instanced(descriptor, &self.prepass.default_prepass_shader))
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
        } else {
            let defs: [ShaderDefVal; 2] = ["MERGED".into(), "PROP_INSTANCE".into()];
            descriptor.vertex.shader_defs.extend(defs.iter().cloned());
            if let Some(fragment) = descriptor.fragment.as_mut() {
                fragment.shader_defs.extend(defs);
            }
        }
        descriptor.layout[INSTANCE_GROUP] = self.instances.clone();
        descriptor
    }
}

fn material_key(
    mesh_key: MeshPipelineKey,
    properties: &MaterialProperties,
) -> ErasedMaterialPipelineKey {
    ErasedMaterialPipelineKey {
        mesh_key: ErasedMeshPipelineKey::new(mesh_key),
        material_key: properties.material_key.clone(),
        type_id: TypeId::of::<WorldMaterial>(),
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
    );
    type ViewQuery = Read<ExtractedView>;
    type ItemQuery = Read<MainEntity>;

    fn render<'w>(
        _: &P,
        view: ROQueryItem<'w, '_, Self::ViewQuery>,
        scenery: Option<ROQueryItem<'w, '_, Self::ItemQuery>>,
        (bindings, culls, models, cache): SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let (bindings, culls, models, cache) = (
            bindings.into_inner(),
            culls.into_inner(),
            models.into_inner(),
            cache.into_inner(),
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
            return RenderCommandResult::Skip;
        };
        pass.set_vertex_buffer(0, vertices.slice(..));
        pass.set_index_buffer(indices.slice(..), IndexFormat::Uint32);
        pass.set_bind_group(INSTANCE_GROUP, instances, &[]);
        for part in parts {
            let Some(drawn) = cache.get_render_pipeline(part.pipeline) else {
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
