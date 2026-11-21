//! Fog that lies in banks standing on the ground, drawn over everything the camera sees once
//! the world is drawn, the sky included: thicker air, thickest at each bank's floor and thinning
//! upward, each pixel fogged by how much of it the way from the eye to what it shows crosses,
//! and lit by the sky it lies under.

use super::sky_picture;
use crate::core::units::Meters;
use bevy::core_pipeline::core_3d::main_transparent_pass_3d;
use bevy::core_pipeline::{Core3d, Core3dSystems, FullscreenShader};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::diagnostic::RecordDiagnostics;
use bevy::render::extract_component::{
    ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin,
    UniformComponentPlugin,
};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{
    sampler, texture_2d, texture_depth_2d, uniform_buffer,
};
use bevy::render::render_resource::{
    BindGroupEntries, BindGroupLayoutDescriptor, BindGroupLayoutEntries, BlendComponent,
    BlendState, CachedRenderPipelineId, ColorTargetState, ColorWrites, FragmentState,
    PipelineCache, RenderPassDescriptor, RenderPipelineDescriptor, SamplerBindingType,
    ShaderStages, ShaderType, TextureFormat, TextureSampleType,
};
use bevy::render::renderer::{RenderContext, ViewQuery};
use bevy::render::texture::{FallbackImage, GpuImage};
use bevy::render::view::{
    ExtractedView, ViewDepthStencilTexture, ViewTarget, ViewUniform, ViewUniformOffset,
    ViewUniforms,
};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};

/// The most boxes of haze a camera looks through at once: the nearest of them.
pub const MAX_HAZE_BOXES: usize = 32;

/// The fog a camera sees the world through: lying in `boxes` (the first `count` of them), lit by
/// `lights`, and by the sky's picture the camera holds as its [`HazeSky`] where it stands
/// `under_sky`, 1 or 0.
#[derive(Component, ExtractComponent, ShaderType, Clone, Copy, Debug, Default, PartialEq)]
#[extract_app(RenderApp)]
pub struct Haze {
    pub lights: [HazeLight; 2],
    pub boxes: [HazeBox; MAX_HAZE_BOXES],
    pub count: u32,
    pub under_sky: u32,
}

/// A light the fog throws onward: the unit way toward it, and what it sheds on the fog round
/// the eye, in lux for each colour.
#[derive(ShaderType, Clone, Copy, Debug, Default, PartialEq)]
pub struct HazeLight {
    pub toward: Vec3,
    pub shed: Vec3,
}

/// The picture of the sky the fog a camera sees is lit by.
#[derive(Component, ExtractComponent, Clone, Debug, PartialEq)]
#[extract_app(RenderApp)]
pub struct HazeSky(pub Handle<Image>);

/// A bank of fog over the rect from `low` to `high` (x east, y south): `thickest` at `floor`,
/// for every metre looked through it there, falling to a third of that (by e) every `fall`
/// metres up, thinning out over `edge` metres at its sides, and shedding `glow` of its own, in
/// candela per square metre.
#[derive(ShaderType, Clone, Copy, Debug, Default, PartialEq)]
pub struct HazeBox {
    pub low: Vec2,
    pub high: Vec2,
    pub floor: f32,
    pub fall: f32,
    pub thickest: f32,
    pub edge: f32,
    pub glow: Vec3,
}

/// How many lengths of fog leave a twentieth of a thing's contrast: what counts as the farthest
/// it is seen, as weather services measure visibility.
pub const SIGHT_LENGTHS: f32 = 3.0;

impl Haze {
    /// Fog lying in `boxes`, the first [`MAX_HAZE_BOXES`] of them taken, lit by `lights` and,
    /// when `under_sky`, by the sky's picture.
    pub fn new(
        lights: [HazeLight; 2],
        under_sky: bool,
        boxes: impl IntoIterator<Item = HazeBox>,
    ) -> Haze {
        let mut haze = Haze {
            lights,
            under_sky: u32::from(under_sky),
            ..Haze::default()
        };
        for (slot, haze_box) in haze.boxes.iter_mut().zip(boxes) {
            *slot = haze_box;
            haze.count += 1;
        }
        haze
    }

    /// How far a thing still shows through the fog standing at `at`, looking level: nothing
    /// where no bank lies over `at`.
    pub fn sight_at(&self, at: Vec3) -> Option<Meters> {
        let thick: f32 = self.boxes[..self.count as usize]
            .iter()
            .map(|bank| bank.thick_at(at))
            .sum();
        (thick > 0.0).then(|| Meters(SIGHT_LENGTHS / thick))
    }
}

impl HazeBox {
    /// How much of the bank's fog a line of sight from `eye` to `point` crosses: lengths of it,
    /// each of which leaves 1/e of what is seen through it.
    pub fn crossed(&self, eye: Vec3, point: Vec3) -> f32 {
        let way = point - eye;
        let across = span(eye.x, way.x, self.low.x, self.high.x);
        let along = span(eye.z, way.z, self.low.y, self.high.y);
        let enter = across.0.max(along.0).max(0.0);
        let leave = across.1.min(along.1).min(1.0);
        if leave <= enter {
            return 0.0;
        }
        let run = way.length() * (leave - enter);
        let from = (eye.y + way.y * enter - self.floor).max(0.0) / self.fall;
        let to = (eye.y + way.y * leave - self.floor).max(0.0) / self.fall;
        let rise = to - from;
        let thick = if rise.abs() > 1.0e-3 {
            ((-from).exp() - (-to).exp()) / rise
        } else {
            (-(from + to) / 2.0).exp()
        };
        let sides: f32 = WEIGHED_AT
            .iter()
            .zip(WEIGHS)
            .map(|(at, weighs)| {
                let at = eye.xz() + way.xz() * (enter + (leave - enter) * at);
                weighs * self.sides_at(at)
            })
            .sum();
        self.thickest * thick * run * sides
    }

    /// How thick the bank's fog stands at `at`: lengths of it to the metre.
    pub fn thick_at(&self, at: Vec3) -> f32 {
        let up = (at.y - self.floor).max(0.0) / self.fall;
        self.thickest * (-up).exp() * self.sides_at(at.xz())
    }

    fn sides_at(&self, at: Vec2) -> f32 {
        let inside = (at - self.low).min(self.high - at) / self.edge.max(1.0e-3);
        inside.x.clamp(0.0, 1.0) * inside.y.clamp(0.0, 1.0)
    }
}

/// The shares of the way from `start`, going `step` along, at which it enters and leaves the
/// stretch from `low` to `high`.
fn span(start: f32, step: f32, low: f32, high: f32) -> (f32, f32) {
    if step.abs() < 1e-6 {
        return if start >= low && start <= high {
            (0.0, 1.0)
        } else {
            (1.0, 0.0)
        };
    }
    let (a, b) = ((low - start) / step, (high - start) / step);
    (a.min(b), a.max(b))
}

/// Where along a stretch a bank's sides are weighed, and how much each place counts: Gauss's
/// four points.
const WEIGHED_AT: [f32; 4] = [0.069_431_8, 0.330_009_5, 0.669_990_5, 0.930_568_2];
const WEIGHS: [f32; 4] = [0.173_927_4, 0.326_072_6, 0.326_072_6, 0.173_927_4];

/// Where a camera's rays run over the ground before they meet it: from under its eye, between
/// the two ways its outermost rays run, or every way when it looks straight down or up, and as
/// far as a ray of its runs before it has sunk to the lowest the ground lies, or any distance
/// when one runs level or upward. A box of haze none of them crosses hazes nothing the camera
/// sees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HazeSight {
    eye: Vec2,
    between: Option<[Vec2; 2]>,
    reach: f32,
}

impl HazeSight {
    /// The sight of a camera of lens `clip_from_view` standing as `eye` does over ground that
    /// lies nowhere lower than `floor`.
    pub fn of(clip_from_view: Mat4, eye: &GlobalTransform, floor: f32) -> HazeSight {
        let world_from_view = eye.affine();
        let sees = |way: Vec3| {
            let seen = world_from_view.inverse().transform_vector3(way);
            let clip = clip_from_view * seen.extend(1.0);
            clip.w > 0.0 && clip.x.abs() <= clip.w && clip.y.abs() <= clip.w
        };
        let view_from_clip = clip_from_view.inverse();
        let rays = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].map(|(x, y)| {
            let widened = Vec3::new(x, y, 0.0) * (1.0 + SIGHT_MARGIN);
            world_from_view
                .transform_vector3(view_from_clip.project_point3(widened.with_z(0.5)))
                .normalize_or_zero()
        });
        let ways = rays.map(|ray| ray.xz().normalize_or_zero());
        let outermost = |side: f32| {
            ways.into_iter().find(|way| {
                ways.iter()
                    .all(|other| way.perp_dot(*other) * side >= -1e-6)
            })
        };
        let between = match (sees(Vec3::Y), sees(Vec3::NEG_Y), ways.contains(&Vec2::ZERO)) {
            (false, false, false) => outermost(1.0).zip(outermost(-1.0)).map(|(a, b)| [a, b]),
            _ => None,
        };
        let above = (eye.translation().y - floor).max(0.0);
        let reach = rays
            .into_iter()
            .map(|ray| {
                if ray.y < -1e-3 {
                    above * ray.xz().length() / -ray.y
                } else {
                    f32::INFINITY
                }
            })
            .fold(0.0, f32::max);
        HazeSight {
            eye: eye.translation().xz(),
            between,
            reach,
        }
    }

    /// Whether any ray of the camera's runs over the rect from `low` to `high`.
    pub fn crosses(&self, low: Vec2, high: Vec2) -> bool {
        if self.eye.distance(self.eye.clamp(low, high)) > self.reach {
            return false;
        }
        let Some([first, last]) = self.between else {
            return true;
        };
        let corners = [
            low,
            Vec2::new(high.x, low.y),
            high,
            Vec2::new(low.x, high.y),
        ]
        .map(|corner| corner - self.eye);
        corners.iter().any(|corner| first.perp_dot(*corner) >= 0.0)
            && corners.iter().any(|corner| last.perp_dot(*corner) <= 0.0)
    }
}

/// How much wider than the lens a camera's sight is taken to be, as a share of the lens, for
/// the jitter its picture is drawn with from frame to frame.
const SIGHT_MARGIN: f32 = 0.01;

pub struct HazePlugin;

/// Where the haze is laid over what a camera sees, for what is laid over the haze in turn.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Hazing;

impl Plugin for HazePlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "haze.wesl");
        app.add_plugins((
            ExtractComponentPlugin::<Haze>::default(),
            ExtractComponentPlugin::<HazeSky>::default(),
            UniformComponentPlugin::<Haze>::default(),
        ));
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .add_systems(RenderStartup, start_pipeline)
            .add_systems(Render, queue_pipelines.in_set(RenderSystems::Queue))
            .add_systems(
                Core3d,
                draw_haze
                    .after(main_transparent_pass_3d)
                    .in_set(Core3dSystems::MainPass)
                    .in_set(Hazing),
            );
    }
}

#[derive(Resource)]
struct HazePipeline {
    layout: BindGroupLayoutDescriptor,
    shader: Handle<Shader>,
}

#[derive(Component, Clone, Copy)]
struct HazePipelineId(CachedRenderPipelineId);

type HazedCamera = (With<Haze>, With<ExtractedCamera>);

type HazedView = (
    &'static ExtractedCamera,
    &'static ViewTarget,
    &'static ViewUniformOffset,
    &'static Haze,
    &'static DynamicUniformIndex<Haze>,
    &'static HazePipelineId,
    &'static ViewDepthStencilTexture,
    Option<&'static HazeSky>,
);

fn start_pipeline(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(HazePipeline {
        layout: BindGroupLayoutDescriptor::new(
            "haze_bind_group_layout",
            &BindGroupLayoutEntries::sequential(
                ShaderStages::FRAGMENT,
                (
                    uniform_buffer::<ViewUniform>(true),
                    uniform_buffer::<Haze>(true),
                    texture_depth_2d(),
                    texture_2d(TextureSampleType::Float { filterable: true }),
                    sampler(SamplerBindingType::Filtering),
                ),
            ),
        ),
        shader: bevy::asset::load_embedded_asset!(assets.as_ref(), "haze.wesl"),
    });
}

fn queue_pipelines(
    mut commands: Commands,
    cache: Res<PipelineCache>,
    pipeline: Res<HazePipeline>,
    fullscreen: Res<FullscreenShader>,
    views: Query<(Entity, &ExtractedView), HazedCamera>,
    mut queued: Local<HashMap<TextureFormat, CachedRenderPipelineId>>,
) {
    for (entity, view) in &views {
        let id = *queued.entry(view.target_format).or_insert_with(|| {
            cache.queue_render_pipeline(RenderPipelineDescriptor {
                label: Some("haze_pipeline".into()),
                layout: vec![pipeline.layout.clone()],
                vertex: fullscreen.to_vertex_state(),
                fragment: Some(FragmentState {
                    shader: pipeline.shader.clone(),
                    targets: vec![Some(ColorTargetState {
                        format: view.target_format,
                        blend: Some(BlendState {
                            color: BlendComponent::OVER,
                            alpha: BlendComponent::OVER,
                        }),
                        write_mask: ColorWrites::ALL,
                    })],
                    ..default()
                }),
                ..default()
            })
        });
        commands.entity(entity).insert(HazePipelineId(id));
    }
}

fn draw_haze(
    view: ViewQuery<HazedView>,
    pipeline: Option<Res<HazePipeline>>,
    cache: Res<PipelineCache>,
    (view_uniforms, hazes): (Res<ViewUniforms>, Res<ComponentUniforms<Haze>>),
    (images, fallback): (Res<RenderAssets<GpuImage>>, Res<FallbackImage>),
    mut ctx: RenderContext,
) {
    let (camera, target, view_offset, haze, haze_index, pipeline_id, depth, sky) =
        view.into_inner();
    if haze.count == 0 {
        return;
    }
    let (Some(pipeline), Some(view_binding), Some(haze_binding)) = (
        pipeline,
        view_uniforms.uniforms.binding(),
        hazes.uniforms().binding(),
    ) else {
        return;
    };
    let Some(depth_view) = depth.attachment.depth_stencil_views().depth_only_view() else {
        return;
    };
    let Some(render_pipeline) = cache.get_render_pipeline(pipeline_id.0) else {
        return;
    };
    let sky = sky.and_then(|sky| images.get(&sky.0));
    let (sky_view, sky_sampler) = match sky {
        Some(picture) => (sky_picture::every_size(picture), &picture.sampler),
        None => (fallback.d2.texture_view.clone(), &fallback.d2.sampler),
    };
    let bind_group = ctx.render_device().create_bind_group(
        "haze_bind_group",
        &cache.get_bind_group_layout(&pipeline.layout),
        &BindGroupEntries::sequential((
            view_binding,
            haze_binding,
            depth_view,
            &sky_view,
            sky_sampler,
        )),
    );
    let recorder = ctx.diagnostic_recorder();
    let recorder = recorder.as_deref();
    let span = recorder.map(|recorder| recorder.time_span(ctx.command_encoder(), HAZE));
    let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some(HAZE),
        color_attachments: &[Some(target.get_color_attachment())],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    if let Some(viewport) = camera.viewport.as_ref() {
        pass.set_camera_viewport(viewport);
    }
    pass.set_render_pipeline(render_pipeline);
    pass.set_bind_group(0, &bind_group, &[view_offset.offset, haze_index.index()]);
    pass.draw(0..3, 0..1);
    drop(pass);
    if let Some(span) = span {
        span.end(ctx.command_encoder());
    }
}

const HAZE: &str = "haze";
