//! The blades a ground's covers grow, laid out on the GPU. Each piece of a cover that grows blades
//! grows them over the cells of the ground the views look over, and the main world says which:
//! for each such cell, its paint and its heights, and where in the piece's slab its blades go, as
//! many as its paint asks for. A kernel lays every blade out of those alone, a row of the cell's
//! paint a thread, the same blades for the same ground every time. Each view draws the blades of
//! the cells in its frustum, farthest first, blended in its transparent pass with the cover's
//! material, lit by the ground's normal, casting no shadow. The
//! farther a blade, the fewer stand: below the grain they are thinned by their seeds and grown to
//! make up for those dropped, as strewn copies are, and the player's grass setting thins them all.

use std::any::TypeId;
use std::ops::Range;
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::camera::primitives::{Aabb, Frustum};
use bevy::camera::visibility::RenderLayers;
use bevy::core_pipeline::core_3d::{Transparent3d, TransparentSortingInfo3d};
use bevy::ecs::query::ROQueryItem;
use bevy::ecs::system::SystemParamItem;
use bevy::ecs::system::lifetimeless::{Read, SRes};
use bevy::material::MaterialProperties;
use bevy::material::key::{ErasedMaterialKey, ErasedMaterialPipelineKey, ErasedMeshPipelineKey};
use bevy::math::Affine3A;
use bevy::mesh::{Mesh, MeshVertexBufferLayoutRef, MeshVertexBufferLayouts, PrimitiveTopology};
use bevy::pbr::{
    MATERIAL_BIND_GROUP_INDEX, MaterialFragmentShader, MaterialPipeline, MaterialVertexShader,
    MeshPipeline, MeshPipelineKey, PreparedMaterial, SetMeshViewBindGroup,
    SetMeshViewBindingArrayBindGroup, ViewKeyCache, alpha_mode_pipeline_key, init_prepass_pipeline,
};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::erased_render_asset::ErasedRenderAssets;
use bevy::render::material_bind_groups::MaterialBindGroupAllocators;
use bevy::render::render_phase::{
    AddRenderCommand, DrawFunctions, PhaseItem, PhaseItemExtraIndex, RenderCommand,
    RenderCommandResult, TrackedRenderPass, ViewSortedRenderPhases,
};
use bevy::render::render_resource::binding_types::{
    storage_buffer_read_only_sized, storage_buffer_sized, uniform_buffer,
};
use bevy::render::render_resource::{
    BindGroup, BindGroupEntries, BindGroupLayout, BindGroupLayoutDescriptor,
    BindGroupLayoutEntries, Buffer, BufferUsages, CachedRenderPipelineId, ComputePassDescriptor,
    ComputePipeline, PipelineCache, PipelineCompilationOptions, PipelineLayoutDescriptor,
    RawComputePipelineDescriptor, RenderPipelineDescriptor, ShaderModuleDescriptor, ShaderSource,
    ShaderStages, ShaderType, SpecializedMeshPipeline, StorageBuffer, UniformBuffer,
};
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::sync_world::MainEntity;
use bevy::render::view::{ExtractedView, RetainedViewEntity};
use bevy::render::{ExtractSchedule, MainWorld, Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::ShaderDefVal;

use super::cull::read_buffer;
use super::draw::ExtractedGround;
use super::slab::{Slab, SlabBlock, SlabSpace};
use crate::core::ground::{Heightfield, PaintMap, SAMPLES_PER_METER};
use crate::core::math::{Pos, Rect, Size};
use crate::core::render::world::RenderProgress;
use crate::core::render::world::cover::CoverMaterial;
use crate::core::render::world::lod::GRAIN_PIXELS;
use crate::core::units::{GROUND_ORIGIN, Meters};

pub struct GrassPlugin;

impl Plugin for GrassPlugin {
    fn build(&self, app: &mut App) {
        bevy::shader::load_shader_library!(app, "grass.wesl");
        if app.get_sub_app(RenderApp).is_none() {
            return;
        }
        app.init_resource::<GrassChanges>();
        app.sub_app_mut(RenderApp)
            .init_resource::<GrassArenas>()
            .add_render_command::<Transparent3d, DrawGrassBlades>()
            .add_systems(RenderStartup, start_grass.after(init_prepass_pipeline))
            .add_systems(ExtractSchedule, extract_grass)
            .add_systems(
                Render,
                (
                    grow_grass
                        .after(RenderSystems::PrepareViews)
                        .before(RenderSystems::Queue),
                    bind_grass.in_set(RenderSystems::PrepareBindGroups),
                    queue_grass.in_set(RenderSystems::Queue),
                ),
            );
    }
}

/// The paint pixels along a side of a cell of grass: a cell is the most one kernel's workgroup lays
/// out, a row of its pixels a thread.
pub const GRASS_CELL_PIXELS: u32 = 64;

/// The height samples along a side of the ground a cell of grass grows on: from one before the
/// cell's first pixel to one past its last, so a blade at its edge reads the slope beside it.
pub const GRASS_HEIGHT_SIDE: u32 = GRASS_CELL_PIXELS + 3;

/// What a cell of grass's paint holds past the ground's edge: no terrain paints there.
pub const NO_TERRAIN: u8 = u8::MAX;

/// A blade's shape at size 1: how wide it stands at its root and how tall.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BladeShape {
    pub width: f32,
    pub height: f32,
}

/// A cover's piece that grows blades, as the GPU grows it: where `terrain`, the terrain's place
/// among the area's, paints, `per_square_meter` blades a square metre, each `shape` at a size
/// between `size`'s and a colour between `color`'s, scattered by `seed`, drawn in the cover's
/// `material` as its `piece`th piece.
#[derive(Clone, Debug)]
pub struct GrassPiece {
    pub terrain: u8,
    pub piece: u32,
    pub seed: u64,
    pub per_square_meter: f32,
    pub size: (f32, f32),
    pub color: (LinearRgba, LinearRgba),
    pub shape: BladeShape,
    pub material: Handle<CoverMaterial>,
}

impl GrassPiece {
    /// How far across the piece's largest blades measure.
    pub fn extent(&self) -> Meters {
        Meters(self.shape.width.max(self.shape.height) * self.size.1)
    }

    /// How many blades a pixel of paint the piece's terrain paints grows, on average.
    pub fn per_pixel(&self) -> f32 {
        let pixel = 1.0 / SAMPLES_PER_METER as f32;
        self.per_square_meter * pixel * pixel
    }

    /// The number every blade's draws of the piece start from, mixed from its seed and which piece
    /// of which terrain's cover it is.
    pub fn salt(&self) -> u32 {
        let folded = (self.seed ^ (self.seed >> 32)) as u32;
        pcg(pcg(folded ^ self.piece) ^ u32::from(self.terrain))
    }
}

/// Where a cell of grass lies among the cells of the ground, from its first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GrassCell {
    pub column: u32,
    pub row: u32,
}

impl GrassCell {
    /// The cell's first pixel of paint.
    pub fn first_pixel(self) -> (u32, u32) {
        (
            self.column * GRASS_CELL_PIXELS,
            self.row * GRASS_CELL_PIXELS,
        )
    }

    /// The ground the cell covers.
    pub fn bounds(self) -> Rect<Meters> {
        let side = GRASS_CELL_PIXELS as f32 / SAMPLES_PER_METER as f32;
        Rect::new(
            Pos::new(
                GROUND_ORIGIN.x + self.column as f32 * side,
                GROUND_ORIGIN.y + self.row as f32 * side,
            ),
            Size::new(side, side),
        )
    }
}

/// The ground a cell of grass grows on, as its blades are laid out of it: the terrain painting
/// each of its pixels, row by row, and the ground's height in metres at each of its
/// [`GRASS_HEIGHT_SIDE`] samples each way, row by row.
#[derive(Clone, Debug, PartialEq)]
pub struct GrassGround {
    pub cell: GrassCell,
    pub paint: Vec<u8>,
    pub heights: Vec<f32>,
}

impl GrassGround {
    /// The ground `cell` grows on, of `heights` and `paint`.
    pub fn of(cell: GrassCell, heights: &Heightfield, paint: &PaintMap) -> GrassGround {
        let (columns, rows) = paint.pixels();
        let (first_column, first_row) = cell.first_pixel();
        let paint = (first_row..first_row + GRASS_CELL_PIXELS)
            .flat_map(|row| {
                (first_column..first_column + GRASS_CELL_PIXELS).map(move |column| (column, row))
            })
            .map(|(column, row)| {
                if column < columns && row < rows {
                    paint.index(column, row)
                } else {
                    NO_TERRAIN
                }
            })
            .collect();
        let sample = 1.0 / SAMPLES_PER_METER as f32;
        let heights = (0..GRASS_HEIGHT_SIDE)
            .flat_map(|row| (0..GRASS_HEIGHT_SIDE).map(move |column| (column, row)))
            .map(|(column, row)| {
                let at = |first: u32, along: u32, origin: f32| {
                    origin + (first as f32 - 1.0 + along as f32) * sample
                };
                heights
                    .height_at(Pos::new(
                        at(first_column, column, GROUND_ORIGIN.x),
                        at(first_row, row, GROUND_ORIGIN.y),
                    ))
                    .0
            })
            .collect();
        GrassGround {
            cell,
            paint,
            heights,
        }
    }

    /// Where each row of the cell's blades of `piece` starts among them, and how many there are in
    /// all, as the kernel lays them out.
    pub fn rows(&self, piece: &GrassPiece) -> (Vec<u32>, u32) {
        let salt = piece.salt();
        let (first_column, first_row) = self.cell.first_pixel();
        let mut starts = Vec::with_capacity(GRASS_CELL_PIXELS as usize);
        let mut total = 0;
        for row in 0..GRASS_CELL_PIXELS {
            starts.push(total);
            for column in 0..GRASS_CELL_PIXELS {
                let painter = self.paint[(row * GRASS_CELL_PIXELS + column) as usize];
                if painter == piece.terrain {
                    let stream = grass_stream(salt, first_column + column, first_row + row);
                    total += blades_of(stream, piece.per_pixel());
                }
            }
        }
        (starts, total)
    }
}

/// How many blades a pixel whose draws come from `stream` grows, where a pixel grows `wanted` on
/// average: its whole part, and one more as often as its fraction says.
pub fn blades_of(stream: u32, wanted: f32) -> u32 {
    wanted.floor() as u32 + u32::from(grass_random(stream, 0) < wanted.fract())
}

/// The draws of the pixel at `column` and `row` of the ground's paint, for a piece of `salt`.
pub fn grass_stream(salt: u32, column: u32, row: u32) -> u32 {
    pcg(pcg(column ^ salt) ^ row)
}

/// The `draw`th draw of `stream`, from 0 up to but not 1.
pub fn grass_random(stream: u32, draw: u32) -> f32 {
    (pcg(stream.wrapping_add(draw)) >> 8) as f32 / (1u32 << 24) as f32
}

/// How many draws a blade takes, after its pixel's first: across, down, its turn, its size, its
/// tint and its seed.
pub const BLADE_DRAWS: u32 = 6;

/// A blade as the kernel lays it out: where its root stands, its size, the ground's normal under
/// it, its turn about the up axis, how far its colour lies from its piece's first to its second,
/// how heavily its pixel is painted and its seed.
#[derive(ShaderType, Clone, Copy, Debug, Default, PartialEq)]
pub struct Blade {
    pub root: Vec3,
    pub size: f32,
    pub normal: Vec3,
    pub yaw: f32,
    pub tint: f32,
    pub weight: f32,
    pub seed: f32,
    pub unused: f32,
}

impl Blade {
    pub const BYTES: u64 = 48;
}

/// What the GPU holds of a terrain's grass, as the main world keeps it in step: where each cell's
/// blades lie in its piece's slab, none for a cell that grows none.
#[derive(Default)]
pub struct GrassArena {
    spaces: Vec<SlabSpace>,
    cells: HashMap<(u32, GrassCell), Option<SlabBlock>>,
}

impl GrassArena {
    /// Grows the blades of `pieces[piece]` over `ground`: what the render world must take in.
    pub fn grow(
        &mut self,
        pieces: &[GrassPiece],
        piece: u32,
        ground: Arc<GrassGround>,
    ) -> Option<GrassChange> {
        let held = pieces.get(piece as usize)?;
        self.spaces
            .resize_with(pieces.len(), || SlabSpace::new(Blade::BYTES));
        let key = (piece, ground.cell);
        if let Some(Some(block)) = self.cells.remove(&key) {
            self.spaces[piece as usize].free(block);
        }
        let (rows, blades) = ground.rows(held);
        if blades == 0 {
            self.cells.insert(key, None);
            return None;
        }
        let space = &mut self.spaces[piece as usize];
        let block = space.alloc(u64::from(blades) * Blade::BYTES);
        let first = (space.range(block)?.start / Blade::BYTES) as u32;
        self.cells.insert(key, Some(block));
        Some(GrassChange::Grown {
            piece,
            first,
            blades,
            rows: rows.into(),
            capacity: space.capacity(),
            bounds: blade_bounds(&ground, held),
            ground,
        })
    }

    /// Lets go of the blades of `piece` over `cell`.
    pub fn cut(&mut self, piece: u32, cell: GrassCell) -> Option<GrassChange> {
        let block = self.cells.remove(&(piece, cell))?;
        if let Some(block) = block {
            self.spaces[piece as usize].free(block);
        }
        Some(GrassChange::Cut { piece, cell })
    }

    /// Whether the blades of `piece` over `cell` are grown.
    pub fn holds(&self, piece: u32, cell: GrassCell) -> bool {
        self.cells.contains_key(&(piece, cell))
    }

    /// Every piece's cells grown.
    pub fn grown(&self) -> impl Iterator<Item = (u32, GrassCell)> + '_ {
        self.cells.keys().copied()
    }

    /// How many blades the arena holds.
    pub fn blades(&self) -> u64 {
        self.cells
            .iter()
            .filter_map(|((piece, _), block)| {
                let range = self.spaces[*piece as usize].range((*block)?)?;
                Some((range.end - range.start) / Blade::BYTES)
            })
            .sum()
    }
}

/// A change to a terrain's grass the render world takes in: the pieces it grows, blades grown
/// over a cell, a cell's let go, how thickly the player has them stand, or all of it gone.
#[derive(Clone)]
pub enum GrassChange {
    Pieces(Arc<[GrassPiece]>),
    Grown {
        piece: u32,
        ground: Arc<GrassGround>,
        rows: Arc<[u32]>,
        first: u32,
        blades: u32,
        capacity: u64,
        bounds: Aabb,
    },
    Cut {
        piece: u32,
        cell: GrassCell,
    },
    Thinned(f32),
    Dropped,
}

/// The changes the terrains made to their grass since the render world last took them, each by
/// the terrain it belongs to, in order. Held only where a render world takes them.
#[derive(Resource, Default)]
pub struct GrassChanges(pub Vec<(Entity, GrassChange)>);

/// The box every blade of `piece` over `ground` stands in, however it is grown or turned.
fn blade_bounds(ground: &GrassGround, piece: &GrassPiece) -> Aabb {
    let bounds = ground.cell.bounds();
    let (low, high) = ground
        .heights
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(low, high), &at| {
            (low.min(at), high.max(at))
        });
    let reach = piece.extent().0 * GRAIN_GROWTH;
    Aabb::from_min_max(
        Vec3::new(bounds.min().x - reach, low - reach, bounds.min().y - reach),
        Vec3::new(bounds.max().x + reach, high + reach, bounds.max().y + reach),
    )
}

/// The most a blade is grown to make up for those its distance thins: past it, a blade is too
/// far to see.
const GRAIN_GROWTH: f32 = 8.0;

fn pcg(value: u32) -> u32 {
    let state = value.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    let word = ((state >> ((state >> 28) + 4)) ^ state).wrapping_mul(277_803_737);
    (word >> 22) ^ word
}

/// Each terrain's grass as the GPU holds it, by the terrain.
#[derive(Resource, Default)]
pub struct GrassArenas(pub HashMap<Entity, GpuGrass>);

/// A terrain's grass on the GPU: its pieces, each one's slab of blades, the cells whose blades the
/// slabs hold, the cells waiting for the kernel, and how thickly blades stand.
pub struct GpuGrass {
    pieces: Arc<[GrassPiece]>,
    slabs: Vec<Slab>,
    cells: HashMap<(u32, GrassCell), GpuGrassCell>,
    growing: Vec<Growing>,
    density: f32,
}

impl Default for GpuGrass {
    fn default() -> GpuGrass {
        GpuGrass {
            pieces: Arc::new([]),
            slabs: Vec::new(),
            cells: HashMap::default(),
            growing: Vec::new(),
            density: 1.0,
        }
    }
}

/// Where a cell's blades lie in its piece's slab, and the box they stand in.
struct GpuGrassCell {
    blades: Range<u32>,
    bounds: Aabb,
}

/// A cell whose blades the kernel is yet to lay out: its piece, its ground, where its rows start
/// among its blades and where they go in the slab, which must hold `capacity` bytes.
struct Growing {
    piece: u32,
    ground: Arc<GrassGround>,
    rows: Arc<[u32]>,
    first: u32,
    capacity: u64,
}

/// Takes the changes the terrains made since the last frame was extracted, whenever they were
/// made.
fn extract_grass(mut main: ResMut<MainWorld>, mut arenas: ResMut<GrassArenas>) {
    let changes = std::mem::take(
        &mut main
            .resource_mut::<GrassChanges>()
            .bypass_change_detection()
            .0,
    );
    for (terrain, change) in changes {
        if let GrassChange::Dropped = change {
            arenas.0.remove(&terrain);
            continue;
        }
        let grass = arenas.0.entry(terrain).or_default();
        match change {
            GrassChange::Pieces(pieces) => {
                grass.slabs = (0..pieces.len())
                    .map(|_| Slab::new("grass blades", BufferUsages::STORAGE))
                    .collect();
                grass.pieces = pieces;
                grass.cells.clear();
                grass.growing.clear();
            }
            GrassChange::Grown {
                piece,
                ground,
                rows,
                first,
                blades,
                capacity,
                bounds,
            } => {
                grass.cells.insert(
                    (piece, ground.cell),
                    GpuGrassCell {
                        blades: first..first + blades,
                        bounds,
                    },
                );
                grass.growing.push(Growing {
                    piece,
                    ground,
                    rows,
                    first,
                    capacity,
                });
            }
            GrassChange::Cut { piece, cell } => {
                grass.cells.remove(&(piece, cell));
            }
            GrassChange::Thinned(density) => grass.density = density,
            GrassChange::Dropped => {}
        }
    }
}

/// The kernel laying out blades, and the pipelines drawing them in every cover's material.
#[derive(Resource)]
struct GrassPipeline {
    kernel: GrassKernel,
    material: MaterialPipeline,
    blades: BindGroupLayoutDescriptor,
    layout: MeshVertexBufferLayoutRef,
    built: HashMap<(MeshPipelineKey, ErasedMaterialKey), CachedRenderPipelineId>,
    /// What each view draws of each terrain's grass this frame, farthest first.
    drawn: HashMap<(RetainedViewEntity, Entity), Vec<BladeDraw>>,
}

/// A cell's blades as a view draws them: on its material's pipeline, with its piece's bindings.
struct BladeDraw {
    pipeline: CachedRenderPipelineId,
    piece: u32,
    blades: Range<u32>,
}

fn start_grass(
    mut commands: Commands,
    device: Res<RenderDevice>,
    mesh: Res<MeshPipeline>,
    mut layouts: ResMut<MeshVertexBufferLayouts>,
) {
    let blades = BindGroupLayoutDescriptor::new(
        "grass_blades",
        &BindGroupLayoutEntries::with_indices(
            ShaderStages::VERTEX_FRAGMENT,
            (
                (100, storage_buffer_read_only_sized(false, None)),
                (101, uniform_buffer::<BladeLook>(false)),
            ),
        ),
    );
    let blade_mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]])
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32; 3]])
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32; 2]])
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_1, vec![[0.0f32; 2]])
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.0f32; 4]]);
    commands.insert_resource(GrassPipeline {
        kernel: GrassKernel::new(&device),
        material: MaterialPipeline {
            mesh_pipeline: mesh.clone(),
        },
        blades,
        layout: blade_mesh.get_mesh_vertex_buffer_layout(&mut layouts),
        built: HashMap::default(),
        drawn: HashMap::default(),
    });
}

/// The kernel laying out a cell's blades of a piece from its ground, a workgroup a cell.
pub struct GrassKernel {
    pipeline: ComputePipeline,
    layout: BindGroupLayout,
}

/// A piece as the kernel reads it.
#[derive(ShaderType, Clone, Copy, Debug, Default)]
struct KernelPiece {
    origin: Vec2,
    per_pixel: f32,
    size_low: f32,
    size_high: f32,
    salt: u32,
    terrain: u32,
    samples_per_metre: u32,
}

/// A cell as the kernel reads it: its first pixel and where its blades go in the slab.
#[derive(ShaderType, Clone, Copy, Debug, Default)]
struct KernelCell {
    first: UVec2,
    blade: u32,
    unused: u32,
}

impl GrassKernel {
    pub fn new(device: &RenderDevice) -> GrassKernel {
        let layout = device.create_bind_group_layout(
            "grass_kernel",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::COMPUTE,
                (
                    uniform_buffer::<KernelPiece>(false),
                    storage_buffer_read_only_sized(false, None),
                    storage_buffer_read_only_sized(false, None),
                    storage_buffer_read_only_sized(false, None),
                    storage_buffer_read_only_sized(false, None),
                    storage_buffer_sized(false, None),
                ),
            ),
        );
        let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("grass_kernel"),
            source: ShaderSource::Wgsl(include_str!("grass.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("grass_kernel"),
            bind_group_layouts: &[Some(&layout)],
            ..default()
        });
        let pipeline = device.create_compute_pipeline(&RawComputePipelineDescriptor {
            label: Some("grass_kernel"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("grow"),
            compilation_options: PipelineCompilationOptions::default(),
            cache: None,
        });
        GrassKernel { pipeline, layout }
    }

    /// Lays out the blades of `piece` over each of `cells`' grounds into `blades`, each cell's
    /// rows starting where its own say among its blades, from the first it names.
    pub fn grow(
        &self,
        device: &RenderDevice,
        queue: &RenderQueue,
        piece: &GrassPiece,
        cells: &[(&GrassGround, &[u32], u32)],
        blades: &Buffer,
    ) {
        if cells.is_empty() {
            return;
        }
        let mut uniform = UniformBuffer::from(KernelPiece {
            origin: Vec2::new(GROUND_ORIGIN.x, GROUND_ORIGIN.y),
            per_pixel: piece.per_pixel(),
            size_low: piece.size.0,
            size_high: piece.size.1,
            salt: piece.salt(),
            terrain: u32::from(piece.terrain),
            samples_per_metre: SAMPLES_PER_METER,
        });
        uniform.write_buffer(device, queue);
        let mut heads = StorageBuffer::from(
            cells
                .iter()
                .map(|(ground, _, first)| {
                    let (column, row) = ground.cell.first_pixel();
                    KernelCell {
                        first: UVec2::new(column, row),
                        blade: *first,
                        unused: 0,
                    }
                })
                .collect::<Vec<_>>(),
        );
        let mut rows = StorageBuffer::from(
            cells
                .iter()
                .flat_map(|(_, rows, _)| rows.iter().copied())
                .collect::<Vec<u32>>(),
        );
        let mut paint = StorageBuffer::from(
            cells
                .iter()
                .flat_map(|(ground, _, _)| {
                    ground
                        .paint
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .map(|four| u32::from_le_bytes(*four))
                })
                .collect::<Vec<u32>>(),
        );
        let mut heights = StorageBuffer::from(
            cells
                .iter()
                .flat_map(|(ground, _, _)| ground.heights.iter().copied())
                .collect::<Vec<f32>>(),
        );
        heads.write_buffer(device, queue);
        rows.write_buffer(device, queue);
        paint.write_buffer(device, queue);
        heights.write_buffer(device, queue);
        let (Some(heads), Some(rows), Some(paint), Some(heights)) = (
            heads.binding(),
            rows.binding(),
            paint.binding(),
            heights.binding(),
        ) else {
            return;
        };
        let Some(piece_binding) = uniform.binding() else {
            return;
        };
        let group = device.create_bind_group(
            "grass_kernel",
            &self.layout,
            &BindGroupEntries::sequential((
                piece_binding,
                heads,
                rows,
                paint,
                heights,
                blades.as_entire_binding(),
            )),
        );
        let mut encoder = device.create_command_encoder(&default());
        {
            let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                label: Some("grass_kernel"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.dispatch_workgroups(cells.len() as u32, 1, 1);
        }
        queue.submit([encoder.finish()]);
    }

    /// The blades `blades` holds, as the kernel laid them out.
    pub fn read(device: &RenderDevice, queue: &RenderQueue, blades: &Buffer) -> Vec<Blade> {
        read_buffer(device, queue, blades)
            .as_chunks::<{ Blade::BYTES as usize }>()
            .0
            .iter()
            .map(|bytes| {
                let word = |at: usize| {
                    f32::from_le_bytes([
                        bytes[at * 4],
                        bytes[at * 4 + 1],
                        bytes[at * 4 + 2],
                        bytes[at * 4 + 3],
                    ])
                };
                Blade {
                    root: Vec3::new(word(0), word(1), word(2)),
                    size: word(3),
                    normal: Vec3::new(word(4), word(5), word(6)),
                    yaw: word(7),
                    tint: word(8),
                    weight: word(9),
                    seed: word(10),
                    unused: word(11),
                }
            })
            .collect()
    }
}

/// Lays out the blades of every cell grown since the last frame, before anything draws them.
fn grow_grass(
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    pipeline: Option<Res<GrassPipeline>>,
    mut arenas: ResMut<GrassArenas>,
) {
    let Some(pipeline) = pipeline else {
        return;
    };
    for grass in arenas.0.values_mut() {
        if grass.growing.is_empty() {
            continue;
        }
        let growing = std::mem::take(&mut grass.growing);
        for (index, piece) in grass.pieces.iter().enumerate() {
            let index = index as u32;
            let cells: Vec<&Growing> = growing
                .iter()
                .filter(|growing| growing.piece == index)
                .filter(|growing| {
                    grass
                        .cells
                        .get(&(index, growing.ground.cell))
                        .is_some_and(|cell| cell.blades.start == growing.first)
                })
                .collect();
            let Some(capacity) = cells.iter().map(|growing| growing.capacity).max() else {
                continue;
            };
            let slab = &mut grass.slabs[index as usize];
            slab.fit(&device, &queue, capacity);
            let Some(blades) = slab.buffer() else {
                continue;
            };
            let laid: Vec<(&GrassGround, &[u32], u32)> = cells
                .iter()
                .map(|growing| (&*growing.ground, &*growing.rows, growing.first))
                .collect();
            pipeline.kernel.grow(&device, &queue, piece, &laid, blades);
        }
    }
}

/// How a piece's blades look and thin, as their shader reads it.
#[derive(ShaderType, Clone, Copy, Debug, Default)]
struct BladeLook {
    color_from: Vec4,
    color_to: Vec4,
    width: f32,
    height: f32,
    piece: u32,
    density: f32,
    grain_pixels: f32,
    grain_growth: f32,
}

/// A terrain's pieces as they are bound to draw, each once its slab and its cover's material are
/// on the GPU.
#[derive(Component)]
struct GrassBindings(Vec<Option<BoundBlades>>);

/// A piece's bind group, its cover's material's, and how its material is drawn.
struct BoundBlades {
    blades: BindGroup,
    material: BindGroup,
    properties: Arc<MaterialProperties>,
}

#[allow(clippy::too_many_arguments)]
fn bind_grass(
    mut commands: Commands,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    cache: Res<PipelineCache>,
    pipeline: Option<Res<GrassPipeline>>,
    materials: Res<ErasedRenderAssets<PreparedMaterial>>,
    allocators: Res<MaterialBindGroupAllocators>,
    arenas: Res<GrassArenas>,
    grounds: Query<(Entity, &MainEntity), With<ExtractedGround>>,
    mut looks: Local<HashMap<Entity, Vec<UniformBuffer<BladeLook>>>>,
) {
    let Some(pipeline) = pipeline else {
        return;
    };
    looks.retain(|entity, _| grounds.contains(*entity));
    let layout = cache.get_bind_group_layout(&pipeline.blades);
    let allocator = allocators.get(&TypeId::of::<CoverMaterial>());
    for (entity, main) in &grounds {
        let Some(grass) = arenas.0.get(&main.id()) else {
            continue;
        };
        let held = looks.entry(entity).or_default();
        held.resize_with(grass.pieces.len(), UniformBuffer::default);
        let bound = grass
            .pieces
            .iter()
            .zip(&grass.slabs)
            .zip(held.iter_mut())
            .map(|((piece, slab), look)| {
                look.set(BladeLook {
                    color_from: piece.color.0.to_vec4(),
                    color_to: piece.color.1.to_vec4(),
                    width: piece.shape.width,
                    height: piece.shape.height,
                    piece: piece.piece,
                    density: grass.density,
                    grain_pixels: GRAIN_PIXELS,
                    grain_growth: GRAIN_GROWTH,
                });
                look.write_buffer(&device, &queue);
                let prepared = materials.get(piece.material.id())?;
                let material = allocator?
                    .get(prepared.binding.group)?
                    .bind_group()?
                    .clone();
                let blades = device.create_bind_group(
                    "grass_blades",
                    &layout,
                    &BindGroupEntries::with_indices((
                        (100, slab.buffer()?.as_entire_binding()),
                        (101, look.binding()?),
                    )),
                );
                Some(BoundBlades {
                    blades,
                    material,
                    properties: prepared.properties.clone(),
                })
            })
            .collect();
        commands.entity(entity).insert(GrassBindings(bound));
    }
}

/// Queues each terrain's blades into the transparent pass of every camera view its layers show,
/// a cell an item, so they sort with whatever else the pass blends: of each piece bound to draw,
/// the cells whose blades' box reaches into the view's frustum.
#[allow(clippy::too_many_arguments)]
fn queue_grass(
    cache: Res<PipelineCache>,
    pipeline: Option<ResMut<GrassPipeline>>,
    draws: Res<DrawFunctions<Transparent3d>>,
    mut phases: ResMut<ViewSortedRenderPhases<Transparent3d>>,
    keys: Res<ViewKeyCache>,
    arenas: Res<GrassArenas>,
    progress: Res<RenderProgress>,
    views: Query<(&ExtractedView, &Frustum, Option<&RenderLayers>)>,
    grounds: Query<(
        Entity,
        &MainEntity,
        &ExtractedGround,
        Option<&GrassBindings>,
    )>,
) {
    let Some(mut pipeline) = pipeline else {
        return;
    };
    let pipeline = &mut *pipeline;
    pipeline.drawn.clear();
    let draw_function = draws.read().id::<DrawGrassBlades>();
    for (view, frustum, layers) in &views {
        let (Some(phase), Some(&key)) = (
            phases.get_mut(&view.retained_view_entity),
            keys.get(&view.retained_view_entity),
        ) else {
            continue;
        };
        let eye = view.world_from_view.translation();
        for (entity, &main, ground, bindings) in &grounds {
            if !layers.unwrap_or_default().intersects(ground.layers()) {
                continue;
            }
            let Some(grass) = arenas.0.get(&main.id()) else {
                continue;
            };
            let Some(bindings) = bindings else {
                if grass.cells.values().any(|cell| !cell.blades.is_empty()) {
                    progress.left_out();
                }
                continue;
            };
            let mut drawn: Vec<(f32, Vec3, BladeDraw)> = grass
                .cells
                .iter()
                .filter(|(_, cell)| {
                    !cell.blades.is_empty()
                        && frustum.intersects_obb(&cell.bounds, &Affine3A::IDENTITY, true, true)
                })
                .filter_map(|(&(piece, _), cell)| {
                    let Some(bound) = bindings.0.get(piece as usize).and_then(Option::as_ref)
                    else {
                        progress.left_out();
                        return None;
                    };
                    let pipeline_id = pipeline.specialize(&cache, key, &bound.properties)?;
                    let center = Vec3::from(cell.bounds.center);
                    Some((
                        center.distance_squared(eye),
                        center,
                        BladeDraw {
                            pipeline: pipeline_id,
                            piece,
                            blades: cell.blades.clone(),
                        },
                    ))
                })
                .collect();
            drawn.sort_by(|a, b| b.0.total_cmp(&a.0));
            for (index, (_, center, draw)) in drawn.iter().enumerate() {
                phase.add_transient(Transparent3d {
                    sorting_info: TransparentSortingInfo3d::Sorted {
                        mesh_center: *center,
                        depth_bias: 0.0,
                    },
                    distance: 0.0,
                    pipeline: draw.pipeline,
                    entity: (entity, main),
                    draw_function,
                    batch_range: index as u32..index as u32 + 1,
                    extra_index: PhaseItemExtraIndex::None,
                    indexed: false,
                });
            }
            pipeline.drawn.insert(
                (view.retained_view_entity, entity),
                drawn.into_iter().map(|(_, _, draw)| draw).collect(),
            );
        }
    }
}

impl GrassPipeline {
    /// The pipeline drawing blades in the material `properties` describes in a view of `view`'s
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
    /// material in, its shader drawing blades instead: three vertices a blade, read from the
    /// piece's slab, with the piece's bindings where the mesh's would be.
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
        let mut defs = vec![group, "GRASS_BLADE".into()];
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
        descriptor.label = Some("grass_blades".into());
        descriptor.layout[BLADES_GROUP] = self.blades.clone();
        descriptor.vertex.buffers = Vec::new();
        descriptor.vertex.shader_defs.extend(defs.iter().cloned());
        descriptor.fragment.as_mut()?.shader_defs.extend(defs);
        Some(descriptor)
    }
}

/// The bind group of a piece's blades, after the view's two, where a mesh's would be.
const BLADES_GROUP: usize = 2;

type DrawGrassBlades = (
    SetMeshViewBindGroup<0>,
    SetMeshViewBindingArrayBindGroup<1>,
    DrawBlades,
);

/// Draws the cells of a terrain's grass an item stands for, of what the view draws of it, each on
/// its material's pipeline with its piece's bindings and its material's.
struct DrawBlades;

impl<P: PhaseItem> RenderCommand<P> for DrawBlades {
    type Param = (
        SRes<GrassPipeline>,
        SRes<PipelineCache>,
        SRes<RenderProgress>,
    );
    type ViewQuery = Read<ExtractedView>;
    type ItemQuery = Read<GrassBindings>;

    fn render<'w>(
        item: &P,
        view: ROQueryItem<'w, '_, Self::ViewQuery>,
        bindings: Option<ROQueryItem<'w, '_, Self::ItemQuery>>,
        (pipeline, cache, progress): SystemParamItem<'w, '_, Self::Param>,
        pass: &mut TrackedRenderPass<'w>,
    ) -> RenderCommandResult {
        let (pipeline, cache, progress) = (
            pipeline.into_inner(),
            cache.into_inner(),
            progress.into_inner(),
        );
        let (Some(bindings), Some(drawn)) = (
            bindings,
            pipeline
                .drawn
                .get(&(view.retained_view_entity, item.entity())),
        ) else {
            progress.left_out();
            return RenderCommandResult::Skip;
        };
        let range = item.batch_range();
        for draw in drawn
            .get(range.start as usize..range.end as usize)
            .into_iter()
            .flatten()
        {
            let Some(bound) = bindings.0.get(draw.piece as usize).and_then(Option::as_ref) else {
                progress.left_out();
                continue;
            };
            let Some(drawing) = cache.get_render_pipeline(draw.pipeline) else {
                progress.awaiting(cache.get_render_pipeline_state(draw.pipeline));
                continue;
            };
            pass.set_render_pipeline(drawing);
            pass.set_bind_group(BLADES_GROUP, &bound.blades, &[]);
            pass.set_bind_group(MATERIAL_BIND_GROUP_INDEX, &bound.material, &[]);
            pass.draw(draw.blades.start * 3..draw.blades.end * 3, 0..1);
        }
        RenderCommandResult::Success
    }
}
