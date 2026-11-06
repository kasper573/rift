//! Which prop instances each view draws, worked out on the GPU. Every scenery's arena has a draw
//! table: for each prop whose model is on the GPU, the sphere round its model, and for each of its
//! parts a command per level, each with a region of its own in a view's drawn instances, as large
//! as the prop's instances in the arena. Each frame, every view lists the arena's chunks whose box
//! reaches into its frustum, and the kernel culls their instances one by one: those it sees go into
//! the region of the level their size on screen asks for, of each part, and count in the command
//! drawing it. Instances smaller on screen than the grain are thinned by their seeds and grown to
//! make up for those dropped, as strewn copies are. The commands are drawn indirectly, so what the
//! kernel leaves out is never seen by the CPU.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Range;

use bevy::camera::primitives::{Aabb, Frustum};
use bevy::camera::visibility::RenderLayers;
use bevy::core_pipeline::{Core3d, Core3dSystems};
use bevy::math::Affine3A;
use bevy::pbr::{
    EARLY_SHADOW_PASS, LightEntity, ViewLightEntities, per_view_shadow_pass, shared_shadow_pass,
};
use bevy::prelude::*;
use bevy::render::diagnostic::RecordDiagnostics;
use bevy::render::render_resource::binding_types::{
    storage_buffer_read_only_sized, storage_buffer_sized, uniform_buffer,
};
use bevy::render::render_resource::{
    BindGroup, BindGroupEntries, BindGroupLayout, BindGroupLayoutEntries, Buffer, BufferDescriptor,
    BufferUsages, CommandEncoder, ComputePassDescriptor, ComputePipeline, DrawIndexedIndirectArgs,
    PipelineCompilationOptions, PipelineLayoutDescriptor, RawComputePipelineDescriptor,
    ShaderModuleDescriptor, ShaderSource, ShaderStages, ShaderType, StorageBuffer, UniformBuffer,
};
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::sync_world::MainEntity;
use bevy::render::view::{ExtractedView, RetainedViewEntity};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};

use super::arena::{
    ExtractedPropArena, GpuChunk, GpuInstances, InstanceArenas, InstanceRecord, ModelArena,
    PlacedModel,
};
use crate::core::render::camera::NEAR_PLANE;
use crate::core::render::world::lod::{GRAIN_PIXELS, STRAY_PIXELS};

pub struct PropCullPlugin;

impl Plugin for PropCullPlugin {
    fn build(&self, app: &mut App) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<PropCulls>()
            .add_systems(RenderStartup, make_kernel)
            .add_systems(
                Render,
                prepare_culls.in_set(RenderSystems::PrepareBindGroups),
            )
            .add_systems(
                Core3d,
                cull_props
                    .before(Core3dSystems::Prepass)
                    .before(per_view_shadow_pass::<EARLY_SHADOW_PASS>)
                    .before(shared_shadow_pass::<EARLY_SHADOW_PASS>),
            );
    }
}

/// The name the props a frame's views drew, read back from the GPU, go by among the render
/// diagnostics: one measurement for each scenery a view culled.
pub const PROPS_DRAWN: &str = "props_drawn";

/// How a view culls: its frustum's half-spaces (left, right, bottom, top, near and far), where
/// sizes on screen are measured from and how many pixels a metre spans a metre off it, whether a
/// metre spans as many however far it lies, whether what lies behind the near half-space is left
/// out, and the sizes on screen the kernel judges by.
#[derive(ShaderType, Clone, Copy, Debug, PartialEq)]
pub struct CullView {
    planes: [Vec4; 6],
    eye: Vec3,
    pixels_at_a_meter: f32,
    orthographic: u32,
    test_near: u32,
    grain_pixels: f32,
    stray_pixels: f32,
    near: f32,
}

impl CullView {
    /// The view through `clip_from_world` from `eye`, `pixels_at_a_meter` the pixels a metre spans
    /// a metre off it, or wherever it lies where `orthographic`; a light's view keeps what lies
    /// behind its near half-space, whose shadows still fall into what it sees.
    pub fn new(
        frustum: &Frustum,
        eye: Vec3,
        pixels_at_a_meter: f32,
        orthographic: bool,
        light: bool,
    ) -> CullView {
        CullView {
            planes: frustum.half_spaces.map(|half_space| half_space.normal_d()),
            eye,
            pixels_at_a_meter,
            orthographic: u32::from(orthographic),
            test_near: u32::from(!light),
            grain_pixels: GRAIN_PIXELS,
            stray_pixels: STRAY_PIXELS,
            near: NEAR_PLANE.0,
        }
    }

    /// The view an extracted view culls through, a light's when `light`.
    pub fn of(view: &ExtractedView, frustum: &Frustum, light: bool) -> CullView {
        let orthographic = view.clip_from_view.w_axis.w == 1.0;
        let pixels_at_a_meter = view.viewport.w as f32 / 2.0 * view.clip_from_view.y_axis.y;
        CullView::new(
            frustum,
            view.world_from_view.translation(),
            pixels_at_a_meter,
            orthographic,
            light,
        )
    }

    fn intersect_near(&self) -> bool {
        self.test_near != 0
    }
}

/// What a scenery's arena draws, as the kernel reads it: for each prop by its place among the
/// content's props, the sphere round its model and its parts; each part's levels, finest first;
/// each level's command; and the commands as they start each frame, each with its region of the
/// drawn instances and none in it yet.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DrawTable {
    pub props: Vec<PropDraw>,
    pub parts: Vec<PartDraw>,
    pub levels: Vec<LevelDraw>,
    pub commands: Vec<DrawCommand>,
    /// The parts drawn, each in its material, with its levels' commands among `commands`.
    pub buckets: Vec<DrawBucket>,
    /// The drawn instances all the regions hold.
    pub drawn_capacity: u32,
}

#[derive(ShaderType, Clone, Copy, Debug, Default, PartialEq)]
pub struct PropDraw {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub radius: f32,
    pub extent: f32,
    pub first_part: u32,
    pub parts: u32,
    unused: u32,
}

#[derive(ShaderType, Clone, Copy, Debug, Default, PartialEq)]
pub struct PartDraw {
    pub first_level: u32,
    pub levels: u32,
}

#[derive(ShaderType, Clone, Copy, Debug, Default, PartialEq)]
pub struct LevelDraw {
    pub strays: f32,
    pub command: u32,
}

/// A command drawing one level of one part: its indices among the model slab's, the vertex they
/// count from, and its region of the drawn instances.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrawCommand {
    pub first_index: u32,
    pub indices: u32,
    pub base_vertex: u32,
    pub first_drawn: u32,
}

impl DrawCommand {
    pub const BYTES: u64 = 20;

    fn args(&self, instances: u32) -> DrawIndexedIndirectArgs {
        DrawIndexedIndirectArgs {
            index_count: self.indices,
            instance_count: instances,
            first_index: self.first_index,
            base_vertex: self.base_vertex as i32,
            first_instance: self.first_drawn,
        }
    }
}

/// A part of a prop's model, drawn in its own material, by its place among the model's parts: its
/// levels' commands one after another.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrawBucket {
    pub prop: u32,
    pub part: u32,
    pub commands: Range<u32>,
}

impl DrawTable {
    /// The table drawing `props`: each prop by its place among the content's props, with its model
    /// as the slabs hold it and how many of its instances the arena holds, the counts of a prop
    /// named more than once, as by each chunk it stands in, added up.
    pub fn of<'a>(props: impl IntoIterator<Item = (u32, &'a PlacedModel, u32)>) -> DrawTable {
        let mut held: BTreeMap<u32, (&PlacedModel, u32)> = BTreeMap::new();
        for (prop, model, instances) in props {
            held.entry(prop).or_insert((model, 0)).1 += instances;
        }
        let props: Vec<(u32, &PlacedModel, u32)> = held
            .into_iter()
            .filter(|(_, (_, instances))| *instances > 0)
            .map(|(prop, (model, instances))| (prop, model, instances))
            .collect();
        let mut table = DrawTable {
            props: vec![
                PropDraw::default();
                props.last().map_or(0, |(prop, _, _)| *prop as usize + 1)
            ],
            ..default()
        };
        for (prop, model, instances) in props {
            let middle = model.bounds.center;
            table.props[prop as usize] = PropDraw {
                x: middle.x,
                y: middle.y,
                z: middle.z,
                radius: model.bounds.half_extents.length(),
                extent: 2.0 * model.bounds.half_extents.max_element(),
                first_part: table.parts.len() as u32,
                parts: model.parts.len() as u32,
                unused: 0,
            };
            for (at, part) in (0..).zip(&model.parts) {
                let first_command = table.commands.len() as u32;
                table.parts.push(PartDraw {
                    first_level: table.levels.len() as u32,
                    levels: part.levels.len() as u32,
                });
                for level in &part.levels {
                    table.levels.push(LevelDraw {
                        strays: level.strays,
                        command: table.commands.len() as u32,
                    });
                    table.commands.push(DrawCommand {
                        first_index: level.first_index,
                        indices: level.indices,
                        base_vertex: part.first_vertex,
                        first_drawn: table.drawn_capacity,
                    });
                    table.drawn_capacity += instances;
                }
                table.buckets.push(DrawBucket {
                    prop,
                    part: at,
                    commands: first_command..table.commands.len() as u32,
                });
            }
        }
        table
    }

    /// The radius round a placement the model of `prop` reaches, at scale 1.
    fn radius_of(&self, prop: u32) -> f32 {
        self.props
            .get(prop as usize)
            .filter(|drawn| drawn.parts > 0)
            .map_or(0.0, |drawn| {
                drawn.radius + Vec3::new(drawn.x, drawn.y, drawn.z).length()
            })
    }
}

/// The culling kernel and the layout of what it reads and writes.
#[derive(Resource)]
pub struct CullKernel {
    layout: BindGroupLayout,
    pipeline: ComputePipeline,
}

impl CullKernel {
    pub fn new(device: &RenderDevice) -> CullKernel {
        let layout = device.create_bind_group_layout(
            "prop cull",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::COMPUTE,
                (
                    uniform_buffer::<CullView>(false),
                    storage_buffer_read_only_sized(false, None),
                    storage_buffer_read_only_sized(false, None),
                    storage_buffer_read_only_sized(false, None),
                    storage_buffer_read_only_sized(false, None),
                    storage_buffer_read_only_sized(false, None),
                    storage_buffer_sized(false, None),
                    storage_buffer_sized(false, None),
                ),
            ),
        );
        let module = device.create_and_validate_shader_module(ShaderModuleDescriptor {
            label: Some("prop cull"),
            source: ShaderSource::Wgsl(include_str!("cull.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("prop cull"),
            bind_group_layouts: &[Some(&layout)],
            ..default()
        });
        let pipeline = device.create_compute_pipeline(&RawComputePipelineDescriptor {
            label: Some("prop cull"),
            layout: Some(&pipeline_layout),
            module: &module,
            entry_point: Some("cull"),
            compilation_options: PipelineCompilationOptions::default(),
            cache: None,
        });
        CullKernel { layout, pipeline }
    }
}

/// A draw table on the GPU.
pub struct GpuDrawTable {
    table: DrawTable,
    props: StorageBuffer<Vec<PropDraw>>,
    parts: StorageBuffer<Vec<PartDraw>>,
    levels: StorageBuffer<Vec<LevelDraw>>,
    /// The commands as each frame starts them, after the four bytes the drawn count takes.
    starts: Buffer,
}

impl GpuDrawTable {
    /// `table` on the GPU; none when it draws nothing.
    pub fn new(
        device: &RenderDevice,
        queue: &RenderQueue,
        table: DrawTable,
    ) -> Option<GpuDrawTable> {
        if table.commands.is_empty() {
            return None;
        }
        let mut props = StorageBuffer::from(table.props.clone());
        let mut parts = StorageBuffer::from(table.parts.clone());
        let mut levels = StorageBuffer::from(table.levels.clone());
        props.write_buffer(device, queue);
        parts.write_buffer(device, queue);
        levels.write_buffer(device, queue);
        let starts = device.create_buffer(&BufferDescriptor {
            label: Some("prop draw starts"),
            size: commands_bytes(table.commands.len()),
            usage: BufferUsages::COPY_SRC | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&starts, 0, &starts_bytes(&table.commands));
        Some(GpuDrawTable {
            table,
            props,
            parts,
            levels,
            starts,
        })
    }

    pub fn table(&self) -> &DrawTable {
        &self.table
    }
}

/// What one view draws of a scenery: the view, the chunks it may see, each by its first instance
/// in the arena and how many it holds, the commands, and the instances they draw.
pub struct ViewCull {
    view: Option<UniformBuffer<CullView>>,
    chunks: StorageBuffer<Vec<UVec2>>,
    seen: u32,
    commands: Buffer,
    drawn: Buffer,
    bind_group: Option<BindGroup>,
}

impl ViewCull {
    /// Room for a view to draw what `table` does.
    pub fn new(device: &RenderDevice, table: &GpuDrawTable) -> ViewCull {
        ViewCull {
            view: None,
            chunks: StorageBuffer::default(),
            seen: 0,
            commands: device.create_buffer(&BufferDescriptor {
                label: Some("prop draw commands"),
                size: commands_bytes(table.table.commands.len()),
                usage: BufferUsages::STORAGE
                    | BufferUsages::INDIRECT
                    | BufferUsages::COPY_DST
                    | BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            drawn: device.create_buffer(&BufferDescriptor {
                label: Some("props drawn"),
                size: u64::from(table.table.drawn_capacity.max(1)) * DRAWN_BYTES,
                usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            bind_group: None,
        }
    }

    /// Culls the chunks `seen` of the instances `records` holds through `view` this frame.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        kernel: &CullKernel,
        table: &GpuDrawTable,
        records: &Buffer,
        seen: Vec<UVec2>,
        view: CullView,
    ) {
        self.seen = seen.len() as u32;
        self.chunks.set(if seen.is_empty() {
            vec![UVec2::ZERO]
        } else {
            seen
        });
        self.chunks.write_buffer(device, queue);
        let uniform = self.view.get_or_insert_with(|| UniformBuffer::from(view));
        uniform.set(view);
        uniform.write_buffer(device, queue);
        self.bind_group = Some(device.create_bind_group(
            "prop cull",
            &kernel.layout,
            &BindGroupEntries::sequential((
                uniform.binding().expect("a view just written"),
                records.as_entire_binding(),
                self.chunks.binding().expect("chunks just written"),
                table.props.binding().expect("a table written when made"),
                table.parts.binding().expect("a table written when made"),
                table.levels.binding().expect("a table written when made"),
                self.commands.as_entire_binding(),
                self.drawn.as_entire_binding(),
            )),
        ));
    }

    /// Starts every command anew and culls the chunks seen into them.
    pub fn encode(&self, kernel: &CullKernel, table: &GpuDrawTable, encoder: &mut CommandEncoder) {
        encoder.copy_buffer_to_buffer(&table.starts, 0, &self.commands, 0, self.commands.size());
        let Some(bind_group) = self.bind_group.as_ref().filter(|_| self.seen > 0) else {
            return;
        };
        let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
            label: Some("prop cull"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&kernel.pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.dispatch_workgroups(self.seen, 1, 1);
    }

    /// The commands the view draws its props with, after the four bytes the count drawn takes.
    pub fn commands(&self) -> &Buffer {
        &self.commands
    }

    /// The instances the commands draw, each by its place among the arena's and the scale it is
    /// drawn at.
    pub fn drawn(&self) -> &Buffer {
        &self.drawn
    }

    /// Where `command` lies among [`Self::commands`].
    pub fn command_offset(command: u32) -> u64 {
        COUNT_BYTES + u64::from(command) * DrawCommand::BYTES
    }

    /// Reads back what the view's kernel drew, waiting for the GPU: for checks of the kernel.
    pub fn read(&self, device: &RenderDevice, queue: &RenderQueue) -> CullReadback {
        let commands = read_buffer(device, queue, &self.commands);
        let drawn = read_buffer(device, queue, &self.drawn);
        let words = |bytes: &[u8]| -> Vec<u32> {
            bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|word| u32::from_le_bytes(*word))
                .collect()
        };
        let command_words = words(&commands);
        CullReadback {
            drawn: command_words[0],
            instances: command_words[1..]
                .as_chunks::<5>()
                .0
                .iter()
                .map(|command| command[1])
                .collect(),
            listed: words(&drawn)
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&[record, scale]| (record, f32::from_bits(scale)))
                .collect(),
        }
    }
}

/// What a view's kernel drew: how many instances in all, each command's count, and the drawn
/// instances' list, each by its place among the arena's and its scale, as far as it was filled.
#[derive(Clone, Debug, PartialEq)]
pub struct CullReadback {
    pub drawn: u32,
    pub instances: Vec<u32>,
    pub listed: Vec<(u32, f32)>,
}

/// The chunks of an arena whose box reaches into `view`'s frustum, each by its first instance
/// among the arena's and how many it holds, in the order of their keys.
pub fn seen_chunks(
    chunks: &HashMap<u32, GpuChunk>,
    table: &DrawTable,
    frustum: &Frustum,
    view: &CullView,
) -> Vec<UVec2> {
    let mut seen: Vec<(u32, UVec2)> = chunks
        .iter()
        .filter(|(_, chunk)| {
            let reach = chunk
                .contents
                .props
                .iter()
                .map(|(prop, _, _)| table.radius_of(*prop))
                .fold(0.0, f32::max)
                * chunk.contents.largest_scale;
            let bounds = Aabb::from_min_max(
                chunk.contents.low - Vec3::splat(reach),
                chunk.contents.high + Vec3::splat(reach),
            );
            frustum.intersects_obb(&bounds, &Affine3A::IDENTITY, view.intersect_near(), true)
        })
        .map(|(&key, chunk)| {
            (
                key,
                UVec2::new(
                    (chunk.offset / InstanceRecord::BYTES) as u32,
                    chunk.instances,
                ),
            )
        })
        .collect();
    seen.sort_by_key(|(key, _)| *key);
    seen.into_iter().map(|(_, span)| span).collect()
}

/// What each scenery draws: its table, and what each view draws of it.
#[derive(Resource, Default)]
pub struct PropCulls(pub HashMap<Entity, SceneryCull>);

#[derive(Default)]
pub struct SceneryCull {
    pub table: Option<GpuDrawTable>,
    /// The arena's and the models' changes the table was made after.
    made_after: Option<(u64, u64)>,
    pub views: HashMap<RetainedViewEntity, ViewCull>,
}

/// The bytes a drawn instance takes: its place among the arena's, and its scale.
const DRAWN_BYTES: u64 = 8;
/// The bytes the count of instances drawn takes, before the commands.
const COUNT_BYTES: u64 = 4;

fn make_kernel(mut commands: Commands, device: Res<RenderDevice>) {
    commands.insert_resource(CullKernel::new(&device));
}

/// Makes each scenery's table anew once its arena or the models have changed, and what each view
/// that shows it culls: the chunks that reach into its frustum.
#[allow(clippy::too_many_arguments)]
pub(super) fn prepare_culls(
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    kernel: Option<Res<CullKernel>>,
    arenas: Res<InstanceArenas>,
    models: Res<ModelArena>,
    mut culls: ResMut<PropCulls>,
    sceneries: Query<(&MainEntity, &ExtractedPropArena)>,
    views: Query<(
        &ExtractedView,
        &Frustum,
        Option<&RenderLayers>,
        Has<LightEntity>,
    )>,
) {
    let Some(kernel) = kernel else {
        return;
    };
    culls
        .0
        .retain(|scenery, _| sceneries.iter().any(|(main, _)| main.id() == *scenery));
    for (main, drawn) in &sceneries {
        let Some(arena) = arenas.0.get(&main.id()) else {
            continue;
        };
        let cull = culls.0.entry(main.id()).or_default();
        let changes = (arena.changes, models.changes);
        if cull.made_after != Some(changes) {
            cull.table = GpuDrawTable::new(&device, &queue, draw_table(arena, &models));
            cull.made_after = Some(changes);
            cull.views.clear();
        }
        let (Some(table), Some(records)) = (&cull.table, arena.slab.buffer()) else {
            continue;
        };
        let mut shown = HashSet::new();
        for (view, frustum, layers, light) in &views {
            if !layers.unwrap_or_default().intersects(&drawn.layers) {
                continue;
            }
            let culled = CullView::of(view, frustum, light);
            let seen = seen_chunks(&arena.chunks, &table.table, frustum, &culled);
            cull.views
                .entry(view.retained_view_entity)
                .or_insert_with(|| ViewCull::new(&device, table))
                .update(&device, &queue, &kernel, table, records, seen, culled);
            shown.insert(view.retained_view_entity);
        }
        cull.views.retain(|view, _| shown.contains(view));
    }
}

/// Culls every scenery the view shows, and what each of the view's lights casts shadows of,
/// before anything of the view or its shadows is drawn: the props the view itself draws counted
/// among the render diagnostics.
fn cull_props(
    view: ViewQuery<(&ExtractedView, Option<&ViewLightEntities>)>,
    lights: Query<&ExtractedView, With<LightEntity>>,
    kernel: Option<Res<CullKernel>>,
    culls: Res<PropCulls>,
    mut ctx: RenderContext,
) {
    let Some(kernel) = kernel else {
        return;
    };
    let (seen, view_lights) = view.into_inner();
    let shadows: Vec<RetainedViewEntity> = view_lights
        .into_iter()
        .flat_map(|view_lights| view_lights.lights.iter())
        .filter_map(|&light| lights.get(light).ok())
        .map(|light| light.retained_view_entity)
        .collect();
    let recorder = ctx.diagnostic_recorder();
    let recorder = recorder.as_deref();
    let encoder = ctx.command_encoder();
    for cull in culls.0.values() {
        let Some(table) = &cull.table else {
            continue;
        };
        if let Some(culled) = cull.views.get(&seen.retained_view_entity) {
            culled.encode(&kernel, table, encoder);
            if let Some(recorder) = recorder {
                recorder.record_u32(encoder, &culled.commands.slice(0..COUNT_BYTES), PROPS_DRAWN);
            }
        }
        for light in &shadows {
            if let Some(culled) = cull.views.get(light) {
                culled.encode(&kernel, table, encoder);
            }
        }
    }
}

/// The table drawing what `arena` holds, of the models on the GPU.
fn draw_table(arena: &GpuInstances, models: &ModelArena) -> DrawTable {
    DrawTable::of(arena.chunks.values().flat_map(|chunk| {
        chunk
            .contents
            .props
            .iter()
            .filter_map(|&(prop, model, instances)| {
                Some((prop, models.slabs.get(&model)?, instances))
            })
    }))
}

fn commands_bytes(commands: usize) -> u64 {
    COUNT_BYTES + commands as u64 * DrawCommand::BYTES
}

/// The bytes of `commands` as each frame starts them: none drawn, and no instance in any.
fn starts_bytes(commands: &[DrawCommand]) -> Vec<u8> {
    let mut bytes = vec![0; COUNT_BYTES as usize];
    for command in commands {
        bytes.extend_from_slice(command.args(0).as_bytes());
    }
    bytes
}

fn read_buffer(device: &RenderDevice, queue: &RenderQueue, buffer: &Buffer) -> Vec<u8> {
    let staging = device.create_buffer(&BufferDescriptor {
        label: Some("prop cull readback"),
        size: buffer.size(),
        usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&default());
    encoder.copy_buffer_to_buffer(buffer, 0, &staging, 0, buffer.size());
    queue.submit([encoder.finish()]);
    let slice = staging.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(
        bevy::render::render_resource::MapMode::Read,
        move |mapped| {
            let _ = sender.send(mapped);
        },
    );
    device
        .poll(bevy::render::render_resource::PollType::wait_indefinitely())
        .expect("the GPU finishing the readback");
    receiver
        .recv()
        .expect("the readback's answer")
        .expect("the readback mapping");
    let bytes = slice
        .get_mapped_range()
        .expect("the readback mapped")
        .to_vec();
    staging.unmap();
    bytes
}
