//! The ground of every terrain drawn from one shared grid, a patch an instance, its heights and
//! paint read from the terrain's atlas, in every view's opaque pass, prepass and shadow maps.
//! Each view draws the patches that lie in its frustum, among the terrains its layers show.

use std::ops::Range;
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::asset::uuid::Uuid;
use bevy::camera::primitives::{Aabb, Frustum};
use bevy::camera::visibility::RenderLayers;
use bevy::core_pipeline::core_3d::{
    CORE_3D_DEPTH_FORMAT, Opaque3d, Opaque3dBatchSetKey, Opaque3dBinKey,
};
use bevy::core_pipeline::prepass::{
    Opaque3dPrepass, OpaqueNoLightmap3dBatchSetKey, OpaqueNoLightmap3dBinKey,
    prepass_target_descriptors,
};
use bevy::ecs::query::{QueryItem, ROQueryItem};
use bevy::ecs::system::SystemParamItem;
use bevy::ecs::system::lifetimeless::{Read, SRes};
use bevy::math::Affine3A;
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
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_phase::{
    AddRenderCommand, BinnedPhaseItem, BinnedRenderPhase, BinnedRenderPhaseType, DrawFunctions,
    InputUniformIndex, PhaseItem, RenderCommand, RenderCommandResult, SetItemPipeline,
    TrackedRenderPass, ViewBinnedRenderPhases,
};
use bevy::render::render_resource::binding_types::{
    sampler, storage_buffer_read_only_sized, texture_2d, texture_2d_array, uniform_buffer,
};
use bevy::render::render_resource::{
    BindGroup, BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, BufferUsages,
    CachedRenderPipelineId, CompareFunction, DepthBiasState, DepthStencilState, Face,
    FragmentState, IndexFormat, MultisampleState, PipelineCache, PrimitiveState, RawBufferVec,
    RenderPipelineDescriptor, SamplerBindingType, ShaderStages, ShaderType,
    SpecializedMeshPipeline, SpecializedRenderPipeline, SpecializedRenderPipelines, StencilState,
    StorageBuffer, TextureSampleType, UniformBuffer, VertexAttribute, VertexFormat, VertexState,
    VertexStepMode,
};
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::sync_component::SyncComponent;
use bevy::render::sync_world::MainEntity;
use bevy::render::texture::GpuImage;
use bevy::render::view::{ExtractedView, RetainedViewEntity};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::{Shader, ShaderDefVal};

use super::atlas::{AtlasLayout, AtlasTexture, GpuTerrainAtlas, TerrainAtlas};
use crate::core::ground::{OVERVIEW_STRIDE, SAMPLES_PER_METER, TILE_PIXELS};
use crate::core::units::GROUND_ORIGIN;
use crate::core::vfx::shader::weather_helpers;

/// The cells along a patch's side: a patch is a grid of a point more than this each way.
pub const PATCH_CELLS: u32 = 128;

/// The ground a terrain draws: the patches standing, and what its ground is painted with.
#[derive(Component, Clone)]
#[require(TerrainAtlas)]
pub struct GroundDraw {
    pub patches: Arc<[GroundPatch]>,
    pub paint: GroundPaint,
}

/// What a terrain's ground is painted with: the terrains' pictures as one stack, white at the
/// bottom for a terrain that has none; for each terrain its colour and tile in the first row of
/// `layers`, its picture's place in the stack in the second and how it looks soaked in the third;
/// the weather, which wets it; and how rough and how reflective it lies dry.
#[derive(Clone, Debug)]
pub struct GroundPaint {
    pub pictures: Handle<Image>,
    pub layers: Handle<Image>,
    pub weather: Handle<Image>,
    pub roughness: f32,
    pub reflectance: f32,
}

/// A square of the ground laid over its height samples from `first`, a point of the shared grid
/// every `stride` samples, meeting its coarser neighbours across each edge on their own points.
#[derive(ShaderType, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GroundPatch {
    first: UVec2,
    stride: u32,
    edges: u32,
}

impl GroundPatch {
    /// The patch from sample `first`, `stride` samples between its points, beside patches whose
    /// strides are at least `beside` across its west, north, east and south edges.
    pub fn new(first: (u32, u32), stride: u32, beside: [u32; 4]) -> GroundPatch {
        let edges = beside
            .iter()
            .enumerate()
            .map(|(edge, &across)| across.max(stride).trailing_zeros() << (edge * 8))
            .fold(0, |edges, edge| edges | edge);
        GroundPatch {
            first: UVec2::new(first.0, first.1),
            stride,
            edges,
        }
    }

    /// The sample the patch is laid from.
    pub fn first(&self) -> (u32, u32) {
        (self.first.x, self.first.y)
    }

    /// The samples between the patch's points.
    pub fn stride(&self) -> u32 {
        self.stride
    }
}

/// The lit shader of the ground, composed as the world starts with the weather it reads.
pub const GROUND_LIT_SHADER: Handle<Shader> = Handle::Uuid(
    Uuid::from_u128(0x7269_6674_6772_6f75_6e64_0000_0000_0002),
    std::marker::PhantomData,
);

/// The ground's lit shader: its own text, then the weather's helpers it reads the rain by.
pub fn ground_lit_shader() -> Shader {
    Shader::from_wesl(
        format!(
            "{}\n{}",
            include_str!("terrain_lit.wesl"),
            weather_helpers()
        ),
        "engine/core/render/world/gpu/terrain_lit.wesl",
    )
}

pub struct GroundDrawPlugin;

impl Plugin for GroundDrawPlugin {
    fn build(&self, app: &mut App) {
        bevy::shader::load_shader_library!(app, "terrain.wesl");
        bevy::asset::embedded_asset!(app, "terrain_depth.wesl");
        let _ = app
            .world_mut()
            .resource_mut::<Assets<Shader>>()
            .insert(GROUND_LIT_SHADER.id(), ground_lit_shader());
        app.add_plugins(ExtractComponentPlugin::<GroundDraw>::default());
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<SpecializedRenderPipelines<GroundPipeline>>()
            .add_render_command::<Opaque3d, DrawLitGround>()
            .add_render_command::<Opaque3dPrepass, DrawGroundDepth>()
            .add_render_command::<Shadow, DrawGroundDepth>()
            .add_systems(RenderStartup, start.after(init_prepass_pipeline))
            .add_systems(
                Render,
                (
                    (prepare_grounds, bind_grounds)
                        .chain()
                        .in_set(RenderSystems::PrepareBindGroups),
                    (queue_lit, queue_prepass, queue_shadows).in_set(RenderSystems::Queue),
                ),
            );
    }
}

/// A terrain's ground as the render world draws it, while it is shown.
#[derive(Component, Clone)]
pub struct ExtractedGround {
    patches: Arc<[GroundPatch]>,
    paint: GroundPaint,
    layers: RenderLayers,
}

impl SyncComponent<RenderApp> for GroundDraw {
    type Target = ExtractedGround;
}

impl ExtractComponent<RenderApp> for GroundDraw {
    type QueryData = (
        &'static GroundDraw,
        Option<&'static RenderLayers>,
        &'static InheritedVisibility,
    );
    type QueryFilter = ();
    type Out = ExtractedGround;

    fn extract_component(
        (draw, layers, visible): QueryItem<'_, '_, Self::QueryData>,
    ) -> Option<ExtractedGround> {
        visible.get().then(|| ExtractedGround {
            patches: draw.patches.clone(),
            paint: draw.paint.clone(),
            layers: layers.cloned().unwrap_or_default(),
        })
    }
}

/// Lights the ground in the main pass, with the view's lights, shadows, ambient occlusion and fog.
type DrawLitGround = (
    SetItemPipeline,
    SetMeshViewBindGroup<0>,
    SetMeshViewBindingArrayBindGroup<1>,
    DrawGround<true>,
);

/// Writes the ground's depth, and its normal and motion where the view keeps them: for the
/// prepass of a camera and for a light's shadow map.
type DrawGroundDepth = (
    SetItemPipeline,
    SetPrepassViewBindGroup<0>,
    SetPrepassViewEmptyBindGroup<1>,
    DrawGround<false>,
);

/// The bind group of the ground's patches and atlas, after the view's two, and of its paint.
const GROUND_GROUP: usize = 2;
const PAINT_GROUP: usize = 3;
/// The points of the grid along each side of a patch.
const GRID_POINTS: u32 = PATCH_CELLS + 1;

/// The ground the atlas holds, as the shaders read it.
#[derive(ShaderType, Clone, Copy, Debug, Default, PartialEq)]
struct TerrainGround {
    origin: Vec2,
    samples: UVec2,
    tiles: UVec2,
    tile_pixels: u32,
    samples_per_metre: u32,
    overview: UVec2,
    paint_overview: UVec2,
    overview_span: u32,
    grid_cells: u32,
    level_step: f32,
    ground_level: f32,
}

impl TerrainGround {
    fn of(layout: AtlasLayout) -> TerrainGround {
        let (columns, rows) = layout.size.samples();
        TerrainGround {
            origin: Vec2::new(GROUND_ORIGIN.x, GROUND_ORIGIN.y),
            samples: UVec2::new(columns, rows),
            tiles: UVec2::new(layout.tiles.0, layout.tiles.1),
            tile_pixels: TILE_PIXELS,
            samples_per_metre: SAMPLES_PER_METER,
            overview: UVec2::new(layout.heights_overview.0, layout.heights_overview.1),
            paint_overview: UVec2::new(layout.paint_overview.0, layout.paint_overview.1),
            overview_span: OVERVIEW_STRIDE * layout.overview_step,
            grid_cells: PATCH_CELLS,
            level_step: layout.range.step(),
            ground_level: f32::from(layout.range.ground_level()),
        }
    }
}

/// The grid every patch is drawn from, from 0 to 1 across each way.
#[derive(Resource)]
struct GroundGrid {
    vertices: RawBufferVec<Vec3>,
    indices: RawBufferVec<u32>,
}

/// A terrain's patches on the GPU, and which of them each view shows.
#[derive(Component)]
struct GroundBuffers {
    drawn: Arc<[GroundPatch]>,
    bounds: Vec<Aabb>,
    patches: StorageBuffer<Vec<GroundPatch>>,
    shown: RawBufferVec<u32>,
    ground: UniformBuffer<TerrainGround>,
    /// The instances each view draws, among `shown`.
    views: HashMap<RetainedViewEntity, Range<u32>>,
}

/// A terrain's bind groups: its patches and atlas, and its paint once its pictures are on the
/// GPU.
#[derive(Component)]
struct GroundBindings {
    ground: BindGroup,
    paint: Option<BindGroup>,
}

#[derive(Resource)]
struct GroundPipeline {
    mesh: MeshPipeline,
    prepass_moving: BindGroupLayoutDescriptor,
    prepass_still: BindGroupLayoutDescriptor,
    empty: BindGroupLayoutDescriptor,
    ground: BindGroupLayoutDescriptor,
    paint: BindGroupLayoutDescriptor,
    grid: MeshVertexBufferLayoutRef,
    depth: Handle<Shader>,
    depth_clip_control: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum GroundPass {
    Lit,
    Depth,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct GroundKey {
    pass: GroundPass,
    view: MeshPipelineKey,
}

impl SpecializedRenderPipeline for GroundPipeline {
    type Key = GroundKey;

    fn specialize(&self, key: GroundKey) -> RenderPipelineDescriptor {
        match key.pass {
            GroundPass::Lit => self.lit(key.view),
            GroundPass::Depth => self.depth(key.view),
        }
    }
}

impl GroundPipeline {
    /// The mesh pipeline's own, for every shader definition the view's lighting reads, drawing
    /// the ground's shader with its patches where a mesh's would be bound, and its paint after.
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
        descriptor.label = Some("ground_lit".into());
        descriptor.layout.truncate(GROUND_GROUP);
        descriptor.layout.push(self.ground.clone());
        descriptor.layout.push(self.paint.clone());
        descriptor.vertex.shader = GROUND_LIT_SHADER;
        if let Some(fragment) = descriptor.fragment.as_mut() {
            fragment.shader = GROUND_LIT_SHADER;
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
            label: Some("ground_depth".into()),
            layout: vec![
                if moving {
                    self.prepass_moving.clone()
                } else {
                    self.prepass_still.clone()
                },
                self.empty.clone(),
                self.ground.clone(),
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
                cull_mode: Some(Face::Back),
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

struct DrawGround<const LIT: bool>;

impl<P: PhaseItem, const LIT: bool> RenderCommand<P> for DrawGround<LIT> {
    type Param = SRes<GroundGrid>;
    type ViewQuery = Read<ExtractedView>;
    type ItemQuery = (Read<GroundBuffers>, Read<GroundBindings>);

    fn render<'w>(
        _: &P,
        view: ROQueryItem<'w, '_, Self::ViewQuery>,
        ground: Option<ROQueryItem<'w, '_, Self::ItemQuery>>,
        grid: SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let grid = grid.into_inner();
        let (Some((buffers, bindings)), Some(vertices), Some(indices)) =
            (ground, grid.vertices.buffer(), grid.indices.buffer())
        else {
            return RenderCommandResult::Skip;
        };
        let Some(shown) = buffers.views.get(&view.retained_view_entity) else {
            return RenderCommandResult::Skip;
        };
        pass.set_bind_group(GROUND_GROUP, &bindings.ground, &[]);
        if LIT {
            let Some(paint) = &bindings.paint else {
                return RenderCommandResult::Skip;
            };
            pass.set_bind_group(PAINT_GROUP, paint, &[]);
        }
        pass.set_vertex_buffer(0, vertices.slice(..));
        pass.set_index_buffer(indices.slice(..), IndexFormat::Uint32);
        pass.draw_indexed(0..grid.indices.len() as u32, 0, shown.clone());
        RenderCommandResult::Success
    }
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
            let (east, south) = (at + 1, at + GRID_POINTS);
            for index in [at, south, east, east, south, south + 1] {
                indices.push(index);
            }
        }
    }
    indices.write_buffer(&device, &queue);
    let grid = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]]);
    let stages = ShaderStages::VERTEX_FRAGMENT;
    let ground = BindGroupLayoutDescriptor::new(
        "ground_patches",
        &BindGroupLayoutEntries::with_indices(
            stages,
            (
                (100, uniform_buffer::<TerrainGround>(false)),
                (101, storage_buffer_read_only_sized(false, None)),
                (102, storage_buffer_read_only_sized(false, None)),
                (103, texture_2d_array(TextureSampleType::Uint)),
                (104, texture_2d(TextureSampleType::Uint)),
                (105, texture_2d(TextureSampleType::Uint)),
                (106, texture_2d_array(TextureSampleType::Uint)),
                (107, texture_2d(TextureSampleType::Uint)),
            ),
        ),
    );
    let paint = BindGroupLayoutDescriptor::new(
        "ground_paint",
        &BindGroupLayoutEntries::with_indices(
            ShaderStages::FRAGMENT,
            (
                (
                    100,
                    texture_2d_array(TextureSampleType::Float { filterable: true }),
                ),
                (101, sampler(SamplerBindingType::Filtering)),
                (
                    102,
                    texture_2d(TextureSampleType::Float { filterable: false }),
                ),
                (
                    103,
                    texture_2d(TextureSampleType::Float { filterable: false }),
                ),
                (104, uniform_buffer::<Vec2>(false)),
            ),
        ),
    );
    commands.insert_resource(GroundPipeline {
        mesh: mesh.clone(),
        prepass_moving: prepass.view_layout_motion_vectors.clone(),
        prepass_still: prepass.view_layout_no_motion_vectors.clone(),
        empty: prepass.empty_layout.clone(),
        ground,
        paint,
        grid: grid.get_mesh_vertex_buffer_layout(&mut layouts),
        depth: bevy::asset::load_embedded_asset!(assets.as_ref(), "terrain_depth.wesl"),
        depth_clip_control: prepass.depth_clip_control_supported,
    });
    commands.insert_resource(GroundGrid { vertices, indices });
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

/// Puts each terrain's patches on the GPU as they change, and the patches each view shows: those
/// whose bounds lie in its frustum, of the terrains its layers show.
fn prepare_grounds(
    mut commands: Commands,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut grounds: Query<(
        Entity,
        &ExtractedGround,
        &GpuTerrainAtlas,
        Option<&mut GroundBuffers>,
    )>,
    views: Query<(
        &ExtractedView,
        &Frustum,
        Option<&RenderLayers>,
        Has<LightEntity>,
    )>,
) {
    for (entity, ground, atlas, buffers) in &mut grounds {
        let mut fresh = None;
        let buffers = match buffers {
            Some(buffers) => buffers.into_inner(),
            None => fresh.insert(GroundBuffers::new()),
        };
        if !Arc::ptr_eq(&buffers.drawn, &ground.patches) {
            buffers.lay(ground.patches.clone(), atlas.layout, &device, &queue);
        }
        buffers.ground.set(TerrainGround::of(atlas.layout));
        buffers.ground.write_buffer(&device, &queue);
        buffers.views.clear();
        buffers.shown.clear();
        for (view, frustum, layers, light) in &views {
            if !layers.unwrap_or_default().intersects(&ground.layers) {
                continue;
            }
            let first = buffers.shown.len() as u32;
            for (index, bounds) in buffers.bounds.iter().enumerate() {
                if frustum.intersects_obb(bounds, &Affine3A::IDENTITY, !light, light) {
                    buffers.shown.push(index as u32);
                }
            }
            let past = buffers.shown.len() as u32;
            if past > first {
                buffers.views.insert(view.retained_view_entity, first..past);
            }
        }
        if buffers.shown.is_empty() {
            buffers.shown.push(0);
        }
        buffers.shown.write_buffer(&device, &queue);
        if let Some(fresh) = fresh {
            commands.entity(entity).insert(fresh);
        }
    }
}

impl GroundBuffers {
    fn new() -> GroundBuffers {
        GroundBuffers {
            drawn: Arc::new([]),
            bounds: Vec::new(),
            patches: StorageBuffer::default(),
            shown: RawBufferVec::new(BufferUsages::STORAGE),
            ground: UniformBuffer::default(),
            views: HashMap::default(),
        }
    }

    /// Takes `patches` in place of those drawn, over a ground laid out as `layout`.
    fn lay(
        &mut self,
        patches: Arc<[GroundPatch]>,
        layout: AtlasLayout,
        device: &RenderDevice,
        queue: &RenderQueue,
    ) {
        let (columns, rows) = layout.size.samples();
        let last = UVec2::new(columns - 1, rows - 1);
        let (lowest, highest) = (layout.range.lowest.0, layout.range.highest.0);
        let per_metre = SAMPLES_PER_METER as f32;
        self.bounds = patches
            .iter()
            .map(|patch| {
                let low = patch.first.min(last);
                let high = (patch.first + UVec2::splat(PATCH_CELLS * patch.stride)).min(last);
                let at = |sample: UVec2| {
                    Vec2::new(GROUND_ORIGIN.x, GROUND_ORIGIN.y) + sample.as_vec2() / per_metre
                };
                let (low, high) = (at(low), at(high));
                Aabb::from_min_max(
                    Vec3::new(low.x, lowest, low.y),
                    Vec3::new(high.x, highest, high.y),
                )
            })
            .collect();
        let mut held = patches.to_vec();
        if held.is_empty() {
            held.push(GroundPatch::default());
        }
        self.patches.set(held);
        self.patches.write_buffer(device, queue);
        self.drawn = patches;
    }
}

/// Binds each terrain's patches, atlas and paint for its draws.
#[allow(clippy::too_many_arguments)]
fn bind_grounds(
    mut commands: Commands,
    device: Res<RenderDevice>,
    cache: Res<PipelineCache>,
    pipeline: Option<Res<GroundPipeline>>,
    images: Res<RenderAssets<GpuImage>>,
    grounds: Query<(Entity, &ExtractedGround, &GpuTerrainAtlas, &GroundBuffers)>,
    mut paints: Local<HashMap<Entity, UniformBuffer<Vec2>>>,
    queue: Res<RenderQueue>,
) {
    let Some(pipeline) = pipeline else {
        return;
    };
    paints.retain(|entity, _| grounds.contains(*entity));
    for (entity, ground, atlas, buffers) in &grounds {
        let (Some(uniform), Some(patches), Some(shown)) = (
            buffers.ground.binding(),
            buffers.patches.binding(),
            buffers.shown.buffer(),
        ) else {
            continue;
        };
        let bound = device.create_bind_group(
            "ground_patches",
            &cache.get_bind_group_layout(&pipeline.ground),
            &BindGroupEntries::with_indices((
                (100, uniform),
                (101, patches),
                (102, shown.as_entire_binding()),
                (103, atlas.view(AtlasTexture::Heights)),
                (104, atlas.view(AtlasTexture::OverviewHeights)),
                (105, atlas.view(AtlasTexture::Table)),
                (106, atlas.view(AtlasTexture::Paint)),
                (107, atlas.view(AtlasTexture::OverviewPaint)),
            )),
        );
        let paint = &ground.paint;
        let finish = paints.entry(entity).or_default();
        finish.set(Vec2::new(paint.roughness, paint.reflectance));
        finish.write_buffer(&device, &queue);
        let painted = match (
            images.get(&paint.pictures),
            images.get(&paint.weather),
            images.get(&paint.layers),
            finish.binding(),
        ) {
            (Some(pictures), Some(weather), Some(layers), Some(finish)) => {
                Some(device.create_bind_group(
                    "ground_paint",
                    &cache.get_bind_group_layout(&pipeline.paint),
                    &BindGroupEntries::with_indices((
                        (100, &pictures.texture_view),
                        (101, &pictures.sampler),
                        (102, &weather.texture_view),
                        (103, &layers.texture_view),
                        (104, finish),
                    )),
                ))
            }
            _ => None,
        };
        commands.entity(entity).insert(GroundBindings {
            ground: bound,
            paint: painted,
        });
    }
}

/// The terrains each view of a phase was queued with, by the pipeline each was queued on.
#[derive(Default)]
struct Queued(HashMap<RetainedViewEntity, HashMap<MainEntity, CachedRenderPipelineId>>);

impl Queued {
    /// Queues every one of `grounds` into `phase` on `pipeline`, taking out first what was
    /// queued on another pipeline or is gone.
    fn queue<P: BinnedPhaseItem>(
        &mut self,
        phase: &mut BinnedRenderPhase<P>,
        view: RetainedViewEntity,
        pipeline: CachedRenderPipelineId,
        grounds: &[(Entity, MainEntity)],
        keys: impl Fn() -> (P::BatchSetKey, P::BinKey),
    ) {
        let queued = self.0.entry(view).or_default();
        queued.retain(|main, was| {
            let kept = *was == pipeline && grounds.iter().any(|(_, ground)| ground == main);
            if !kept {
                phase.remove(*main);
            }
            kept
        });
        for &(entity, main) in grounds {
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

type Grounds<'w, 's> = Query<'w, 's, (Entity, &'static MainEntity, &'static GroundBuffers)>;

/// Each terrain `view` draws patches of, by its entity in the render world and in the main
/// world.
fn shown_in(grounds: &Grounds, view: &ExtractedView) -> Vec<(Entity, MainEntity)> {
    grounds
        .iter()
        .filter(|(_, _, buffers)| buffers.views.contains_key(&view.retained_view_entity))
        .map(|(entity, &main, _)| (entity, main))
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn queue_lit(
    cache: Res<PipelineCache>,
    pipeline: Option<Res<GroundPipeline>>,
    mut pipelines: ResMut<SpecializedRenderPipelines<GroundPipeline>>,
    draws: Res<DrawFunctions<Opaque3d>>,
    mut phases: ResMut<ViewBinnedRenderPhases<Opaque3d>>,
    keys: Res<ViewKeyCache>,
    views: Query<&ExtractedView>,
    grounds: Grounds,
    mut queued: Local<Queued>,
) {
    let Some(pipeline) = pipeline else {
        return;
    };
    let draw_function = draws.read().id::<DrawLitGround>();
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
            GroundKey {
                pass: GroundPass::Lit,
                view: key,
            },
        );
        let shown = shown_in(&grounds, view);
        queued.queue(phase, view.retained_view_entity, id, &shown, || {
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
    pipeline: Option<Res<GroundPipeline>>,
    mut pipelines: ResMut<SpecializedRenderPipelines<GroundPipeline>>,
    draws: Res<DrawFunctions<Opaque3dPrepass>>,
    mut phases: ResMut<ViewBinnedRenderPhases<Opaque3dPrepass>>,
    keys: Res<ViewKeyPrepassCache>,
    views: Query<&ExtractedView>,
    grounds: Grounds,
    mut queued: Local<Queued>,
) {
    let Some(pipeline) = pipeline else {
        return;
    };
    let draw_function = draws.read().id::<DrawGroundDepth>();
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
            GroundKey {
                pass: GroundPass::Depth,
                view: key,
            },
        );
        let shown = shown_in(&grounds, view);
        queued.queue(phase, view.retained_view_entity, id, &shown, || {
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
    pipeline: Option<Res<GroundPipeline>>,
    mut pipelines: ResMut<SpecializedRenderPipelines<GroundPipeline>>,
    draws: Res<DrawFunctions<Shadow>>,
    mut phases: ResMut<ViewBinnedRenderPhases<Shadow>>,
    keys: Res<LightKeyCache>,
    views: Query<&ExtractedView, With<LightEntity>>,
    grounds: Grounds,
    mut queued: Local<Queued>,
) {
    let Some(pipeline) = pipeline else {
        return;
    };
    let draw_function = draws.read().id::<DrawGroundDepth>();
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
            GroundKey {
                pass: GroundPass::Depth,
                view: key,
            },
        );
        let shown = shown_in(&grounds, view);
        queued.queue(phase, view.retained_view_entity, id, &shown, || {
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
