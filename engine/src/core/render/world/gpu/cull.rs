//! Which prop instances each view draws, worked out on the GPU. Every scenery's arena has a draw
//! table: for each prop whose model is on the GPU, the sphere round its model, and for each of its
//! parts a command per level. Each frame, every view lists the arena's chunks whose box reaches
//! into its frustum, and the kernel culls their instances one by one: those it sees count in the
//! command of the level their size on screen asks for, of each part; the commands' regions of the
//! view's drawn instances are then laid out as large as their counts, one after another, and the
//! seen instances listed in them. So a view's drawn instances take room for what it may see, each
//! instance once for each part of its model, and never for every level besides. Instances smaller
//! on screen than the grain are thinned by their seeds and grown to make up for those dropped, as
//! strewn copies are. The commands are drawn indirectly, so what the kernel leaves out is never
//! seen by the CPU.

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Range;
use std::sync::{Arc, Mutex};

use bevy::camera::primitives::{Aabb, Frustum};
use bevy::camera::visibility::RenderLayers;
use bevy::core_pipeline::mip_generation::experimental::depth::{
    DepthPyramidDummyTexture, DownsampleDepthPipelines, ViewDepthPyramid,
};
use bevy::core_pipeline::{Core3d, Core3dSystems};
use bevy::math::{Affine3A, Vec3A};
use bevy::pbr::{
    EARLY_SHADOW_PASS, LightEntity, ViewLightEntities, per_view_shadow_pass, shared_shadow_pass,
};
use bevy::prelude::*;
use bevy::render::diagnostic::RecordDiagnostics;
use bevy::render::extract_resource::ExtractResourcePlugin;
use bevy::render::render_resource::binding_types::{
    storage_buffer_read_only_sized, storage_buffer_sized, texture_2d, uniform_buffer,
};
use bevy::render::render_resource::{
    BindGroup, BindGroupEntries, BindGroupLayout, BindGroupLayoutEntries, BindingResource, Buffer,
    BufferDescriptor, BufferInitDescriptor, BufferUsages, CommandEncoder, ComputePassDescriptor,
    ComputePipeline, DrawIndexedIndirectArgs, MapMode, PipelineCache, PipelineCompilationOptions,
    PipelineLayoutDescriptor, RawComputePipelineDescriptor, ShaderModuleDescriptor, ShaderSource,
    ShaderStages, ShaderType, StorageBuffer, TextureSampleType, TextureUsages, TextureView,
    UniformBuffer,
};
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue, ViewQuery};
use bevy::render::sync_world::MainEntity;
use bevy::render::texture::TextureCache;
use bevy::render::view::{ExtractedView, RetainedViewEntity, ViewDepthStencilTexture};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shape::HalfSpace;

use super::arena::{
    self, BOX_INDICES, ChunkPlace, ExtractedPropArena, GpuChunk, GpuInstances, InstanceArenas,
    InstanceRecord, ModelArena, PlacedModel, QUAD_INDICES,
};
use super::clearing::{self, Clearing};
use super::impostor::{self, AtlasKey, ImpostorAtlas};
use crate::core::render::camera::NEAR_PLANE;
use crate::core::render::video::VideoPreferences;
use crate::core::render::world::RenderProgress;
use crate::core::render::world::lod::{DRAWN_STRAY_PIXELS, GRAIN_PIXELS, horizon_band};
use crate::core::units::Meters;

pub struct PropCullPlugin;

impl Plugin for PropCullPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Clearing>()
            .add_plugins((
                ExtractResourcePlugin::<Clearing>::default(),
                ExtractResourcePlugin::<VideoPreferences>::default(),
            ))
            .add_systems(PostUpdate, (clearing::aim_clearing, let_depth_be_read));
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<PropCulls>()
            .init_resource::<MissingReadbacks>()
            .add_systems(RenderStartup, make_kernel)
            .add_systems(
                Render,
                (
                    prepare_culls
                        .after(arena::write_models)
                        .after(arena::write_instances)
                        .after(impostor::ImpostorsPrepared)
                        .before(RenderSystems::Queue),
                    prepare_occlusion.in_set(RenderSystems::PrepareBindGroups),
                    read_missing.in_set(RenderSystems::Cleanup),
                ),
            )
            .add_systems(
                Core3d,
                (
                    cull_props
                        .before(Core3dSystems::Prepass)
                        .before(per_view_shadow_pass::<EARLY_SHADOW_PASS>)
                        .before(shared_shadow_pass::<EARLY_SHADOW_PASS>),
                    occlude_props
                        .after(Core3dSystems::Prepass)
                        .before(Core3dSystems::MainPass),
                ),
            );
    }
}

/// The name the props a frame's views drew, read back from the GPU, go by among the render
/// diagnostics: one measurement for each scenery a view culled.
pub const PROPS_DRAWN: &str = "props_drawn";

/// The name the passes culling a view's props and its shadows' go by among the render
/// diagnostics.
pub const PROP_CULL: &str = "prop_cull";
/// The GPU pass that builds a view's depth pyramid and keeps what it doesn't hide, in the frame's
/// timings.
pub const PROP_OCCLUDE: &str = "prop_occlude";

/// How a view culls: its frustum's half-spaces (left, right, bottom, top, near and far), where
/// sizes on screen are measured from and how many pixels a metre spans a metre off it, whether a
/// metre spans as many however far it lies, whether what lies behind the near half-space is left
/// out, the sizes on screen the kernel judges by, how far off the eye props still stand (none
/// for as far as they go), for the view that clears what hides the camera's subject, whom it
/// follows and how far a fade moves this frame, and how many chunks it sees this frame.
#[derive(ShaderType, Clone, Copy, Debug, PartialEq)]
pub struct CullView {
    planes: [Vec4; 6],
    eye: Vec3,
    pixels_at_a_meter: f32,
    orthographic: u32,
    test_near: u32,
    casting_sine: f32,
    grain_pixels: f32,
    stray_pixels: f32,
    near: f32,
    impostor_pixels: f32,
    band_pixels: f32,
    horizon_start: f32,
    horizon: f32,
    subject: Vec3,
    fade_step: f32,
    chunks_seen: u32,
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
            casting_sine: 0.0,
            grain_pixels: GRAIN_PIXELS,
            stray_pixels: DRAWN_STRAY_PIXELS,
            near: NEAR_PLANE.0,
            impostor_pixels: IMPOSTOR_PIXELS,
            band_pixels: BAND_PIXELS,
            horizon_start: 0.0,
            horizon: 0.0,
            subject: Vec3::ZERO,
            fade_step: 0.0,
            chunks_seen: 0,
        }
    }

    /// The view with its props standing no farther off its eye than `horizon`, thinning out
    /// toward it.
    pub fn within(self, horizon: Option<Meters>) -> CullView {
        let (horizon_start, horizon) = horizon_band(horizon);
        CullView {
            horizon_start,
            horizon,
            ..self
        }
    }

    /// The view handing each prop from its model to its impostor where it spans `pixels` across.
    pub fn handing_over_at(self, pixels: f32) -> CullView {
        CullView {
            impostor_pixels: pixels,
            ..self
        }
    }

    /// The view clearing what comes between its eye and `subject`, each fade moving `step` of the
    /// way this frame. Every other view draws its props whole.
    pub fn clearing(self, subject: Vec3, step: f32) -> CullView {
        CullView {
            subject,
            fade_step: step,
            ..self
        }
    }

    /// The view an extracted view culls through, a light's when `light`: through `frustum`, a
    /// light's with the near half-space its projection puts there (see [`lit_frustum`]).
    pub fn of(view: &ExtractedView, frustum: &Frustum, light: bool) -> CullView {
        let orthographic = view.clip_from_view.w_axis.w == 1.0;
        let pixels_at_a_meter = view.viewport.w as f32 / 2.0 * view.clip_from_view.y_axis.y;
        let culled = CullView::new(
            frustum,
            view.world_from_view.translation(),
            pixels_at_a_meter,
            orthographic,
            light,
        );
        if light {
            culled.lit_from(sun_sine(view))
        } else {
            culled
        }
    }

    /// A light's view whose sun stands `sine` of the way up the sky: a caster farther toward the
    /// sun than its height over that sine throws its shadow short of what the view sees, so the
    /// view leaves it out, as far as the near half-space those casters lie behind tells.
    pub fn lit_from(self, sine: f32) -> CullView {
        if sine <= SUN_RIM {
            return self;
        }
        CullView {
            test_near: 1,
            casting_sine: sine,
            ..self
        }
    }

    fn intersect_near(&self) -> bool {
        self.test_near != 0
    }

    /// How far behind the near half-space a caster `height` tall may still throw its shadow into
    /// the view, in metres: without end for a view that keeps all behind it.
    fn casting_from(&self, height: f32) -> f32 {
        if self.casting_sine > 0.0 {
            height / self.casting_sine
        } else {
            f32::INFINITY
        }
    }
}

/// How high the sun's sine must stand for a light's view to bound its casters: lower, their
/// shadows run too long for the bound to leave any out.
pub const SUN_RIM: f32 = 0.01;

/// The sine of how high the sun of a light's view stands over the horizon: what the view looks
/// back along, lifted.
pub fn sun_sine(view: &ExtractedView) -> f32 {
    view.world_from_view.back().y
}

/// A light's `frustum` with the near half-space its orthographic projection puts there. The
/// frustum bevy derives from a reversed-depth orthographic projection holds no near half-space
/// (its fifth lies twice the far depth off), so that every caster behind a cascade casts into it;
/// a cull that bounds those casters needs the plane.
pub fn lit_frustum(clip_from_world: &Mat4, frustum: &Frustum) -> Frustum {
    let mut frustum = *frustum;
    frustum.half_spaces[4] = HalfSpace::new(clip_from_world.row(3) - clip_from_world.row(2));
    frustum
}

/// The matrix an extracted view projects the world through.
pub fn clip_from_world_of(view: &ExtractedView) -> Mat4 {
    view.clip_from_view * view.world_from_view.to_matrix().inverse()
}

/// `frustum` with its near half-space moved `beyond` metres back, so a light's view keeps the
/// casters behind it that still throw their shadows into it.
pub fn casting_frustum(frustum: &Frustum, beyond: f32) -> Frustum {
    let mut frustum = *frustum;
    if beyond.is_finite() {
        let near = frustum.half_spaces[4].normal_d();
        frustum.half_spaces[4] = HalfSpace::new(near + Vec4::new(0.0, 0.0, 0.0, beyond));
    }
    frustum
}

/// What a scenery's arena draws, as the kernel reads it: for each prop by its place among the
/// content's props, the sphere round its model, its parts and the commands drawing its impostor
/// and its box; each part's levels, finest first;
/// each level's command; and the commands as they start each frame, with nothing drawn by them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DrawTable {
    pub props: Vec<PropDraw>,
    pub parts: Vec<PartDraw>,
    pub levels: Vec<LevelDraw>,
    pub commands: Vec<DrawCommand>,
    /// The parts drawn, each in its material, with its levels' commands among `commands`.
    pub buckets: Vec<DrawBucket>,
    /// The commands drawing the props' impostors, one a prop drawn, and those drawing their
    /// boxes, in the same order, among `commands`.
    pub impostors: Range<u32>,
    pub proxies: Range<u32>,
    /// The atlas each prop is drawn from afar by, by its place among `props`: none for a prop
    /// drawn by nothing or as its mesh alone.
    pub atlases: Vec<Option<AtlasKey>>,
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
    /// Whether the prop clears away where it hides the camera's subject, tested by its box, `half`
    /// each way of the middle.
    pub fades: u32,
    pub half_x: f32,
    pub half_y: f32,
    pub half_z: f32,
    pub impostor_command: u32,
    pub proxy_command: u32,
    /// How translucent its materials are at most, which its impostor is lit by.
    pub translucency: f32,
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

/// A command drawing one level of one part: its indices among the model slab's, and the vertex
/// they count from. Its region of the drawn instances is the kernel's to lay out each frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrawCommand {
    pub first_index: u32,
    pub indices: u32,
    pub base_vertex: u32,
}

impl DrawCommand {
    pub const BYTES: u64 = 20;

    /// The command as a frame starts it: nothing drawn, and its region nowhere yet.
    fn start(&self) -> DrawIndexedIndirectArgs {
        DrawIndexedIndirectArgs {
            index_count: self.indices,
            instance_count: 0,
            first_index: self.first_index,
            base_vertex: self.base_vertex as i32,
            first_instance: 0,
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

/// A prop as the table is made from: its model as the slabs hold it, how many of its instances
/// the arena holds, whether its parts are drawn, in their materials, and whether it clears away
/// where it hides the camera's subject.
#[derive(Clone, Copy)]
pub struct DrawnProp<'a> {
    pub model: AssetId<Gltf>,
    pub placed: &'a PlacedModel,
    pub instances: u32,
    pub dressed: bool,
    pub fades: bool,
    /// How far its shaders draw past its model, in the model's metres, which its sphere and box
    /// grow by.
    pub overhang: f32,
    /// How translucent its materials are at most.
    pub translucency: f32,
    /// The atlas it is drawn from afar by, if it has one.
    pub impostor: Option<AtlasKey>,
}

impl DrawTable {
    /// The table drawing `props`: each prop by its place among the content's props, the counts of
    /// a prop named more than once, as by each chunk it stands in, added up. A prop's parts are
    /// drawn only while it is dressed; its impostor and its box whenever its model is held.
    pub fn of<'a>(props: impl IntoIterator<Item = (u32, DrawnProp<'a>)>) -> DrawTable {
        let mut held: BTreeMap<u32, DrawnProp> = BTreeMap::new();
        for (prop, drawn) in props {
            match held.entry(prop) {
                Entry::Vacant(vacant) => {
                    vacant.insert(drawn);
                }
                Entry::Occupied(mut occupied) => occupied.get_mut().instances += drawn.instances,
            }
        }
        let props: Vec<(u32, DrawnProp)> = held
            .into_iter()
            .filter(|(_, drawn)| drawn.instances > 0)
            .collect();
        let count = props.last().map_or(0, |(prop, _)| *prop as usize + 1);
        let mut table = DrawTable {
            props: vec![PropDraw::default(); count],
            atlases: vec![None; count],
            ..default()
        };
        for (prop, drawn) in &props {
            let model = drawn.placed;
            let middle = model.bounds.center;
            let half = model.bounds.half_extents + Vec3A::splat(drawn.overhang);
            table.atlases[*prop as usize] = drawn.impostor;
            table.props[*prop as usize] = PropDraw {
                x: middle.x,
                y: middle.y,
                z: middle.z,
                radius: (model.bounds.half_extents.length() + drawn.overhang)
                    .max(f32::MIN_POSITIVE),
                extent: 2.0 * half.max_element(),
                first_part: table.parts.len() as u32,
                parts: if drawn.dressed {
                    model.parts.len() as u32
                } else {
                    0
                },
                fades: u32::from(drawn.fades),
                half_x: half.x,
                half_y: half.y,
                half_z: half.z,
                impostor_command: 0,
                proxy_command: 0,
                translucency: drawn.translucency,
            };
            if !drawn.dressed {
                continue;
            }
            let prop = *prop;
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
                    });
                }
                table.buckets.push(DrawBucket {
                    prop,
                    part: at,
                    commands: first_command..table.commands.len() as u32,
                });
            }
        }
        let first_impostor = table.commands.len() as u32;
        for (prop, _) in &props {
            table.props[*prop as usize].impostor_command = table.commands.len() as u32;
            table.commands.push(Shapes::QUAD);
        }
        let first_proxy = table.commands.len() as u32;
        for (prop, _) in &props {
            table.props[*prop as usize].proxy_command = table.commands.len() as u32;
            table.commands.push(Shapes::BOX);
        }
        table.impostors = first_impostor..first_proxy;
        table.proxies = first_proxy..table.commands.len() as u32;
        table
    }

    /// How many times an instance of `prop` may be listed at most: once for each part of its
    /// model and once more for its impostor or its box; none when nothing of it is drawn.
    fn listings_of(&self, prop: u32) -> u32 {
        self.props
            .get(prop as usize)
            .filter(|drawn| drawn.radius > 0.0)
            .map_or(0, |drawn| drawn.parts + 1)
    }

    /// The radius round a placement the model of `prop` reaches, at scale 1.
    pub fn radius_of(&self, prop: u32) -> f32 {
        self.props
            .get(prop as usize)
            .filter(|drawn| drawn.radius > 0.0)
            .map_or(0.0, |drawn| {
                drawn.radius + Vec3::new(drawn.x, drawn.y, drawn.z).length()
            })
    }
}

/// The culling kernel and the layout of what it reads and writes.
#[derive(Resource)]
pub struct CullKernel {
    layout: BindGroupLayout,
    occlusion_layout: BindGroupLayout,
    count: ComputePipeline,
    place: ComputePipeline,
    list: ComputePipeline,
    reset_kept: ComputePipeline,
    occlude: ComputePipeline,
}

/// What the occlusion of a view's listings reads: its matrices.
#[derive(ShaderType, Clone, Copy, Debug, PartialEq)]
struct Occlusion {
    view_from_world: Mat4,
    clip_from_view: Mat4,
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
                    storage_buffer_sized(false, None),
                    storage_buffer_read_only_sized(false, None),
                    storage_buffer_read_only_sized(false, None),
                ),
            ),
        );
        let occlusion_layout = device.create_bind_group_layout(
            "prop occlusion",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::COMPUTE,
                (
                    uniform_buffer::<Occlusion>(false),
                    texture_2d(TextureSampleType::Float { filterable: false }),
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
        let occlusion_pipeline_layout = device.create_pipeline_layout(&PipelineLayoutDescriptor {
            label: Some("prop occlusion"),
            bind_group_layouts: &[Some(&layout), Some(&occlusion_layout)],
            ..default()
        });
        let pass = |pipeline_layout: &_, entry: &'static str| {
            device.create_compute_pipeline(&RawComputePipelineDescriptor {
                label: Some(entry),
                layout: Some(pipeline_layout),
                module: &module,
                entry_point: Some(entry),
                compilation_options: PipelineCompilationOptions::default(),
                cache: None,
            })
        };
        CullKernel {
            count: pass(&pipeline_layout, "count"),
            place: pass(&pipeline_layout, "place"),
            list: pass(&pipeline_layout, "list"),
            reset_kept: pass(&occlusion_pipeline_layout, "reset_kept"),
            occlude: pass(&occlusion_pipeline_layout, "occlude"),
            layout,
            occlusion_layout,
        }
    }
}

/// A draw table on the GPU.
pub struct GpuDrawTable {
    table: DrawTable,
    props: StorageBuffer<Vec<PropDraw>>,
    parts: StorageBuffer<Vec<PartDraw>>,
    levels: StorageBuffer<Vec<LevelDraw>>,
    /// Each prop's layer of the impostor atlas, one more than it is, 0 for none, and the
    /// residency's round it was written after.
    impostors: StorageBuffer<Vec<u32>>,
    impostors_after: Option<u64>,
    /// The commands as each frame starts them, after the bytes the counts take.
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
        let mut impostors = StorageBuffer::from(vec![0u32; table.props.len().max(1)]);
        props.write_buffer(device, queue);
        parts.write_buffer(device, queue);
        levels.write_buffer(device, queue);
        impostors.write_buffer(device, queue);
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
            impostors,
            impostors_after: None,
            starts,
        })
    }

    pub fn table(&self) -> &DrawTable {
        &self.table
    }

    /// Tells the kernel which props the impostor atlas holds a picture of, at which layer, as
    /// `layer_of` says, once the atlas's `round` differs from the one told before.
    pub fn place_impostors(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        round: u64,
        layer_of: impl Fn(AtlasKey) -> Option<u32>,
    ) {
        if self.impostors_after == Some(round) {
            return;
        }
        let layers: Vec<u32> = self
            .table
            .atlases
            .iter()
            .map(|key| key.and_then(&layer_of).map_or(0, |layer| layer + 1))
            .collect();
        self.impostors
            .set(if layers.is_empty() { vec![0] } else { layers });
        self.impostors.write_buffer(device, queue);
        self.impostors_after = Some(round);
    }

    /// The props' bindings the stages read beside the instances: the props and their impostor
    /// layers.
    pub fn prop_bindings(&self) -> Option<(BindingResource<'_>, BindingResource<'_>)> {
        Some((self.props.binding()?, self.impostors.binding()?))
    }
}

/// What one view draws of a scenery: the view, the chunks it may see, each by its first instance
/// in the arena and how many it holds, the commands, and the instances they draw, with room for
/// every instance the chunks hold, once for each part of its model; and for a view whose prepass
/// depth is read, the commands and instances kept of them for the lit passes.
pub struct ViewCull {
    view: Option<UniformBuffer<CullView>>,
    chunks: StorageBuffer<Vec<UVec4>>,
    /// The workgroups that give every instance of the chunks seen a thread.
    groups: u32,
    commands: Buffer,
    drawn: Buffer,
    bind_group: Option<BindGroup>,
    kept_commands: Buffer,
    kept: Buffer,
    occlusion: Option<UniformBuffer<Occlusion>>,
    occluding: Option<BindGroup>,
}

impl ViewCull {
    /// Room for a view to draw what `table` does.
    pub fn new(device: &RenderDevice, table: &GpuDrawTable) -> ViewCull {
        ViewCull {
            view: None,
            chunks: StorageBuffer::default(),
            groups: 0,
            commands: commands_buffer(device, table),
            drawn: drawn_buffer(device, 1),
            bind_group: None,
            kept_commands: commands_buffer(device, table),
            kept: drawn_buffer(device, 1),
            occlusion: None,
            occluding: None,
        }
    }

    /// Keeps for the lit passes only the listings whose instances the depth the prepass drew,
    /// `pyramid` holding each level of it, doesn't hide whole; until the next update. The view
    /// is the one of `world_from_view` through `clip_from_view`.
    pub fn occlude_with(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        kernel: &CullKernel,
        pyramid: &TextureView,
        world_from_view: Mat4,
        clip_from_view: Mat4,
    ) {
        let occlusion = Occlusion {
            view_from_world: world_from_view.inverse(),
            clip_from_view,
        };
        let uniform = self
            .occlusion
            .get_or_insert_with(|| UniformBuffer::from(occlusion));
        uniform.set(occlusion);
        uniform.write_buffer(device, queue);
        self.occluding = Some(device.create_bind_group(
            "prop occlusion",
            &kernel.occlusion_layout,
            &BindGroupEntries::sequential((
                uniform.binding().expect("an occlusion just written"),
                pyramid,
                self.kept_commands.as_entire_binding(),
                self.kept.as_entire_binding(),
            )),
        ));
    }

    /// Whether the lit passes draw what the prepass's depth leaves of the listings.
    pub fn occludes(&self) -> bool {
        self.occluding.is_some()
    }

    /// The commands the lit passes draw with: [`Self::commands`] but for what the prepass's
    /// depth hides.
    pub fn lit_commands(&self) -> &Buffer {
        if self.occludes() {
            &self.kept_commands
        } else {
            &self.commands
        }
    }

    /// The instances the lit passes draw: [`Self::drawn`] but for what the prepass's depth hides.
    pub fn lit_drawn(&self) -> &Buffer {
        if self.occludes() {
            &self.kept
        } else {
            &self.drawn
        }
    }

    /// Keeps the listings the prepass's depth doesn't hide for the lit passes, once the prepass is
    /// drawn and `pyramid` holds its depth.
    pub fn encode_occlusion(
        &self,
        kernel: &CullKernel,
        table: &GpuDrawTable,
        encoder: &mut CommandEncoder,
    ) {
        let (Some(bind_group), Some(occluding)) = (&self.bind_group, &self.occluding) else {
            return;
        };
        encoder.copy_buffer_to_buffer(
            &self.commands,
            0,
            &self.kept_commands,
            0,
            self.commands.size(),
        );
        let listings = (self.drawn.size() / DRAWN_BYTES) as u32;
        for (label, pipeline, groups) in [
            (
                "prop occlusion reset",
                &kernel.reset_kept,
                (table.table.commands.len() as u32).div_ceil(THREADS),
            ),
            (
                "prop occlusion",
                &kernel.occlude,
                listings.div_ceil(THREADS),
            ),
        ] {
            let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                label: Some(label),
                timestamp_writes: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bind_group, &[]);
            pass.set_bind_group(1, occluding, &[]);
            let (columns, rows) = spread(groups);
            pass.dispatch_workgroups(columns, rows, 1);
        }
    }

    /// Culls the chunks `seen` of the instances `records` holds through `view` this frame, `shown`
    /// holding how far each shows, and `standing` which near chunks of the ground stand.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        device: &RenderDevice,
        queue: &RenderQueue,
        kernel: &CullKernel,
        table: &GpuDrawTable,
        records: &Buffer,
        shown: &Buffer,
        standing: &Buffer,
        seen: SeenChunks,
        view: CullView,
    ) {
        if u64::from(seen.listed) * DRAWN_BYTES > self.drawn.size() {
            self.drawn = drawn_buffer(device, seen.listed);
            self.kept = drawn_buffer(device, seen.listed);
        }
        self.occluding = None;
        self.groups = seen.instances.div_ceil(THREADS);
        let view = CullView {
            chunks_seen: seen.spans.len() as u32,
            ..view
        };
        self.chunks.set(if seen.spans.is_empty() {
            vec![UVec4::ZERO]
        } else {
            seen.spans
        });
        self.chunks.write_buffer(device, queue);
        let uniform = self.view.get_or_insert_with(|| UniformBuffer::from(view));
        uniform.set(view);
        uniform.write_buffer(device, queue);
        self.bind_group = Some(
            device.create_bind_group(
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
                    shown.as_entire_binding(),
                    table
                        .impostors
                        .binding()
                        .expect("a table written when made"),
                    standing.as_entire_binding(),
                )),
            ),
        );
    }

    /// Starts every command anew and culls the chunks seen into them: counted, placed and listed,
    /// each pass over once the one before is through.
    pub fn encode(&self, kernel: &CullKernel, table: &GpuDrawTable, encoder: &mut CommandEncoder) {
        encoder.copy_buffer_to_buffer(&table.starts, 0, &self.commands, 0, self.commands.size());
        let Some(bind_group) = self.bind_group.as_ref().filter(|_| self.groups > 0) else {
            return;
        };
        for (label, pipeline, groups) in [
            ("prop cull count", &kernel.count, self.groups),
            ("prop cull place", &kernel.place, 1),
            ("prop cull list", &kernel.list, self.groups),
        ] {
            let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
                label: Some(label),
                timestamp_writes: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, bind_group, &[]);
            let (columns, rows) = spread(groups);
            pass.dispatch_workgroups(columns, rows, 1);
        }
    }

    /// The commands the view draws its props with, after the bytes the counts take: the
    /// instances drawn, and those drawn coarser than their size asks.
    pub fn commands(&self) -> &Buffer {
        &self.commands
    }

    /// The instances the commands draw, each by its place among the arena's, the scale it is drawn
    /// at, how far it shows, and how far it fades in with whether the eye stands inside it.
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
        let words = |bytes: &[u8]| -> Vec<u32> {
            bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|word| u32::from_le_bytes(*word))
                .collect()
        };
        let regions = |bytes: &[u8]| -> Vec<Range<u32>> {
            words(bytes)[2..]
                .as_chunks::<5>()
                .0
                .iter()
                .map(|&[_, instances, _, _, first]| first..first + instances)
                .collect()
        };
        let listings = |bytes: &[u8]| -> Vec<ListedInstance> {
            words(bytes)
                .as_chunks::<4>()
                .0
                .iter()
                .map(|&[record, scale, shown, packed]| ListedInstance {
                    record,
                    scale: f32::from_bits(scale),
                    shown: f32::from_bits(shown),
                    around_eye: packed & 1 != 0,
                    faded_in: (packed >> 1) as f32 / 65535.0,
                })
                .collect()
        };
        let command_words = words(&commands);
        let (kept_commands, kept) = if self.occludes() {
            (
                regions(&read_buffer(device, queue, &self.kept_commands)),
                listings(&read_buffer(device, queue, &self.kept)),
            )
        } else {
            (Vec::new(), Vec::new())
        };
        CullReadback {
            drawn: command_words[0],
            missing: command_words[1],
            commands: regions(&commands),
            listed: listings(&read_buffer(device, queue, &self.drawn)),
            kept_commands,
            kept,
        }
    }
}

/// What a view's kernel drew: how many instances in all, how many of them coarser than their
/// size asks, each command's region of the drawn instances' list, and the list, as far as it
/// was filled.
#[derive(Clone, Debug, PartialEq)]
pub struct CullReadback {
    pub drawn: u32,
    pub missing: u32,
    pub commands: Vec<Range<u32>>,
    pub listed: Vec<ListedInstance>,
    /// What the lit passes draw of them once the prepass's depth has hidden its share: each
    /// command's region of the kept list, and the list; nothing for a view that occludes nothing.
    pub kept_commands: Vec<Range<u32>>,
    pub kept: Vec<ListedInstance>,
}

/// An instance a view draws: its place among the arena's, the scale it is drawn at, how far it
/// shows, whether the eye stands inside it, so that it clears away whole, and how far it has
/// faded into the representation the listing draws it as.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ListedInstance {
    pub record: u32,
    pub scale: f32,
    pub shown: f32,
    pub around_eye: bool,
    pub faded_in: f32,
}

/// The chunks of an arena a view may see: each by its first instance among the arena's, how
/// many it holds and whether it stands in the far field, and how many drawn instances they can
/// list at most.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SeenChunks {
    pub spans: Vec<UVec4>,
    /// How many instances the chunks hold together.
    pub instances: u32,
    pub listed: u32,
}

/// The chunks of an arena whose box reaches into `view`'s frustum, in the order of their keys.
pub fn seen_chunks(
    chunks: &HashMap<u32, GpuChunk>,
    table: &DrawTable,
    frustum: &Frustum,
    view: &CullView,
) -> SeenChunks {
    let mut listed = 0;
    let mut seen: Vec<(u32, UVec4)> = chunks
        .iter()
        .filter(|(_, chunk)| {
            let reach = chunk
                .contents
                .props
                .iter()
                .map(|(prop, _, _)| table.radius_of(*prop))
                .fold(0.0, f32::max)
                * chunk.contents.largest_scale;
            let low = chunk.contents.low - Vec3::splat(reach);
            let high = chunk.contents.high + Vec3::splat(reach);
            let bounds = Aabb::from_min_max(low, high);
            let casting = casting_frustum(frustum, view.casting_from(high.y - low.y));
            casting.intersects_obb(&bounds, &Affine3A::IDENTITY, view.intersect_near(), true)
        })
        .map(|(&key, chunk)| {
            for &(prop, _, instances) in &chunk.contents.props {
                listed += instances * table.listings_of(prop);
            }
            (
                key,
                UVec4::new(
                    (chunk.offset / InstanceRecord::BYTES) as u32,
                    chunk.instances,
                    u32::from(chunk.contents.place == ChunkPlace::Far),
                    0,
                ),
            )
        })
        .collect();
    seen.sort_by_key(|(key, _)| *key);
    let mut instances = 0;
    let spans = seen
        .into_iter()
        .map(|(_, mut span)| {
            span.w = instances;
            instances += span.y;
            span
        })
        .collect();
    SeenChunks {
        spans,
        instances,
        listed,
    }
}

/// What each scenery draws: its table, and what each view draws of it.
#[derive(Resource, Default)]
pub struct PropCulls(pub HashMap<Entity, SceneryCull>);

#[derive(Default)]
pub struct SceneryCull {
    pub table: Option<GpuDrawTable>,
    /// The arena's and the models' changes the table was made after.
    made_after: Option<(u64, u64)>,
    /// Which near chunks of the ground the arena holds, a bit each by the chunk's index.
    standing: Option<Buffer>,
    pub views: HashMap<RetainedViewEntity, ViewCull>,
}

/// The missing counts of the camera views on their way back from the GPU.
#[derive(Resource, Default)]
struct MissingReadbacks(Vec<MissingReadback>);

struct MissingReadback {
    view: RetainedViewEntity,
    buffer: Buffer,
    mapped: Option<Arc<Mutex<Option<bool>>>>,
}

/// The bytes a drawn instance takes: its place among the arena's, its scale, how far it shows,
/// and whether the eye stands inside it.
const DRAWN_BYTES: u64 = 16;

/// The threads of a workgroup of the count and list passes, as the kernel lays them out.
const THREADS: u32 = 64;

/// The most workgroups a dispatch may lay in a row, as the kernel lays them out.
const GROUPS_A_ROW: u32 = 65535;

/// `groups` workgroups laid out in rows the kernel reads: as many columns as fit a row, and the
/// rows they take.
fn spread(groups: u32) -> (u32, u32) {
    (groups.min(GROUPS_A_ROW), groups.div_ceil(GROUPS_A_ROW))
}
/// The bytes the counts take before the commands: the instances drawn, and those drawn coarser
/// than their size asks.
const COUNT_BYTES: u64 = 8;

/// How many pixels across an instance spans at the middle of the band where its model gives way
/// to its impostor: the atlas cell's side, below which the cell's picture loses nothing of the
/// model; and how wide the band is.
pub const IMPOSTOR_PIXELS: f32 = impostor::CELL_PIXELS as f32;
pub const BAND_PIXELS: f32 = 16.0;

/// The commands drawing the shapes the impostor stages pull their vertices by.
struct Shapes;

impl Shapes {
    const QUAD: DrawCommand = DrawCommand {
        first_index: QUAD_INDICES.start,
        indices: QUAD_INDICES.end - QUAD_INDICES.start,
        base_vertex: 0,
    };
    const BOX: DrawCommand = DrawCommand {
        first_index: BOX_INDICES.start,
        indices: BOX_INDICES.end - BOX_INDICES.start,
        base_vertex: 0,
    };
}

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
    atlas: Option<Res<ImpostorAtlas>>,
    clearing: Res<Clearing>,
    video: Res<VideoPreferences>,
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
            cull.standing = Some(standing_buffer(&device, arena));
            cull.made_after = Some(changes);
            cull.views.clear();
        }
        if let (Some(table), Some(atlas)) = (&mut cull.table, atlas.as_deref()) {
            table.place_impostors(&device, &queue, atlas.round, |key| atlas.layers.slot(key));
        }
        let (Some(table), Some(records), Some(shares), Some(standing)) = (
            &cull.table,
            arena.slab.buffer(),
            arena.shown.buffer(),
            &cull.standing,
        ) else {
            continue;
        };
        let mut shown = HashSet::new();
        for (view, frustum, layers, light) in &views {
            if !layers.unwrap_or_default().intersects(&drawn.layers) {
                continue;
            }
            let lit = light.then(|| lit_frustum(&clip_from_world_of(view), frustum));
            let frustum = lit.as_ref().unwrap_or(frustum);
            let mut culled = CullView::of(view, frustum, light);
            if !light {
                culled = culled
                    .within(video.view_distance.horizon())
                    .handing_over_at(IMPOSTOR_PIXELS * video.detail.impostor_widths());
            }
            if let Some(clearing) = clearing.0
                && !light
                && view.retained_view_entity.main_entity.id() == clearing.camera
            {
                culled = culled.clearing(clearing.target, clearing.step);
            }
            let seen = seen_chunks(&arena.chunks, &table.table, frustum, &culled);
            cull.views
                .entry(view.retained_view_entity)
                .or_insert_with(|| ViewCull::new(&device, table))
                .update(
                    &device, &queue, &kernel, table, records, shares, standing, seen, culled,
                );
            shown.insert(view.retained_view_entity);
        }
        cull.views.retain(|view, _| shown.contains(view));
    }
}

/// Culls every scenery the view shows, and what each of the view's lights casts shadows of,
/// before anything of the view or its shadows is drawn: the props the view itself draws counted
/// among the render diagnostics.
/// Every camera's depth can be read once its prepass is drawn, as the occlusion reads it.
fn let_depth_be_read(mut cameras: Query<&mut Camera3d, Changed<Camera3d>>) {
    for mut camera in &mut cameras {
        let usages = TextureUsages::from(camera.depth_texture_usages);
        if !usages.contains(TextureUsages::TEXTURE_BINDING) {
            camera.depth_texture_usages = (usages | TextureUsages::TEXTURE_BINDING).into();
        }
    }
}

/// The pyramid of a view's prepass depth, and the bind group that builds it from the depth.
#[derive(Component)]
struct PropPyramid {
    pyramid: ViewDepthPyramid,
    downsample: BindGroup,
}

/// Gives every camera view that culls props a pyramid of its prepass depth, and sets each of its
/// culls to keep only what that depth doesn't hide for the lit passes.
#[allow(clippy::too_many_arguments)]
pub(super) fn prepare_occlusion(
    mut commands: Commands,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut textures: ResMut<TextureCache>,
    dummy: Option<Res<DepthPyramidDummyTexture>>,
    pipelines: Option<Res<DownsampleDepthPipelines>>,
    cache: Res<PipelineCache>,
    kernel: Option<Res<CullKernel>>,
    mut culls: ResMut<PropCulls>,
    views: Query<(Entity, &ExtractedView, &ViewDepthStencilTexture), Without<LightEntity>>,
) {
    let (Some(dummy), Some(pipelines), Some(kernel)) = (dummy, pipelines, kernel) else {
        return;
    };
    let ready = [&pipelines.first, &pipelines.second]
        .iter()
        .all(|pipeline| {
            pipeline
                .pipeline_id
                .is_some_and(|id| cache.get_compute_pipeline(id).is_some())
        });
    for (entity, view, depth) in &views {
        let culled = ready
            && culls
                .0
                .values()
                .any(|cull| cull.views.contains_key(&view.retained_view_entity));
        let Some(depth_view) = depth
            .attachment
            .depth_stencil_views()
            .depth_only_view()
            .filter(|_| culled && depth.texture().sample_count() == 1)
        else {
            commands.entity(entity).remove::<PropPyramid>();
            continue;
        };
        let pyramid = ViewDepthPyramid::new(
            &device,
            &mut textures,
            &dummy,
            view.viewport.zw(),
            "prop depth pyramid",
            "prop depth pyramid level",
        );
        for cull in culls.0.values_mut() {
            if let Some(culled) = cull.views.get_mut(&view.retained_view_entity) {
                culled.occlude_with(
                    &device,
                    &queue,
                    &kernel,
                    &pyramid.all_mips,
                    view.world_from_view.to_matrix(),
                    view.clip_from_view,
                );
            }
        }
        let downsample = pyramid.create_bind_group(
            &device,
            "prop depth pyramid",
            &cache.get_bind_group_layout(&pipelines.first.bind_group_layout),
            depth_view,
            &pipelines.sampler,
        );
        commands.entity(entity).insert(PropPyramid {
            pyramid,
            downsample,
        });
    }
}

/// Builds the view's pyramid of the depth the prepass drew, and keeps for the lit passes what
/// it doesn't hide of each cull's listings.
fn occlude_props(
    view: ViewQuery<(&ExtractedView, &PropPyramid)>,
    pipelines: Option<Res<DownsampleDepthPipelines>>,
    cache: Res<PipelineCache>,
    kernel: Option<Res<CullKernel>>,
    culls: Res<PropCulls>,
    mut ctx: RenderContext,
) {
    let (seen, prop_pyramid) = view.into_inner();
    let (Some(pipelines), Some(kernel)) = (pipelines, kernel) else {
        return;
    };
    let (Some(first), Some(second)) = (
        pipelines
            .first
            .pipeline_id
            .and_then(|id| cache.get_compute_pipeline(id)),
        pipelines
            .second
            .pipeline_id
            .and_then(|id| cache.get_compute_pipeline(id)),
    ) else {
        return;
    };
    let recorder = ctx.diagnostic_recorder();
    let recorder = recorder.as_deref();
    let span = recorder.map(|recorder| recorder.time_span(ctx.command_encoder(), PROP_OCCLUDE));
    prop_pyramid.pyramid.downsample_depth_with_ctx(
        "prop depth pyramid",
        &mut ctx,
        seen.viewport.zw(),
        &prop_pyramid.downsample,
        first,
        second,
    );
    let encoder = ctx.command_encoder();
    for cull in culls.0.values() {
        let Some(table) = &cull.table else {
            continue;
        };
        if let Some(culled) = cull.views.get(&seen.retained_view_entity) {
            culled.encode_occlusion(&kernel, table, encoder);
        }
    }
    if let Some(span) = span {
        span.end(encoder);
    }
}

fn cull_props(
    view: ViewQuery<(&ExtractedView, Option<&ViewLightEntities>)>,
    lights: Query<&ExtractedView, With<LightEntity>>,
    kernel: Option<Res<CullKernel>>,
    culls: Res<PropCulls>,
    mut readbacks: ResMut<MissingReadbacks>,
    mut ctx: RenderContext,
) {
    let Some(kernel) = kernel else {
        return;
    };
    let (seen, view_lights) = view.into_inner();
    let device = ctx.render_device().clone();
    let shadows: Vec<RetainedViewEntity> = view_lights
        .into_iter()
        .flat_map(|view_lights| view_lights.lights.iter())
        .filter_map(|&light| lights.get(light).ok())
        .map(|light| light.retained_view_entity)
        .collect();
    let recorder = ctx.diagnostic_recorder();
    let recorder = recorder.as_deref();
    let encoder = ctx.command_encoder();
    let span = recorder.map(|recorder| recorder.time_span(encoder, PROP_CULL));
    for cull in culls.0.values() {
        let Some(table) = &cull.table else {
            continue;
        };
        if let Some(culled) = cull.views.get(&seen.retained_view_entity) {
            culled.encode(&kernel, table, encoder);
            if let Some(recorder) = recorder {
                recorder.record_u32(encoder, &culled.commands.slice(0..4), PROPS_DRAWN);
            }
            let buffer = device.create_buffer(&BufferDescriptor {
                label: Some("props missing"),
                size: 4,
                usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            encoder.copy_buffer_to_buffer(&culled.commands, 4, &buffer, 0, 4);
            readbacks.0.push(MissingReadback {
                view: seen.retained_view_entity,
                buffer,
                mapped: None,
            });
        }
        for light in &shadows {
            if let Some(culled) = cull.views.get(light) {
                culled.encode(&kernel, table, encoder);
            }
        }
    }
    if let Some(span) = span {
        span.end(encoder);
    }
}

/// Asks the GPU for each camera view's missing count once the frame that wrote it is sent, and
/// notes the latest counts, summed over the views, in the render progress once they come back.
fn read_missing(mut readbacks: ResMut<MissingReadbacks>, progress: Res<RenderProgress>) {
    let mut latest: HashMap<RetainedViewEntity, u32> = HashMap::new();
    readbacks.0.retain_mut(|readback| {
        let Some(mapped) = &readback.mapped else {
            let mapped = Arc::new(Mutex::new(None));
            let answer = mapped.clone();
            readback
                .buffer
                .slice(..)
                .map_async(MapMode::Read, move |result| {
                    *answer.lock().expect("the missing count's answer lock") = Some(result.is_ok());
                });
            readback.mapped = Some(mapped);
            return true;
        };
        let Some(ok) = *mapped.lock().expect("the missing count's answer lock") else {
            return true;
        };
        let count = ok
            .then(|| {
                let bytes = readback.buffer.slice(..).get_mapped_range().ok()?.to_vec();
                Some(u32::from_le_bytes(bytes.try_into().ok()?))
            })
            .flatten()
            .unwrap_or(0);
        latest.insert(readback.view, count);
        false
    });
    if !latest.is_empty() {
        progress.missing_drawn(latest.into_values().sum());
    }
}

/// Which near chunks of the ground `arena` holds, a bit each by the chunk's index.
fn standing_buffer(device: &RenderDevice, arena: &GpuInstances) -> Buffer {
    let mut words = vec![0u32; 1];
    for chunk in arena.chunks.values() {
        if let ChunkPlace::Near { index } = chunk.contents.place {
            let word = (index / 32) as usize;
            if words.len() <= word {
                words.resize(word + 1, 0);
            }
            words[word] |= 1 << (index % 32);
        }
    }
    let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("near chunks standing"),
        contents: &bytes,
        usage: BufferUsages::STORAGE,
    })
}

/// The table drawing what `arena` holds of the props whose models are on the GPU: their parts
/// where they are dressed, their impostors and boxes regardless.
fn draw_table(arena: &GpuInstances, models: &ModelArena) -> DrawTable {
    DrawTable::of(arena.chunks.values().flat_map(|chunk| {
        chunk
            .contents
            .props
            .iter()
            .filter_map(|&(prop, model, instances)| {
                let dressed = arena.dressed.get(&prop);
                Some((
                    prop,
                    DrawnProp {
                        model,
                        placed: models.slabs.get(&model)?,
                        instances,
                        dressed: dressed.is_some(),
                        fades: dressed.is_some_and(|dressed| dressed.fades),
                        overhang: dressed.map_or(0.0, |dressed| dressed.overhang.0),
                        translucency: dressed.map_or(0.0, |dressed| dressed.translucency),
                        impostor: dressed.and_then(|dressed| dressed.impostor),
                    },
                ))
            })
    }))
}

fn commands_bytes(commands: usize) -> u64 {
    COUNT_BYTES + commands as u64 * DrawCommand::BYTES
}

/// The bytes of `commands` as each frame starts them: none drawn, none missing, and no instance
/// in any.
fn starts_bytes(commands: &[DrawCommand]) -> Vec<u8> {
    let mut bytes = vec![0; COUNT_BYTES as usize];
    for command in commands {
        bytes.extend_from_slice(command.start().as_bytes());
    }
    bytes
}

/// Room for `listed` drawn instances, at least one.
fn commands_buffer(device: &RenderDevice, table: &GpuDrawTable) -> Buffer {
    device.create_buffer(&BufferDescriptor {
        label: Some("prop draw commands"),
        size: commands_bytes(table.table.commands.len()),
        usage: BufferUsages::STORAGE
            | BufferUsages::INDIRECT
            | BufferUsages::COPY_DST
            | BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

fn drawn_buffer(device: &RenderDevice, listed: u32) -> Buffer {
    device.create_buffer(&BufferDescriptor {
        label: Some("props drawn"),
        size: u64::from(listed.max(1)) * DRAWN_BYTES,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    })
}

pub(super) fn read_buffer(device: &RenderDevice, queue: &RenderQueue, buffer: &Buffer) -> Vec<u8> {
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
    slice.map_async(MapMode::Read, move |mapped| {
        let _ = sender.send(mapped);
    });
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
