//! Flat squares of ground drawn from one shared grid by an indirect draw, in every view's opaque
//! pass, prepass and shadow maps. Nothing a game plays lays one yet; `rift render area --patch`
//! does, to show the draw path standing beside the world's own ground.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::RenderLayers;
use bevy::core_pipeline::core_3d::{
    CORE_3D_DEPTH_FORMAT, Opaque3d, Opaque3dBatchSetKey, Opaque3dBinKey,
};
use bevy::core_pipeline::prepass::{
    Opaque3dPrepass, OpaqueNoLightmap3dBatchSetKey, OpaqueNoLightmap3dBinKey,
    prepass_target_descriptors,
};
use bevy::ecs::query::ROQueryItem;
use bevy::ecs::system::SystemParamItem;
use bevy::ecs::system::lifetimeless::{Read, SRes};
use bevy::mesh::{
    Mesh, MeshVertexBufferLayoutRef, MeshVertexBufferLayouts, PrimitiveTopology, VertexBufferLayout,
};
use bevy::pbr::{
    LightEntity, LightKeyCache, MeshPipeline, MeshPipelineKey, PrepassPipeline,
    SetMeshViewBindGroup, SetMeshViewBindingArrayBindGroup, SetPrepassViewBindGroup,
    SetPrepassViewEmptyBindGroup, Shadow, ShadowBatchSetKey, ShadowBinKey, ViewKeyCache,
    ViewKeyPrepassCache, init_prepass_pipeline,
};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::mesh::allocator::MeshSlabs;
use bevy::render::render_phase::{
    AddRenderCommand, BinnedPhaseItem, BinnedRenderPhase, BinnedRenderPhaseType, DrawFunctions,
    InputUniformIndex, PhaseItem, RenderCommand, RenderCommandResult, SetItemPipeline,
    TrackedRenderPass, ViewBinnedRenderPhases,
};
use bevy::render::render_resource::binding_types::uniform_buffer;
use bevy::render::render_resource::{
    BindGroup, BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, Buffer,
    BufferInitDescriptor, BufferUsages, CachedRenderPipelineId, CompareFunction, DepthBiasState,
    DepthStencilState, DrawIndexedIndirectArgs, FragmentState, IndexFormat, MultisampleState,
    PipelineCache, PrimitiveState, RawBufferVec, RenderPipelineDescriptor, ShaderStages,
    ShaderType, SpecializedMeshPipeline, SpecializedRenderPipeline, SpecializedRenderPipelines,
    StencilState, UniformBuffer, VertexAttribute, VertexFormat, VertexState, VertexStepMode,
};
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::sync_world::{MainEntity, MainEntityHashMap};
use bevy::render::view::{ExtractedView, RetainedViewEntity};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::ShaderDefVal;

use crate::core::math::Pos;
use crate::core::units::Meters;

/// A square of flat ground the GPU draws: `side` metres across from its corner at the least `x`
/// and `z`, lying `height` metres up.
#[derive(Component, ExtractComponent, ShaderType, Clone, Copy, PartialEq, Debug)]
#[extract_app(RenderApp)]
pub struct TerrainPatch {
    corner: Vec2,
    side: f32,
    height: f32,
}

impl TerrainPatch {
    pub fn new(corner: Pos<Meters>, side: Meters, height: Meters) -> TerrainPatch {
        TerrainPatch {
            corner: Vec2::new(corner.x, corner.y),
            side: side.0,
            height: height.0,
        }
    }
}

pub struct GpuDrawPlugin;

impl Plugin for GpuDrawPlugin {
    fn build(&self, app: &mut App) {
        bevy::shader::load_shader_library!(app, "patch.wesl");
        bevy::asset::embedded_asset!(app, "patch_lit.wesl");
        bevy::asset::embedded_asset!(app, "patch_depth.wesl");
        app.add_plugins(ExtractComponentPlugin::<TerrainPatch>::default());
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<SpecializedRenderPipelines<PatchPipeline>>()
            .add_render_command::<Opaque3d, DrawLitPatch>()
            .add_render_command::<Opaque3dPrepass, DrawPatchDepth>()
            .add_render_command::<Shadow, DrawPatchDepth>()
            .add_systems(RenderStartup, start.after(init_prepass_pipeline))
            .add_systems(
                Render,
                (
                    (queue_lit, queue_prepass, queue_shadows).in_set(RenderSystems::Queue),
                    bind_patches.in_set(RenderSystems::PrepareBindGroups),
                ),
            );
    }
}

/// Lights a patch in the main pass, with the view's lights, shadows, ambient occlusion and fog.
type DrawLitPatch = (
    SetItemPipeline,
    SetMeshViewBindGroup<0>,
    SetMeshViewBindingArrayBindGroup<1>,
    DrawPatch,
);

/// Writes a patch's depth, and its normal and motion where the view keeps them: for the prepass
/// of a camera and for a light's shadow map.
type DrawPatchDepth = (
    SetItemPipeline,
    SetPrepassViewBindGroup<0>,
    SetPrepassViewEmptyBindGroup<1>,
    DrawPatch,
);

/// The bind group a patch's placement is bound in, after the view's two.
const PATCH_GROUP: usize = 2;
/// The binding of the placement in its group, clear of every binding of bevy's mesh group the
/// shaders' imports declare there.
const PATCH_BINDING: u32 = 100;
/// The points of the grid along each side of a patch.
const GRID_POINTS: u32 = 33;

/// The grid every patch is drawn from, from 0 to 1 across each way, and the draw that draws it
/// whole.
#[derive(Resource)]
struct PatchGrid {
    vertices: RawBufferVec<Vec3>,
    indices: RawBufferVec<u32>,
    draw: Buffer,
}

#[derive(Resource)]
struct PatchPipeline {
    mesh: MeshPipeline,
    prepass_moving: BindGroupLayoutDescriptor,
    prepass_still: BindGroupLayoutDescriptor,
    empty: BindGroupLayoutDescriptor,
    placement: BindGroupLayoutDescriptor,
    grid: MeshVertexBufferLayoutRef,
    lit: Handle<Shader>,
    depth: Handle<Shader>,
    depth_clip_control: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum PatchPass {
    Lit,
    Depth,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct PatchKey {
    pass: PatchPass,
    view: MeshPipelineKey,
}

impl SpecializedRenderPipeline for PatchPipeline {
    type Key = PatchKey;

    fn specialize(&self, key: PatchKey) -> RenderPipelineDescriptor {
        match key.pass {
            PatchPass::Lit => self.lit(key.view),
            PatchPass::Depth => self.depth(key.view),
        }
    }
}

impl PatchPipeline {
    /// The mesh pipeline's own, for every shader definition the view's lighting reads, drawing
    /// the patch's shader with its placement where a mesh's would be bound.
    fn lit(&self, view: MeshPipelineKey) -> RenderPipelineDescriptor {
        let mut descriptor = self
            .mesh
            .specialize(
                view | MeshPipelineKey::from_primitive_topology_and_strip_index(
                    PrimitiveTopology::TriangleList,
                    None,
                ),
                &self.grid,
            )
            .expect("the grid holds the positions the mesh pipeline draws from");
        descriptor.label = Some("terrain_patch_lit".into());
        descriptor.layout[PATCH_GROUP] = self.placement.clone();
        descriptor.vertex.shader = self.lit.clone();
        if let Some(fragment) = descriptor.fragment.as_mut() {
            fragment.shader = self.lit.clone();
        }
        descriptor
    }

    /// As bevy's prepass draws an opaque mesh: depth always, the normal and motion the view
    /// keeps, depth unclipped toward a directional light.
    fn depth(&self, view: MeshPipelineKey) -> RenderPipelineDescriptor {
        let normal = view.contains(MeshPipelineKey::NORMAL_PREPASS);
        let moving = view.contains(MeshPipelineKey::MOTION_VECTOR_PREPASS);
        let unclipped = view.contains(MeshPipelineKey::UNCLIPPED_DEPTH_ORTHO);
        let emulated = unclipped && !self.depth_clip_control;
        let mut defs: Vec<ShaderDefVal> = vec!["PREPASS_PIPELINE".into()];
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
        let fragment = (!targets.is_empty() || emulated).then(|| {
            defs.push("PREPASS_FRAGMENT".into());
            FragmentState {
                shader: self.depth.clone(),
                shader_defs: defs.clone(),
                targets,
                ..default()
            }
        });
        RenderPipelineDescriptor {
            label: Some("terrain_patch_depth".into()),
            layout: vec![
                if moving {
                    self.prepass_moving.clone()
                } else {
                    self.prepass_still.clone()
                },
                self.empty.clone(),
                self.placement.clone(),
            ],
            vertex: VertexState {
                shader: self.depth.clone(),
                shader_defs: defs,
                buffers: vec![grid_layout()],
                ..default()
            },
            fragment,
            primitive: PrimitiveState {
                topology: PrimitiveTopology::TriangleList,
                unclipped_depth: unclipped && self.depth_clip_control,
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
}

struct DrawPatch;

impl<P: PhaseItem> RenderCommand<P> for DrawPatch {
    type Param = SRes<PatchGrid>;
    type ViewQuery = ();
    type ItemQuery = Read<PatchBinding>;

    fn render<'w>(
        _: &P,
        _: ROQueryItem<'w, '_, Self::ViewQuery>,
        binding: Option<ROQueryItem<'w, '_, Self::ItemQuery>>,
        grid: SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let grid = grid.into_inner();
        let (Some(binding), Some(vertices), Some(indices)) =
            (binding, grid.vertices.buffer(), grid.indices.buffer())
        else {
            return RenderCommandResult::Skip;
        };
        pass.set_bind_group(PATCH_GROUP, &binding.group, &[]);
        pass.set_vertex_buffer(0, vertices.slice(..));
        pass.set_index_buffer(indices.slice(..), IndexFormat::Uint32);
        pass.draw_indexed_indirect(&grid.draw, 0);
        RenderCommandResult::Success
    }
}

/// A patch's placement as bound for its draws, and the placement it was bound for.
#[derive(Component)]
struct PatchBinding {
    patch: TerrainPatch,
    group: BindGroup,
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
    let mut vertices = RawBufferVec::new(BufferUsages::VERTEX);
    let last = (GRID_POINTS - 1) as f32;
    for row in 0..GRID_POINTS {
        for column in 0..GRID_POINTS {
            vertices.push(Vec3::new(column as f32 / last, 0.0, row as f32 / last));
        }
    }
    vertices.write_buffer(&device, &queue);
    let mut indices = RawBufferVec::new(BufferUsages::INDEX);
    for row in 0..GRID_POINTS - 1 {
        for column in 0..GRID_POINTS - 1 {
            let at = row * GRID_POINTS + column;
            let (right, below) = (at + 1, at + GRID_POINTS);
            for index in [at, below, right, right, below, below + 1] {
                indices.push(index);
            }
        }
    }
    indices.write_buffer(&device, &queue);
    let draw = device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("terrain_patch_draw"),
        contents: DrawIndexedIndirectArgs {
            index_count: indices.len() as u32,
            instance_count: 1,
            first_index: 0,
            base_vertex: 0,
            first_instance: 0,
        }
        .as_bytes(),
        usage: BufferUsages::INDIRECT,
    });
    let grid = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]]);
    commands.insert_resource(PatchPipeline {
        mesh: mesh.clone(),
        prepass_moving: prepass.view_layout_motion_vectors.clone(),
        prepass_still: prepass.view_layout_no_motion_vectors.clone(),
        empty: prepass.empty_layout.clone(),
        placement: BindGroupLayoutDescriptor::new(
            "terrain_patch_placement",
            &BindGroupLayoutEntries::with_indices(
                ShaderStages::VERTEX,
                ((PATCH_BINDING, uniform_buffer::<TerrainPatch>(false)),),
            ),
        ),
        grid: grid.get_mesh_vertex_buffer_layout(&mut layouts),
        lit: bevy::asset::load_embedded_asset!(assets.as_ref(), "patch_lit.wesl"),
        depth: bevy::asset::load_embedded_asset!(assets.as_ref(), "patch_depth.wesl"),
        depth_clip_control: prepass.depth_clip_control_supported,
    });
    commands.insert_resource(PatchGrid {
        vertices,
        indices,
        draw,
    });
}

/// The grid's vertices as the depth pipelines read them: a position each.
fn grid_layout() -> VertexBufferLayout {
    VertexBufferLayout {
        array_stride: size_of::<Vec3>() as u64,
        step_mode: VertexStepMode::Vertex,
        attributes: vec![VertexAttribute {
            format: VertexFormat::Float32x3,
            offset: 0,
            shader_location: 0,
        }],
    }
}

/// Binds each patch's placement once, and again whenever it moves.
fn bind_patches(
    mut commands: Commands,
    patches: Query<(Entity, &TerrainPatch, Option<&PatchBinding>)>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    cache: Res<PipelineCache>,
    pipeline: Option<Res<PatchPipeline>>,
) {
    let Some(pipeline) = pipeline else {
        return;
    };
    for (entity, &patch, bound) in &patches {
        if bound.is_some_and(|bound| bound.patch == patch) {
            continue;
        }
        let mut buffer = UniformBuffer::from(patch);
        buffer.write_buffer(&device, &queue);
        let Some(placement) = buffer.binding() else {
            continue;
        };
        let group = device.create_bind_group(
            "terrain_patch_placement",
            &cache.get_bind_group_layout(&pipeline.placement),
            &BindGroupEntries::with_indices(((PATCH_BINDING, placement),)),
        );
        commands
            .entity(entity)
            .insert(PatchBinding { patch, group });
    }
}

/// The patches each view of a phase was queued with, by the pipeline each was queued on.
#[derive(Default)]
struct Queued(HashMap<RetainedViewEntity, MainEntityHashMap<CachedRenderPipelineId>>);

impl Queued {
    /// Queues every one of `patches` into `phase` on `pipeline`, taking out first what was
    /// queued on another pipeline or is gone.
    fn queue<P: BinnedPhaseItem>(
        &mut self,
        phase: &mut BinnedRenderPhase<P>,
        view: RetainedViewEntity,
        pipeline: CachedRenderPipelineId,
        patches: &[(Entity, MainEntity)],
        keys: impl Fn() -> (P::BatchSetKey, P::BinKey),
    ) {
        let queued = self.0.entry(view).or_default();
        queued.retain(|main, was| {
            let kept = *was == pipeline && patches.iter().any(|(_, patch)| patch == main);
            if !kept {
                phase.remove(*main);
            }
            kept
        });
        for &(entity, main) in patches {
            queued.insert(main, pipeline);
            let (batch_set, bin) = keys();
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

type Patches<'w, 's> = Query<'w, 's, (Entity, &'static MainEntity), With<TerrainPatch>>;

/// Each patch, by its entity in the render world and in the main world.
fn patches_of(patches: &Patches) -> Vec<(Entity, MainEntity)> {
    patches
        .iter()
        .map(|(entity, &main)| (entity, main))
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn queue_lit(
    cache: Res<PipelineCache>,
    pipeline: Option<Res<PatchPipeline>>,
    mut pipelines: ResMut<SpecializedRenderPipelines<PatchPipeline>>,
    draws: Res<DrawFunctions<Opaque3d>>,
    mut phases: ResMut<ViewBinnedRenderPhases<Opaque3d>>,
    keys: Res<ViewKeyCache>,
    views: Query<&ExtractedView>,
    patches: Patches,
    mut queued: Local<Queued>,
) {
    let Some(pipeline) = pipeline else {
        return;
    };
    let draw_function = draws.read().id::<DrawLitPatch>();
    let patches = patches_of(&patches);
    for view in &views {
        let (Some(phase), Some(&key)) = (
            phases.get_mut(&view.retained_view_entity),
            keys.get(&view.retained_view_entity),
        ) else {
            continue;
        };
        let id = pipelines.specialize(
            &cache,
            &pipeline,
            PatchKey {
                pass: PatchPass::Lit,
                view: key,
            },
        );
        queued.queue(phase, view.retained_view_entity, id, &patches, || {
            (
                Opaque3dBatchSetKey {
                    pipeline: id,
                    draw_function,
                    material_bind_group_index: None,
                    lightmap_slab: None,
                    slabs: MeshSlabs::default(),
                },
                Opaque3dBinKey {
                    asset_id: AssetId::<Mesh>::default().untyped(),
                },
            )
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn queue_prepass(
    cache: Res<PipelineCache>,
    pipeline: Option<Res<PatchPipeline>>,
    mut pipelines: ResMut<SpecializedRenderPipelines<PatchPipeline>>,
    draws: Res<DrawFunctions<Opaque3dPrepass>>,
    mut phases: ResMut<ViewBinnedRenderPhases<Opaque3dPrepass>>,
    keys: Res<ViewKeyPrepassCache>,
    views: Query<&ExtractedView>,
    patches: Patches,
    mut queued: Local<Queued>,
) {
    let Some(pipeline) = pipeline else {
        return;
    };
    let draw_function = draws.read().id::<DrawPatchDepth>();
    let patches = patches_of(&patches);
    for view in &views {
        let (Some(phase), Some(&key)) = (
            phases.get_mut(&view.retained_view_entity),
            keys.get(&view.retained_view_entity),
        ) else {
            continue;
        };
        let id = pipelines.specialize(
            &cache,
            &pipeline,
            PatchKey {
                pass: PatchPass::Depth,
                view: key,
            },
        );
        queued.queue(phase, view.retained_view_entity, id, &patches, || {
            (
                OpaqueNoLightmap3dBatchSetKey {
                    pipeline: id,
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

#[allow(clippy::too_many_arguments)]
fn queue_shadows(
    cache: Res<PipelineCache>,
    pipeline: Option<Res<PatchPipeline>>,
    mut pipelines: ResMut<SpecializedRenderPipelines<PatchPipeline>>,
    draws: Res<DrawFunctions<Shadow>>,
    mut phases: ResMut<ViewBinnedRenderPhases<Shadow>>,
    keys: Res<LightKeyCache>,
    views: Query<(&ExtractedView, &RenderLayers), With<LightEntity>>,
    patches: Patches,
    mut queued: Local<Queued>,
) {
    let Some(pipeline) = pipeline else {
        return;
    };
    let draw_function = draws.read().id::<DrawPatchDepth>();
    let patches = patches_of(&patches);
    for (view, layers) in &views {
        let (Some(phase), Some(&key)) = (
            phases.get_mut(&view.retained_view_entity),
            keys.get(&view.retained_view_entity),
        ) else {
            continue;
        };
        let cast = if layers.intersects(&RenderLayers::default()) {
            patches.as_slice()
        } else {
            &[]
        };
        let id = pipelines.specialize(
            &cache,
            &pipeline,
            PatchKey {
                pass: PatchPass::Depth,
                view: key,
            },
        );
        queued.queue(phase, view.retained_view_entity, id, cast, || {
            (
                ShadowBatchSetKey {
                    pipeline: id,
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
