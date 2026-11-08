//! The ground of every terrain drawn from one shared grid, a patch an instance, its heights and
//! paint read from the terrain's atlas, in every view's opaque pass, prepass and shadow maps.
//! Each view draws the patches that lie in its frustum, among the terrains its layers show. Over
//! the ground, each cover's sheet is drawn from the same patches, blended in the view's
//! transparent pass with its cover's material: on those of them whose ground its terrain paints.

use std::any::TypeId;
use std::ops::Range;
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::asset::uuid::Uuid;
use bevy::camera::primitives::{Aabb, Frustum};
use bevy::camera::visibility::RenderLayers;
use bevy::core_pipeline::core_3d::{
    CORE_3D_DEPTH_FORMAT, Opaque3d, Opaque3dBatchSetKey, Opaque3dBinKey, Transparent3d,
    TransparentSortingInfo3d,
};
use bevy::core_pipeline::prepass::{
    Opaque3dPrepass, OpaqueNoLightmap3dBatchSetKey, OpaqueNoLightmap3dBinKey,
    prepass_target_descriptors,
};
use bevy::ecs::query::{QueryItem, ROQueryItem};
use bevy::ecs::system::SystemParamItem;
use bevy::ecs::system::lifetimeless::{Read, SRes};
use bevy::material::MaterialProperties;
use bevy::material::key::{ErasedMaterialKey, ErasedMaterialPipelineKey, ErasedMeshPipelineKey};
use bevy::math::Affine3A;
use bevy::mesh::{
    Mesh, MeshVertexBufferLayoutRef, MeshVertexBufferLayouts, PrimitiveTopology, VertexBufferLayout,
};
use bevy::pbr::{
    LightEntity, LightKeyCache, MATERIAL_BIND_GROUP_INDEX, MaterialFragmentShader,
    MaterialPipeline, MaterialVertexShader, MeshPipeline, MeshPipelineKey, PreparedMaterial,
    PrepassPipeline, SetMeshViewBindGroup, SetMeshViewBindingArrayBindGroup,
    SetPrepassViewBindGroup, SetPrepassViewEmptyBindGroup, Shadow, ShadowBatchSetKey, ShadowBinKey,
    ViewKeyCache, ViewKeyPrepassCache, alpha_mode_pipeline_key, init_prepass_pipeline,
};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::erased_render_asset::ErasedRenderAssets;
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::material_bind_groups::MaterialBindGroupAllocators;
use bevy::render::mesh::allocator::MeshSlabs;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_phase::{
    AddRenderCommand, BinnedPhaseItem, BinnedRenderPhase, BinnedRenderPhaseType, DrawFunctions,
    InputUniformIndex, PhaseItem, PhaseItemExtraIndex, RenderCommand, RenderCommandResult,
    SetItemPipeline, TrackedRenderPass, ViewBinnedRenderPhases, ViewSortedRenderPhases,
};
use bevy::render::render_resource::binding_types::{
    sampler, storage_buffer_read_only_sized, texture_2d, texture_2d_array, uniform_buffer,
};
use bevy::render::render_resource::{
    BindGroup, BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, BufferUsages,
    CachedRenderPipelineId, CompareFunction, DepthBiasState, DepthStencilState, Face, FilterMode,
    FragmentState, IndexFormat, MultisampleState, PipelineCache, PrimitiveState, RawBufferVec,
    RenderPipelineDescriptor, Sampler, SamplerBindingType, SamplerDescriptor, ShaderStages,
    ShaderType, SpecializedMeshPipeline, SpecializedRenderPipeline, SpecializedRenderPipelines,
    StencilState, StorageBuffer, TextureSampleType, UniformBuffer, VertexAttribute, VertexFormat,
    VertexState, VertexStepMode,
};
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::sync_component::SyncComponent;
use bevy::render::sync_world::MainEntity;
use bevy::render::texture::GpuImage;
use bevy::render::view::{ExtractedView, RetainedViewEntity};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::{Shader, ShaderDefVal};

use super::atlas::{
    AtlasLayout, AtlasTexture, CANOPY_EXTENT_MAX, GpuTerrainAtlas, SURFACE_FALL_UNIT, TerrainAtlas,
};
use super::cull;
use crate::core::ground::{
    OVERVIEW_STRIDE, PaintMap, SAMPLES_PER_METER, TILE_PIXELS, TileAt, TileVersion,
};
use crate::core::render::video::VideoPreferences;
use crate::core::render::world::RenderProgress;
use crate::core::render::world::cover::{CoverMaterial, SHORE_REACH, SheetOn};
use crate::core::render::world::lod::{GRAIN_PIXELS, horizon_band};
use crate::core::units::{GROUND_ORIGIN, Meters};
use crate::core::vfx::shader::weather_helpers;

/// The cells along a patch's side: a patch is a grid of a point more than this each way.
pub const PATCH_CELLS: u32 = 128;

/// The ground a terrain draws: the patches standing, what its ground is painted with, and the
/// sheets its covers lay over it.
#[derive(Component, Clone)]
#[require(TerrainAtlas, SheetPatches)]
pub struct GroundDraw {
    pub patches: Arc<[GroundPatch]>,
    pub paint: GroundPaint,
    pub sheets: Arc<[GroundSheet]>,
}

/// A cover's sheet over the ground, where `terrain`, the terrain's place among the area's, paints
/// it: lying on what `on` says, in the terrain's `color`, drawn with its cover's `material`.
#[derive(Clone, Debug)]
pub struct GroundSheet {
    pub terrain: u8,
    pub on: SheetOn,
    pub color: LinearRgba,
    pub material: Handle<CoverMaterial>,
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
        bevy::shader::load_shader_library!(app, "sheet.wesl");
        bevy::asset::embedded_asset!(app, "terrain_depth.wesl");
        let _ = app
            .world_mut()
            .resource_mut::<Assets<Shader>>()
            .insert(GROUND_LIT_SHADER.id(), ground_lit_shader());
        app.add_plugins(ExtractComponentPlugin::<GroundDraw>::default())
            .add_systems(PostUpdate, find_sheet_patches);
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<SpecializedRenderPipelines<GroundPipeline>>()
            .add_render_command::<Opaque3d, DrawLitGround>()
            .add_render_command::<Opaque3dPrepass, DrawGroundDepth>()
            .add_render_command::<Shadow, DrawGroundDepth>()
            .add_render_command::<Transparent3d, DrawGroundSheets>()
            .add_systems(RenderStartup, start.after(init_prepass_pipeline))
            .add_systems(
                Render,
                (
                    (prepare_grounds, bind_grounds, bind_sheets)
                        .chain()
                        .in_set(RenderSystems::PrepareBindGroups),
                    (queue_lit, queue_prepass, queue_shadows, queue_sheets)
                        .in_set(RenderSystems::Queue),
                ),
            );
    }
}

/// A terrain's ground as the render world draws it, while it is shown: with its sheets, and the
/// patches each sheet is drawn over, by their place among `patches`.
#[derive(Component, Clone)]
pub struct ExtractedGround {
    patches: Arc<[GroundPatch]>,
    paint: GroundPaint,
    layers: RenderLayers,
    sheets: Arc<[GroundSheet]>,
    sheet_patches: Arc<[Vec<u32>]>,
}

impl ExtractedGround {
    /// The render layers the terrain shows on.
    pub fn layers(&self) -> &RenderLayers {
        &self.layers
    }
}

impl SyncComponent<RenderApp> for GroundDraw {
    type Target = ExtractedGround;
}

impl ExtractComponent<RenderApp> for GroundDraw {
    type QueryData = (
        &'static GroundDraw,
        &'static SheetPatches,
        Option<&'static RenderLayers>,
        &'static InheritedVisibility,
    );
    type QueryFilter = ();
    type Out = ExtractedGround;

    fn extract_component(
        (draw, found, layers, visible): QueryItem<'_, '_, Self::QueryData>,
    ) -> Option<ExtractedGround> {
        let current =
            Arc::ptr_eq(&found.over, &draw.patches) && found.lists.len() == draw.sheets.len();
        visible.get().then(|| ExtractedGround {
            patches: draw.patches.clone(),
            paint: draw.paint.clone(),
            layers: layers.cloned().unwrap_or_default(),
            sheets: if current {
                draw.sheets.clone()
            } else {
                Arc::new([])
            },
            sheet_patches: found.lists.clone(),
        })
    }
}

/// The patches each of a terrain's sheets is drawn over, found for the patches `over`, and which
/// terrains paint each tile under them, as its paint was read at its version.
#[derive(Component)]
pub struct SheetPatches {
    over: Arc<[GroundPatch]>,
    lists: Arc<[Vec<u32>]>,
    tiles: HashMap<TileAt, (TileVersion, Terrains)>,
}

impl Default for SheetPatches {
    fn default() -> SheetPatches {
        SheetPatches {
            over: Arc::new([]),
            lists: Arc::new([]),
            tiles: HashMap::default(),
        }
    }
}

/// Which of the area's terrains paint somewhere, a bit each.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Terrains([u64; 4]);

impl Terrains {
    fn of(paint: &[u8]) -> Terrains {
        let mut terrains = Terrains::default();
        for &terrain in paint {
            terrains.0[usize::from(terrain / 64)] |= 1 << (terrain % 64);
        }
        terrains
    }

    fn with(self, other: Terrains) -> Terrains {
        Terrains(std::array::from_fn(|word| self.0[word] | other.0[word]))
    }

    fn contains(self, terrain: u8) -> bool {
        self.0[usize::from(terrain / 64)] & (1 << (terrain % 64)) != 0
    }
}

/// Finds the patches each terrain's sheets are drawn over: those over a tile its terrain paints,
/// as the tile's paint is read now: from the tile where it has loaded, else from the overview,
/// which holds no terrain the tile doesn't. Found again whenever the patches or the paint under
/// them change.
fn find_sheet_patches(mut grounds: Query<(&GroundDraw, &TerrainAtlas, &mut SheetPatches)>) {
    for (draw, atlas, mut found) in &mut grounds {
        let Some(paint) = atlas.paint() else {
            continue;
        };
        let found = found.bypass_change_detection();
        let mut changed =
            !Arc::ptr_eq(&found.over, &draw.patches) || found.lists.len() != draw.sheets.len();
        let tiles: Vec<(u32, u32, u32, u32)> = draw
            .patches
            .iter()
            .map(|patch| tiles_under(patch, paint))
            .collect();
        for &(first_column, first_row, past_column, past_row) in &tiles {
            for row in first_row..past_row {
                for column in first_column..past_column {
                    let tile = TileAt { column, row };
                    let version = paint.version(tile);
                    if found
                        .tiles
                        .get(&tile)
                        .is_some_and(|(held, _)| *held == version)
                    {
                        continue;
                    }
                    found.tiles.insert(tile, (version, painted_in(paint, tile)));
                    changed = true;
                }
            }
        }
        if !changed {
            continue;
        }
        let under: Vec<Terrains> = tiles
            .iter()
            .map(|&(first_column, first_row, past_column, past_row)| {
                (first_row..past_row)
                    .flat_map(|row| {
                        (first_column..past_column).map(move |column| TileAt { column, row })
                    })
                    .filter_map(|tile| found.tiles.get(&tile).map(|(_, terrains)| *terrains))
                    .fold(Terrains::default(), Terrains::with)
            })
            .collect();
        found.lists = draw
            .sheets
            .iter()
            .map(|sheet| {
                (0..under.len() as u32)
                    .filter(|&patch| under[patch as usize].contains(sheet.terrain))
                    .collect()
            })
            .collect();
        found.over = draw.patches.clone();
    }
}

/// The tiles of paint a sheet over `patch` reads, from the first column and row of them to the
/// ones past the last: the pixels round each of its points.
fn tiles_under(patch: &GroundPatch, paint: &PaintMap) -> (u32, u32, u32, u32) {
    let (columns, rows) = paint.pixels();
    let span = PATCH_CELLS * patch.stride;
    let tile = |pixel: u32, pixels: u32| pixel.min(pixels.saturating_sub(1)) / TILE_PIXELS;
    let (first, last) = (
        (
            patch.first.x.saturating_sub(1),
            patch.first.y.saturating_sub(1),
        ),
        (patch.first.x + span, patch.first.y + span),
    );
    (
        tile(first.0, columns),
        tile(first.1, rows),
        tile(last.0, columns) + 1,
        tile(last.1, rows) + 1,
    )
}

/// The terrains painting `tile`, as its paint is read now.
fn painted_in(paint: &PaintMap, tile: TileAt) -> Terrains {
    if paint.is_loaded(tile) {
        let (columns, rows) = paint.pixels();
        let first = (tile.column * TILE_PIXELS, tile.row * TILE_PIXELS);
        let across = TILE_PIXELS.min(columns - first.0);
        let down = TILE_PIXELS.min(rows - first.1);
        return Terrains::of(&paint.block((i64::from(first.0), i64::from(first.1)), across, down));
    }
    let ((first_column, first_row), columns, rows) = paint.overview_span(tile);
    let columns: Vec<u32> = (first_column..first_column + columns).collect();
    let rows: Vec<u32> = (first_row..first_row + rows).collect();
    Terrains::of(&paint.overview_at(&columns, &rows))
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

/// Draws each of a terrain's sheets over its patches in the main pass, lit as the view's lights,
/// shadows, ambient occlusion and fog light them, each with its cover's material.
type DrawGroundSheets = (
    SetMeshViewBindGroup<0>,
    SetMeshViewBindingArrayBindGroup<1>,
    DrawSheets,
);

/// The bind group of the ground's patches and atlas, after the view's two, and of its paint; a
/// sheet's own with the ground's in its place, and its cover's material after.
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
    /// The ground a canopy texel spans each way, in metres, and the widest a canopy's props are
    /// told to be.
    canopy_texel: f32,
    canopy_extent_max: f32,
    /// How many pixels across a prop must span for the view to draw it rather than thin it away.
    grain_pixels: f32,
    /// From where off the eye the props thin out toward the view's horizon, and the horizon: 0
    /// and 0 where they stand as far as the ground goes.
    horizon_start: f32,
    horizon: f32,
}

impl TerrainGround {
    fn of(layout: AtlasLayout, horizon: Option<Meters>) -> TerrainGround {
        let (horizon_start, horizon) = horizon_band(horizon);
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
            canopy_texel: layout.canopy_texel().0,
            canopy_extent_max: CANOPY_EXTENT_MAX.0,
            grain_pixels: GRAIN_PIXELS,
            horizon_start,
            horizon,
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
    /// The instances each view draws, among `shown`: of the ground, and of each sheet.
    views: HashMap<RetainedViewEntity, Range<u32>>,
    sheet_views: HashMap<RetainedViewEntity, Vec<Range<u32>>>,
}

/// What a sheet tells its shader of itself.
#[derive(ShaderType, Clone, Copy, Debug, Default)]
struct SheetUniform {
    color: Vec4,
    terrain: u32,
    on_water: u32,
    shore_reach: f32,
    fall_unit: f32,
}

/// A terrain's sheets as they are bound to draw, each once its cover's material is on the GPU.
#[derive(Component)]
struct SheetBindings(Vec<Option<BoundSheet>>);

/// A sheet's bind group, its cover's material's, and how its material is drawn.
struct BoundSheet {
    sheet: BindGroup,
    material: BindGroup,
    properties: Arc<MaterialProperties>,
}

/// How the sheets are drawn: bevy's mesh pipeline, as a material draws a mesh, with the sheets'
/// bindings where a mesh's would be, and the pipeline of every sheet's material for every view
/// it has been drawn in, by the material's key.
#[derive(Resource)]
struct SheetPipeline {
    material: MaterialPipeline,
    sheets: BindGroupLayoutDescriptor,
    layout: MeshVertexBufferLayoutRef,
    built: HashMap<(MeshPipelineKey, ErasedMaterialKey), CachedRenderPipelineId>,
    /// The pipeline of each sheet of each terrain each view draws, this frame.
    drawn: HashMap<(RetainedViewEntity, Entity), Vec<Option<CachedRenderPipelineId>>>,
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
    canopy_sampler: Sampler,
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
                (
                    105,
                    texture_2d_array(TextureSampleType::Float { filterable: true }),
                ),
                (
                    106,
                    texture_2d_array(TextureSampleType::Float { filterable: true }),
                ),
                (107, sampler(SamplerBindingType::Filtering)),
            ),
        ),
    );
    let sheets = BindGroupLayoutDescriptor::new(
        "ground_sheet",
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
                (108, texture_2d_array(TextureSampleType::Uint)),
                (109, texture_2d(TextureSampleType::Uint)),
                (110, uniform_buffer::<SheetUniform>(false)),
            ),
        ),
    );
    let sheet_mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]])
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32; 3]])
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32; 2]])
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_1, vec![[0.0f32; 2]])
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.0f32; 4]]);
    commands.insert_resource(SheetPipeline {
        material: MaterialPipeline {
            mesh_pipeline: mesh.clone(),
        },
        sheets,
        layout: sheet_mesh.get_mesh_vertex_buffer_layout(&mut layouts),
        built: HashMap::default(),
        drawn: HashMap::default(),
    });
    commands.insert_resource(GroundPipeline {
        mesh: mesh.clone(),
        prepass_moving: prepass.view_layout_motion_vectors.clone(),
        prepass_still: prepass.view_layout_no_motion_vectors.clone(),
        empty: prepass.empty_layout.clone(),
        ground,
        paint,
        canopy_sampler: device.create_sampler(&SamplerDescriptor {
            label: Some("ground canopy"),
            mag_filter: FilterMode::Linear,
            min_filter: FilterMode::Linear,
            ..default()
        }),
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
    video: Res<VideoPreferences>,
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
        buffers.ground.set(TerrainGround::of(
            atlas.layout,
            video.view_distance.horizon(),
        ));
        buffers.ground.write_buffer(&device, &queue);
        buffers.views.clear();
        buffers.sheet_views.clear();
        buffers.shown.clear();
        for (view, frustum, layers, light) in &views {
            if !layers.unwrap_or_default().intersects(&ground.layers) {
                continue;
            }
            let casters = light.then(|| {
                let span = atlas.layout.range.highest.0 - atlas.layout.range.lowest.0;
                let sine = cull::sun_sine(view);
                (sine > cull::SUN_RIM).then(|| {
                    let lit = cull::lit_frustum(&cull::clip_from_world_of(view), frustum);
                    cull::casting_frustum(&lit, span / sine)
                })
            });
            let (frustum, near) = match &casters {
                Some(Some(casting)) => (casting, true),
                Some(None) => (frustum, false),
                None => (frustum, true),
            };
            let seen: Vec<bool> = buffers
                .bounds
                .iter()
                .map(|bounds| frustum.intersects_obb(bounds, &Affine3A::IDENTITY, near, light))
                .collect();
            let first = buffers.shown.len() as u32;
            for (index, _) in seen.iter().enumerate().filter(|(_, seen)| **seen) {
                buffers.shown.push(index as u32);
            }
            let past = buffers.shown.len() as u32;
            if past > first {
                buffers.views.insert(view.retained_view_entity, first..past);
            }
            if light || ground.sheets.is_empty() {
                continue;
            }
            let mut sheets = Vec::with_capacity(ground.sheets.len());
            for patches in ground.sheet_patches.iter() {
                let first = buffers.shown.len() as u32;
                for &patch in patches {
                    if seen.get(patch as usize).copied().unwrap_or(false) {
                        buffers.shown.push(patch);
                    }
                }
                sheets.push(first..buffers.shown.len() as u32);
            }
            buffers
                .sheet_views
                .insert(view.retained_view_entity, sheets);
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
            sheet_views: HashMap::default(),
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
                        (105, atlas.view(AtlasTexture::OverviewCanopy)),
                        (106, atlas.view(AtlasTexture::OverviewCanopyExtent)),
                        (107, &pipeline.canopy_sampler),
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

/// Binds each terrain's sheets for their draws: the ground's patches and atlas with what each
/// sheet tells its shader of itself, and its cover's material once that is on the GPU.
#[allow(clippy::too_many_arguments)]
fn bind_sheets(
    mut commands: Commands,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    cache: Res<PipelineCache>,
    pipeline: Option<Res<SheetPipeline>>,
    materials: Res<ErasedRenderAssets<PreparedMaterial>>,
    allocators: Res<MaterialBindGroupAllocators>,
    grounds: Query<(Entity, &ExtractedGround, &GpuTerrainAtlas, &GroundBuffers)>,
    mut uniforms: Local<HashMap<Entity, Vec<UniformBuffer<SheetUniform>>>>,
) {
    let Some(pipeline) = pipeline else {
        return;
    };
    uniforms.retain(|entity, _| grounds.contains(*entity));
    let layout = cache.get_bind_group_layout(&pipeline.sheets);
    let allocator = allocators.get(&TypeId::of::<CoverMaterial>());
    for (entity, ground, atlas, buffers) in &grounds {
        let (Some(terrain), Some(patches), Some(shown)) = (
            buffers.ground.binding(),
            buffers.patches.binding(),
            buffers.shown.buffer(),
        ) else {
            continue;
        };
        let held = uniforms.entry(entity).or_default();
        held.resize_with(ground.sheets.len(), UniformBuffer::default);
        let bound = ground
            .sheets
            .iter()
            .zip(held.iter_mut())
            .map(|(sheet, uniform)| {
                uniform.set(SheetUniform {
                    color: sheet.color.to_vec4(),
                    terrain: u32::from(sheet.terrain),
                    on_water: u32::from(sheet.on == SheetOn::Water),
                    shore_reach: SHORE_REACH.0,
                    fall_unit: SURFACE_FALL_UNIT,
                });
                uniform.write_buffer(&device, &queue);
                let prepared = materials.get(sheet.material.id())?;
                let material = allocator?
                    .get(prepared.binding.group)?
                    .bind_group()?
                    .clone();
                let sheet = device.create_bind_group(
                    "ground_sheet",
                    &layout,
                    &BindGroupEntries::with_indices((
                        (100, terrain.clone()),
                        (101, patches.clone()),
                        (102, shown.as_entire_binding()),
                        (103, atlas.view(AtlasTexture::Heights)),
                        (104, atlas.view(AtlasTexture::OverviewHeights)),
                        (105, atlas.view(AtlasTexture::Table)),
                        (106, atlas.view(AtlasTexture::Paint)),
                        (107, atlas.view(AtlasTexture::OverviewPaint)),
                        (108, atlas.view(AtlasTexture::Surface)),
                        (109, atlas.view(AtlasTexture::OverviewSurface)),
                        (110, uniform.binding()?),
                    )),
                );
                Some(BoundSheet {
                    sheet,
                    material,
                    properties: prepared.properties.clone(),
                })
            })
            .collect();
        commands.entity(entity).insert(SheetBindings(bound));
    }
}

/// Queues each terrain's sheets into the transparent pass of every view that shows any of them,
/// on its material's pipeline for the view. A sheet lies under whatever blended stands on it, so
/// the sheets are drawn before everything else the pass blends.
#[allow(clippy::too_many_arguments)]
fn queue_sheets(
    cache: Res<PipelineCache>,
    pipeline: Option<ResMut<SheetPipeline>>,
    draws: Res<DrawFunctions<Transparent3d>>,
    mut phases: ResMut<ViewSortedRenderPhases<Transparent3d>>,
    keys: Res<ViewKeyCache>,
    views: Query<&ExtractedView>,
    grounds: Query<(Entity, &MainEntity, &GroundBuffers, &SheetBindings)>,
) {
    let Some(mut pipeline) = pipeline else {
        return;
    };
    let pipeline = &mut *pipeline;
    pipeline.drawn.clear();
    let draw_function = draws.read().id::<DrawGroundSheets>();
    for view in &views {
        let (Some(phase), Some(&key)) = (
            phases.get_mut(&view.retained_view_entity),
            keys.get(&view.retained_view_entity),
        ) else {
            continue;
        };
        for (entity, &main, buffers, bindings) in &grounds {
            let Some(ranges) = buffers.sheet_views.get(&view.retained_view_entity) else {
                continue;
            };
            let ids: Vec<Option<CachedRenderPipelineId>> = bindings
                .0
                .iter()
                .zip(ranges)
                .map(|(bound, range)| {
                    let bound = bound.as_ref().filter(|_| !range.is_empty())?;
                    pipeline.specialize(&cache, key, &bound.properties)
                })
                .collect();
            let Some(first) = ids.iter().flatten().next().copied() else {
                continue;
            };
            pipeline
                .drawn
                .insert((view.retained_view_entity, entity), ids);
            phase.add_transient(Transparent3d {
                sorting_info: TransparentSortingInfo3d::Sorted {
                    mesh_center: Vec3::ZERO,
                    depth_bias: f32::NEG_INFINITY,
                },
                distance: 0.0,
                pipeline: first,
                entity: (entity, main),
                draw_function,
                batch_range: 0..1,
                extra_index: PhaseItemExtraIndex::None,
                indexed: true,
            });
        }
    }
}

impl SheetPipeline {
    /// The pipeline drawing a sheet of the material `properties` describes in a view of `view`'s
    /// key, queued the first time it is asked for.
    fn specialize(
        &mut self,
        cache: &PipelineCache,
        view: MeshPipelineKey,
        properties: &MaterialProperties,
    ) -> Option<CachedRenderPipelineId> {
        let key = (view, properties.material_key.clone());
        if let Some(&id) = self.built.get(&key) {
            return Some(id);
        }
        let descriptor = self.descriptor(view, properties)?;
        let id = cache.queue_render_pipeline(descriptor);
        self.built.insert(key, id);
        Some(id)
    }

    /// The pipeline bevy would draw a mesh with positions, normals, both uvs and colours of the
    /// material in, its shader drawing a sheet instead: from the shared grid, with the sheet's
    /// bindings where the mesh's would be.
    fn descriptor(
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
        let group = ShaderDefVal::UInt(
            "MATERIAL_BIND_GROUP".into(),
            MATERIAL_BIND_GROUP_INDEX as u32,
        );
        let mut defs = vec![group, "SHEET_PATCH".into()];
        if properties.bindless {
            defs.push("BINDLESS".into());
        }
        if let Some(shader) = properties.get_shader(MaterialVertexShader) {
            descriptor.vertex.shader = shader;
        }
        let fragment = descriptor.fragment.as_mut()?;
        if let Some(shader) = properties.get_shader(MaterialFragmentShader) {
            fragment.shader = shader;
        }
        descriptor.layout.insert(
            MATERIAL_BIND_GROUP_INDEX,
            properties.material_layout.clone()?,
        );
        if let Some(specialize) = properties.user_specialize {
            let key = ErasedMaterialPipelineKey {
                mesh_key: ErasedMeshPipelineKey::new(mesh_key),
                material_key: properties.material_key.clone(),
                type_id: TypeId::of::<CoverMaterial>(),
            };
            specialize(&self.material, &mut descriptor, &self.layout, key).ok()?;
        }
        descriptor.label = Some("ground_sheet".into());
        descriptor.layout[GROUND_GROUP] = self.sheets.clone();
        descriptor.vertex.buffers = vec![grid_layout()];
        descriptor.vertex.shader_defs.extend(defs.iter().cloned());
        descriptor.fragment.as_mut()?.shader_defs.extend(defs);
        Some(descriptor)
    }
}

/// Draws a terrain's sheets over the patches the view shows each on, each on its material's
/// pipeline with its own bindings and its material's.
struct DrawSheets;

impl<P: PhaseItem> RenderCommand<P> for DrawSheets {
    type Param = (
        SRes<GroundGrid>,
        SRes<SheetPipeline>,
        SRes<PipelineCache>,
        SRes<RenderProgress>,
    );
    type ViewQuery = Read<ExtractedView>;
    type ItemQuery = (Read<GroundBuffers>, Read<SheetBindings>);

    fn render<'w>(
        item: &P,
        view: ROQueryItem<'w, '_, Self::ViewQuery>,
        sheets: Option<ROQueryItem<'w, '_, Self::ItemQuery>>,
        (grid, pipeline, cache, progress): SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let (grid, pipeline, cache, progress) = (
            grid.into_inner(),
            pipeline.into_inner(),
            cache.into_inner(),
            progress.into_inner(),
        );
        let (Some((buffers, bindings)), Some(vertices), Some(indices)) =
            (sheets, grid.vertices.buffer(), grid.indices.buffer())
        else {
            return RenderCommandResult::Skip;
        };
        let view = view.retained_view_entity;
        let (Some(ranges), Some(ids)) = (
            buffers.sheet_views.get(&view),
            pipeline.drawn.get(&(view, item.entity())),
        ) else {
            return RenderCommandResult::Skip;
        };
        pass.set_vertex_buffer(0, vertices.slice(..));
        pass.set_index_buffer(indices.slice(..), IndexFormat::Uint32);
        for ((bound, range), id) in bindings.0.iter().zip(ranges).zip(ids) {
            let (Some(bound), Some(id)) = (bound, id) else {
                continue;
            };
            let Some(drawn) = cache.get_render_pipeline(*id) else {
                progress.awaiting(cache.get_render_pipeline_state(*id));
                continue;
            };
            pass.set_render_pipeline(drawn);
            pass.set_bind_group(GROUND_GROUP, &bound.sheet, &[]);
            pass.set_bind_group(PAINT_GROUP, &bound.material, &[]);
            pass.draw_indexed(0..grid.indices.len() as u32, 0, range.clone());
        }
        RenderCommandResult::Success
    }
}
